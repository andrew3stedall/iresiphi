//! A small invoice-shaped recipe over real JSON and CSV files, shared by the
//! budget (`limits.rs`) and delivery (`delivery.rs`) suites.
#![allow(dead_code)]
use iresiphi::{Cancellation, RunPhase, model::*};
use serde_json::{Value as Json, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

pub struct Fixture {
    pub dir: tempfile::TempDir,
    pub recipe: Recipe,
    pub bindings: Bindings,
    pub policy: Policy,
}
impl Fixture {
    pub fn root(&self) -> &Path {
        self.dir.path()
    }
    pub fn destination(&self) -> PathBuf {
        self.root().join("bundle")
    }
}

fn field(source: &str, name: &str, path: &[&str]) -> Json {
    json!({"op":"field","source":source,"field":name,"path":path})
}
fn literal(value: Json) -> Json {
    json!({"op":"literal","value":value})
}

pub const LINES: &str = r#"[
 {"id":"1","code":"A","date":"2026-02-01","qty":"2","note":"\\x"},
 {"id":"2","code":"B","date":"2026-03-01","qty":"1","note":"plain"},
 {"id":"3","code":"Z","date":"2026-03-01","qty":"4","note":"\\\\N"}
]"#;
pub const RATES: &str =
    "id,code,rate,start,end\nr1,A,1.50,2026-01-01,\\N\nr2,B,2.25,2026-01-01,2027-01-01\n";
pub const COMPONENTS: &str = "id,line,amount\nc1,1,1.00\nc2,1,2.00\nc3,2,3.00\n";

/// Three invoice lines (one unmatched), a dated rate lookup and component detail.
/// Output columns cover every scalar type plus empty, null and backslash-led text.
pub fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("lines.json"), LINES).unwrap();
    std::fs::write(root.join("rates.csv"), RATES).unwrap();
    std::fs::write(root.join("components.csv"), COMPONENTS).unwrap();
    let string = |p: &str| json!({"path":p,"data_type":{"kind":"string"}});
    let date = |p: &str| json!({"path":p,"data_type":{"kind":"date"}});
    let recipe = Recipe::from_json(&json!({
        "format_version":"0.1","name":"bounded delivery",
        "sources":[
            {"id":"lines","identity":["id"],"grain":["id"],"projection":{"array":"","fields":{
                "id":string("/id"),"code":string("/code"),"date":date("/date"),"note":string("/note"),
                "qty":{"path":"/qty","data_type":{"kind":"decimal","precision":12,"scale":2}}}}},
            {"id":"rates","identity":["id"],"grain":["id"],"projection":{"fields":{
                "id":string("id"),"code":string("code"),"start":date("start"),
                "end":{"path":"end","data_type":{"kind":"date"},"nullable":true},
                "rate":{"path":"rate","data_type":{"kind":"decimal","precision":12,"scale":2}}}}},
            {"id":"components","identity":["id"],"grain":["id"],"projection":{"fields":{
                "id":string("id"),"line":string("line"),
                "amount":{"path":"amount","data_type":{"kind":"decimal","precision":12,"scale":2}}}}}
        ],
        "relationships":[
            {"id":"rate","from":"lines","to":"rates","from_keys":["code"],"to_keys":["code"],
             "cardinality":"many_to_one","validity":{"date":"date","valid_from":"start","valid_to":"end"}},
            {"id":"parts","from":"lines","to":"components","from_keys":["id"],"to_keys":["line"],
             "cardinality":"one_to_many"}
        ],
        "outputs":[{"id":"review","anchor":"lines","columns":[
            {"name":"id","expr":field("lines","id",&[])},
            {"name":"date","expr":field("lines","date",&[])},
            {"name":"note","expr":field("lines","note",&[])},
            {"name":"rate","expr":field("rates","rate",&["rate"])},
            {"name":"expected","expr":{"op":"quantize","expr":{"op":"multiply",
                "left":field("lines","qty",&[]),"right":field("rates","rate",&["rate"])},
                "precision":18,"scale":2,"rounding":"half_even"}},
            {"name":"components","expr":{"op":"aggregate","relationship":"parts","function":"sum","field":"amount"}},
            {"name":"empty","expr":literal(json!({"kind":"string","value":""}))},
            {"name":"slash_n","expr":literal(json!({"kind":"string","value":"\\N"}))},
            {"name":"quoted","expr":literal(json!({"kind":"string","value":"a,\"b\"\nc"}))},
            {"name":"flag","expr":literal(json!({"kind":"boolean","value":true}))},
            {"name":"count","expr":literal(json!({"kind":"int64","value":"-7"}))}
        ]}]
    })
    .to_string())
    .unwrap();
    let bindings = [
        ("lines", "lines.json", "json"),
        ("rates", "rates.csv", "csv"),
        ("components", "components.csv", "csv"),
    ]
    .into_iter()
    .map(|(id, file, adapter)| {
        (
            id.to_owned(),
            Binding {
                path: root.join(file),
                adapter: adapter.into(),
                settings: BTreeMap::new(),
            },
        )
    })
    .collect();
    let policy = Policy {
        allowed_read_roots: vec![root.to_owned()],
        allowed_output_roots: vec![root.to_owned()],
        limits: Limits::default(),
        diagnostic_values: false,
    };
    Fixture {
        dir,
        recipe,
        bindings,
        policy,
    }
}

/// A caller-defined cancellation source that observes run phases.
pub struct Hook<F: Fn(RunPhase, &AtomicBool)> {
    pub flag: AtomicBool,
    pub on: F,
}
pub fn hook<F: Fn(RunPhase, &AtomicBool)>(on: F) -> Hook<F> {
    Hook {
        flag: AtomicBool::new(false),
        on,
    }
}
impl<F: Fn(RunPhase, &AtomicBool)> Cancellation for Hook<F> {
    fn flag(&self) -> &AtomicBool {
        &self.flag
    }
    fn entered(&self, phase: RunPhase) {
        (self.on)(phase, &self.flag)
    }
}

/// Names of entries in `dir` that look like run-owned staging.
pub fn staging_entries(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".iresiphi-"))
        .collect()
}

/// Live engine worker threads owned by runs in this process.
pub fn engine_threads() -> usize {
    std::fs::read_dir("/proc/self/task")
        .map(|tasks| {
            tasks
                .filter_map(|task| std::fs::read_to_string(task.ok()?.path().join("comm")).ok())
                .filter(|comm| comm.trim_end() == "iresiphi-exec")
                .count()
        })
        .unwrap_or(0)
}

/// Exact serialized size of a memory result, measured the way the run budgets it.
pub fn result_bytes(report: &iresiphi::RunReport) -> usize {
    serde_json::to_vec(&(
        &report.tables,
        &report.evidence,
        &report.candidate_evidence,
        &report.detail_evidence,
    ))
    .unwrap()
    .len()
}

pub fn assert_failed_without_results(report: &iresiphi::RunReport, status: &str, code: &str) {
    assert_eq!(report.status, status, "{:?}", report.diagnostics);
    assert_eq!(report.diagnostics[0].code, code, "{:?}", report.diagnostics);
    assert!(report.tables.is_empty(), "no partial tables");
    assert!(report.evidence.is_empty(), "no partial evidence");
    assert_eq!(report.manifest["complete"], false);
}
