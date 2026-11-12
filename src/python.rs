//! Python bindings (decision #12).
//!
//! Opaque handles own Rust state: a [`Recipe`], a [`Catalog`] or a finished
//! [`RunReport`]. Python inputs (recipe declarations, bindings, policy) are
//! converted once into Rust-owned JSON on the calling thread and parsed by the
//! same Rust model as the Rust API; no worker ever sees a Python object, buffer
//! or callback. Results are materialized as *copies*, one bounded batch per
//! call: `int` for Int64, `decimal.Decimal` built from the exact decimal text,
//! `datetime.date`, `str`, `bool` and `None`. Nothing passes through a float.
//!
//! A run executes on a dedicated Rust thread while the calling thread detaches
//! from the interpreter. The caller re-attaches every [`SIGNAL_CHECK`] to run
//! Python signal handlers; a `KeyboardInterrupt` requests cancellation through
//! the run's shared [`CancellationToken`], waits for the worker to finish and
//! raises `iresiphi.errors.RunInterrupted` (a `KeyboardInterrupt` subclass)
//! carrying the run report.
use crate::{
    Bindings, CancellationToken, Error, ExecutionContext, Policy, Recipe, RunPhase, RunReport,
    model::{Catalog, Value},
};
use pyo3::{
    exceptions::{PyKeyboardInterrupt, PyRuntimeError},
    prelude::*,
    sync::PyOnceLock,
    types::{PyBool, PyDate, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple},
};
use serde_json::Value as Json;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

/// Longest interval between signal checks while a Python caller waits on a run.
/// Decision #13 requires at most 100 ms.
const SIGNAL_CHECK: Duration = Duration::from_millis(50);
/// Hard cap on rows copied into Python per batch (decision #13).
const MAX_BATCH_ROWS: usize = 8192;

// ---------------------------------------------------------------------------
// Errors: every failure is an `iresiphi.errors` exception carrying a full record.

fn record(code: &str, phase: &str, message: impl Into<String>) -> Error {
    Error::new(code, phase, message).completed()
}

/// Build the Python exception for a diagnostic record. `class` names a class in
/// `iresiphi.errors`; `report` is the owning run handle, when there is one.
fn raise(py: Python<'_>, class: &str, error: &Error, report: Option<Py<PyRun>>) -> PyErr {
    let build = || -> PyResult<PyErr> {
        let classes = py.import("iresiphi.errors")?;
        let mut value = serde_json::to_value(error)
            .map_err(|e| PyRuntimeError::new_err(format!("INTERNAL_ERROR: {e}")))?;
        let report_id = report.as_ref().and_then(|r| r.get().run_id_unchecked());
        value["report_id"] = report_id.clone().map(Json::String).unwrap_or(Json::Null);
        let record = json_to_py(py, &value)?;
        let instance = classes
            .getattr(class)?
            .call1((record, report, report_id))?;
        Ok(PyErr::from_value(instance))
    };
    build().unwrap_or_else(|failure| {
        let fallback = PyRuntimeError::new_err(format!("{}: {}", error.code, error.message));
        fallback.set_cause(py, Some(failure));
        fallback
    })
}
fn iresiphi_error(py: Python<'_>, error: &Error) -> PyErr {
    let class = if error.code == "HANDLE_CLOSED" {
        "HandleClosedError"
    } else {
        "IresiphiError"
    };
    raise(py, class, error, None)
}
fn closed_error(py: Python<'_>, handle: &str) -> PyErr {
    iresiphi_error(
        py,
        &record(
            "HANDLE_CLOSED",
            "python",
            format!("the {handle} handle is closed"),
        )
        .with_subject(handle.to_string())
        .with_remediation(
            "Use values returned before close, or open a new handle; closed handles release their Rust state.",
        ),
    )
}

// ---------------------------------------------------------------------------
// Python -> Rust: only portable declarative values cross the boundary.

fn non_portable(place: &str, message: impl Into<String>) -> Error {
    record("NON_PORTABLE", "portability", message)
        .with_subject(if place.is_empty() { "/" } else { place }.to_string())
        .with_remediation(
            "Recipes, bindings and policies are portable data: use str, int, bool, None, lists, dicts and the tagged literal builders in iresiphi.build. Host callbacks, floats and live objects are rejected.",
        )
}

/// Convert a Python value into JSON, rejecting anything host-only. Exact numeric
/// domains must already be tagged strings (see `iresiphi.build.lit`).
fn py_to_json(value: &Bound<'_, PyAny>, place: &str) -> Result<Json, Error> {
    let fail = |message: String| non_portable(place, message);
    let kind = || {
        value
            .get_type()
            .name()
            .map(|n| n.to_string())
            .unwrap_or_else(|_| "object".into())
    };
    if value.is_none() {
        return Ok(Json::Null);
    }
    if let Ok(flag) = value.cast::<PyBool>() {
        return Ok(Json::Bool(flag.is_true()));
    }
    if value.is_instance_of::<PyInt>() {
        let text = value.str().map_err(|e| fail(e.to_string()))?.to_string();
        return serde_json::from_str(&text).map_err(|e| fail(e.to_string()));
    }
    if value.is_instance_of::<PyFloat>() {
        return Err(fail(
            "binary floating-point values are not exact; use iresiphi.build.lit(Decimal(...)) or a tagged decimal string".into(),
        ));
    }
    if let Ok(text) = value.cast::<PyString>() {
        return Ok(Json::String(
            text.to_str().map_err(|e| fail(e.to_string()))?.to_owned(),
        ));
    }
    if let Ok(dict) = value.cast::<PyDict>() {
        let mut map = serde_json::Map::new();
        for (key, item) in dict.iter() {
            let key = key
                .cast::<PyString>()
                .map_err(|_| fail("object keys must be str".into()))?
                .to_str()
                .map_err(|e| fail(e.to_string()))?
                .to_owned();
            let child = py_to_json(&item, &format!("{place}/{key}"))?;
            map.insert(key, child);
        }
        return Ok(Json::Object(map));
    }
    if value.is_instance_of::<PyList>() || value.is_instance_of::<PyTuple>() {
        let mut items = Vec::new();
        for (index, item) in value
            .try_iter()
            .map_err(|e| fail(e.to_string()))?
            .enumerate()
        {
            let item = item.map_err(|e| fail(e.to_string()))?;
            items.push(py_to_json(&item, &format!("{place}/{index}"))?);
        }
        return Ok(Json::Array(items));
    }
    // os.PathLike (pathlib.Path) is accepted for binding paths and roots.
    if value.hasattr("__fspath__").unwrap_or(false) {
        let path = value
            .call_method0("__fspath__")
            .map_err(|e| fail(e.to_string()))?;
        if let Ok(text) = path.cast::<PyString>() {
            return Ok(Json::String(
                text.to_str().map_err(|e| fail(e.to_string()))?.to_owned(),
            ));
        }
        return Err(fail("only str file-system paths are supported".into()));
    }
    let kind = kind();
    if matches!(kind.as_str(), "Decimal" | "date" | "datetime") {
        return Err(fail(format!(
            "a bare {kind} has no declared type; wrap it with iresiphi.build.lit(...)"
        )));
    }
    if value.is_callable() {
        return Err(fail(format!(
            "host-only callable {kind} cannot be saved or executed portably; Python callbacks are rejected"
        )));
    }
    Err(fail(format!("{kind} is not a portable recipe value")))
}

/// Text for a Rust model parse: `str` is used verbatim (it is a document);
/// anything else must be portable declarative data.
fn document_text(value: &Bound<'_, PyAny>) -> Result<String, Error> {
    if let Ok(text) = value.cast::<PyString>() {
        return text
            .to_str()
            .map(str::to_owned)
            .map_err(|e| non_portable("", e.to_string()));
    }
    Ok(py_to_json(value, "")?.to_string())
}

fn parse_argument<T: serde::de::DeserializeOwned>(
    value: &Bound<'_, PyAny>,
    code: &str,
    phase: &str,
) -> Result<T, Error> {
    let text = document_text(value)?;
    serde_json::from_str(&text).map_err(|e| record(code, phase, e.to_string()))
}

// ---------------------------------------------------------------------------
// Rust -> Python: exact copies.

static DECIMAL: PyOnceLock<Py<pyo3::types::PyType>> = PyOnceLock::new();

fn decimal<'py>(py: Python<'py>, text: &str) -> PyResult<Bound<'py, PyAny>> {
    DECIMAL.import(py, "decimal", "Decimal")?.call1((text,))
}

fn int_from_text<'py>(py: Python<'py>, text: &str) -> PyResult<Bound<'py, PyAny>> {
    if let Ok(value) = text.parse::<i64>() {
        return Ok(value.into_pyobject(py)?.into_any());
    }
    py.get_type::<PyInt>().call1((text,))
}

/// Materialize one tagged scalar as an owned Python value.
fn value_to_py<'py>(py: Python<'py>, value: &Value) -> PyResult<Bound<'py, PyAny>> {
    Ok(match value {
        Value::Null => py.None().into_bound(py),
        Value::Boolean { value } => PyBool::new(py, *value).to_owned().into_any(),
        Value::String { value } => PyString::new(py, value).into_any(),
        Value::Int64 { value } => {
            let parsed: i64 = value.parse().map_err(|_| {
                iresiphi_error(
                    py,
                    &record("INVALID_VALUE", "python", "Int64 text is not a signed 64-bit integer"),
                )
            })?;
            parsed.into_pyobject(py)?.into_any()
        }
        Value::Decimal { value, .. } => decimal(py, value)?,
        Value::Date { value } => {
            let date = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
                iresiphi_error(py, &record("INVALID_VALUE", "python", "invalid calendar date"))
            })?;
            use chrono::Datelike;
            if !(1..=9999).contains(&date.year()) {
                return Err(iresiphi_error(
                    py,
                    &record(
                        "VALUE_OUT_OF_RANGE",
                        "python",
                        "date year is outside datetime.date's 1..9999 range",
                    )
                    .with_remediation(
                        "Read this value from a bundle or the tagged evidence text; it is not representable as datetime.date.",
                    ),
                ));
            }
            PyDate::new(py, date.year(), date.month() as u8, date.day() as u8)?.into_any()
        }
    })
}

/// Plain JSON (manifests, diagnostics, evidence). Numbers keep their exact
/// lexical form: integers become `int`, anything else `decimal.Decimal`.
fn json_to_py<'py>(py: Python<'py>, value: &Json) -> PyResult<Bound<'py, PyAny>> {
    Ok(match value {
        Json::Null => py.None().into_bound(py),
        Json::Bool(flag) => PyBool::new(py, *flag).to_owned().into_any(),
        Json::Number(number) => {
            let text = number.to_string();
            if text.contains(['.', 'e', 'E']) {
                decimal(py, &text)?
            } else {
                int_from_text(py, &text)?
            }
        }
        Json::String(text) => PyString::new(py, text).into_any(),
        Json::Array(items) => {
            let list = PyList::empty(py);
            for item in items {
                list.append(json_to_py(py, item)?)?;
            }
            list.into_any()
        }
        Json::Object(map) => {
            let dict = PyDict::new(py);
            for (key, item) in map {
                dict.set_item(key, json_to_py(py, item)?)?;
            }
            dict.into_any()
        }
    })
}

fn to_py<T: serde::Serialize>(py: Python<'_>, value: &T) -> PyResult<Py<PyAny>> {
    let value = serde_json::to_value(value)
        .map_err(|e| iresiphi_error(py, &record("INTERNAL_ERROR", "python", e.to_string())))?;
    Ok(json_to_py(py, &value)?.unbind())
}

// ---------------------------------------------------------------------------
// Recipe and catalog handles.

/// A validated, Rust-owned `.iresiphi` recipe.
#[pyclass(name = "Recipe", module = "iresiphi._native", frozen)]
struct PyRecipe {
    inner: Mutex<Option<Recipe>>,
}
impl PyRecipe {
    fn owned(recipe: Recipe) -> Self {
        Self {
            inner: Mutex::new(Some(recipe)),
        }
    }
    /// A copy of the recipe; the copy is what runs, so closing a handle never
    /// affects work already started.
    fn snapshot(&self, py: Python<'_>) -> PyResult<Recipe> {
        self.with(py, |r| Ok(r.clone()))
    }
    fn with<T>(&self, py: Python<'_>, f: impl FnOnce(&mut Recipe) -> PyResult<T>) -> PyResult<T> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_mut() {
            Some(recipe) => f(recipe),
            None => Err(closed_error(py, "Recipe")),
        }
    }
}
#[pymethods]
impl PyRecipe {
    /// Load from a `.iresiphi` JSON document (`str`) or portable declarative data
    /// (`dict` built with `iresiphi.build`). Rust performs all validation.
    #[new]
    fn new(py: Python<'_>, document: &Bound<'_, PyAny>) -> PyResult<Self> {
        let text = document_text(document).map_err(|e| iresiphi_error(py, &e))?;
        let recipe = Recipe::from_json(&text).map_err(|e| iresiphi_error(py, &e.completed()))?;
        Ok(Self::owned(recipe))
    }
    #[staticmethod]
    fn from_json(py: Python<'_>, text: &str) -> PyResult<Self> {
        let recipe = Recipe::from_json(text).map_err(|e| iresiphi_error(py, &e.completed()))?;
        Ok(Self::owned(recipe))
    }
    fn to_json(&self, py: Python<'_>) -> PyResult<String> {
        self.with(py, |r| r.to_json().map_err(|e| iresiphi_error(py, &e.completed())))
    }
    fn canonical_json(&self, py: Python<'_>) -> PyResult<String> {
        self.with(py, |r| {
            r.canonical_json()
                .map_err(|e| iresiphi_error(py, &e.completed()))
        })
    }
    /// SHA-256 of the canonical JSON: the recipe's semantic identity.
    #[getter]
    fn hash(&self, py: Python<'_>) -> PyResult<String> {
        self.with(py, |r| r.hash().map_err(|e| iresiphi_error(py, &e.completed())))
    }
    #[getter]
    fn name(&self, py: Python<'_>) -> PyResult<String> {
        self.with(py, |r| Ok(r.name.clone()))
    }
    #[getter]
    fn format_version(&self, py: Python<'_>) -> PyResult<String> {
        self.with(py, |r| Ok(r.format_version.clone()))
    }
    /// The recipe as plain portable data (tagged literals stay tagged strings).
    fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.with(py, |r| to_py(py, &*r))
    }
    /// `validate(recipe)` or, with bindings and policy, bound validation with
    /// `data_checks` `"none"` (default) or `"full"`.
    #[pyo3(signature = (bindings=None, policy=None, data_checks="none"))]
    fn validate(
        &self,
        py: Python<'_>,
        bindings: Option<&Bound<'_, PyAny>>,
        policy: Option<&Bound<'_, PyAny>>,
        data_checks: &str,
    ) -> PyResult<Py<PyAny>> {
        let recipe = self.snapshot(py)?;
        let report = match (bindings, policy) {
            (None, None) => {
                if data_checks != "none" {
                    return Err(iresiphi_error(
                        py,
                        &record(
                            "MISSING_BINDING",
                            "validation",
                            "data checks require bindings and a policy",
                        ),
                    ));
                }
                crate::validate(&recipe)
            }
            (Some(bindings), Some(policy)) => {
                let (bindings, policy) = arguments(py, bindings, policy)?;
                let data_checks = data_checks.to_owned();
                // Full data checks read sources: do that work detached.
                py.detach(move || {
                    crate::validation::validate_bound(&recipe, &bindings, &policy, &data_checks)
                })
            }
            _ => {
                return Err(iresiphi_error(
                    py,
                    &record(
                        "INVALID_ARGUMENT",
                        "validation",
                        "bindings and policy must be supplied together",
                    ),
                ));
            }
        };
        to_py(py, &report)
    }
    #[pyo3(signature = (bindings=None, policy=None))]
    fn explain(
        &self,
        py: Python<'_>,
        bindings: Option<&Bound<'_, PyAny>>,
        policy: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let recipe = self.snapshot(py)?;
        let explanation = match (bindings, policy) {
            (None, None) => crate::explain(&recipe),
            (Some(bindings), Some(policy)) => {
                let (bindings, policy) = arguments(py, bindings, policy)?;
                py.detach(move || crate::validation::explain_bound(&recipe, &bindings, &policy))
            }
            _ => {
                return Err(iresiphi_error(
                    py,
                    &record(
                        "INVALID_ARGUMENT",
                        "explain",
                        "bindings and policy must be supplied together",
                    ),
                ));
            }
        };
        to_py(py, &explanation)
    }
    /// Copy a catalog declaration into this recipe (self-contained bundling).
    #[pyo3(signature = (catalog, id, alias=None))]
    fn bundle_from(
        &self,
        py: Python<'_>,
        catalog: &PyCatalog,
        id: &str,
        alias: Option<&str>,
    ) -> PyResult<()> {
        let catalog = catalog.snapshot(py)?;
        self.with(py, |r| {
            let mut updated = r.clone();
            updated
                .bundle_from(&catalog, id, alias)
                .map_err(|e| iresiphi_error(py, &e.completed()))?;
            *r = updated;
            Ok(())
        })
    }
    #[pyo3(signature = (id, provenance=None))]
    fn bundle_definition(&self, py: Python<'_>, id: &str, provenance: Option<String>) -> PyResult<()> {
        self.with(py, |r| {
            let mut updated = r.clone();
            updated
                .bundle_definition(id, provenance)
                .map_err(|e| iresiphi_error(py, &e.completed()))?;
            *r = updated;
            Ok(())
        })
    }
    /// An independent handle over a copy of this recipe.
    fn copy(&self, py: Python<'_>) -> PyResult<Self> {
        Ok(Self::owned(self.snapshot(py)?))
    }
    /// Release the Rust recipe. Repeated close is harmless.
    fn close(&self) {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).take();
    }
    #[getter]
    fn closed(&self) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_none()
    }
    fn __enter__(slf: Py<Self>) -> Py<Self> {
        slf
    }
    fn __exit__(
        &self,
        _kind: &Bound<'_, PyAny>,
        _value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) -> bool {
        self.close();
        false
    }
}

fn arguments(
    py: Python<'_>,
    bindings: &Bound<'_, PyAny>,
    policy: &Bound<'_, PyAny>,
) -> PyResult<(Bindings, Policy)> {
    let bindings: Bindings =
        parse_argument(bindings, "INVALID_BINDING", "binding").map_err(|e| iresiphi_error(py, &e))?;
    let policy: Policy =
        parse_argument(policy, "INVALID_POLICY", "policy").map_err(|e| iresiphi_error(py, &e))?;
    Ok((bindings, policy))
}

/// An in-memory catalog of reusable declarations (decision #8).
#[pyclass(name = "Catalog", module = "iresiphi._native", frozen)]
struct PyCatalog {
    inner: Mutex<Option<Catalog>>,
}
impl PyCatalog {
    fn snapshot(&self, py: Python<'_>) -> PyResult<Catalog> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| closed_error(py, "Catalog"))
    }
}
#[pymethods]
impl PyCatalog {
    #[new]
    fn new(py: Python<'_>, document: &Bound<'_, PyAny>) -> PyResult<Self> {
        let catalog: Catalog =
            parse_argument(document, "INVALID_CATALOG", "catalog").map_err(|e| iresiphi_error(py, &e))?;
        Ok(Self {
            inner: Mutex::new(Some(catalog)),
        })
    }
    fn close(&self) {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).take();
    }
    #[getter]
    fn closed(&self) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_none()
    }
    fn __enter__(slf: Py<Self>) -> Py<Self> {
        slf
    }
    fn __exit__(
        &self,
        _kind: &Bound<'_, PyAny>,
        _value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) -> bool {
        self.close();
        false
    }
}

// ---------------------------------------------------------------------------
// Cancellation.

/// The common run cancellation token (decision #13). Clones share one request.
#[pyclass(name = "CancellationToken", module = "iresiphi._native", frozen)]
#[derive(Default)]
struct PyToken {
    inner: CancellationToken,
}
#[pymethods]
impl PyToken {
    #[new]
    fn new() -> Self {
        Self::default()
    }
    /// Request cancellation; safe from any thread, any number of times.
    fn cancel(&self) {
        self.inner.cancel();
    }
    #[getter]
    fn cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }
    /// Cancel as soon as a run using this token enters `phase`
    /// (`"parse"`, `"join"`, `"write"` or `"committed"`).
    fn cancel_at(&self, py: Python<'_>, phase: &str) -> PyResult<()> {
        let phase = match phase {
            "parse" => RunPhase::Parse,
            "join" => RunPhase::Join,
            "write" => RunPhase::Write,
            "committed" => RunPhase::Committed,
            other => {
                return Err(iresiphi_error(
                    py,
                    &record("INVALID_ARGUMENT", "cancellation", format!("unknown run phase {other:?}")),
                ));
            }
        };
        self.inner.cancel_at(phase);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Run results.

/// A finished run: status, manifest, diagnostics and (for memory runs) result
/// tables and evidence, owned by Rust until `close()`.
#[pyclass(name = "RunResult", module = "iresiphi._native", frozen)]
struct PyRun {
    inner: Mutex<Option<Arc<RunReport>>>,
    batch_rows: usize,
    batch_bytes: usize,
}
#[derive(Clone, Copy)]
enum Records {
    Table,
    Evidence,
    Candidates,
    Details,
}
impl PyRun {
    fn report(&self, py: Python<'_>) -> PyResult<Arc<RunReport>> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| closed_error(py, "RunResult"))
    }
    fn run_id_unchecked(&self) -> Option<String> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|r| r.run_id.clone())
    }
    fn evidence_kind(py: Python<'_>, kind: &str) -> PyResult<Records> {
        Ok(match kind {
            "primary" => Records::Evidence,
            "candidate" => Records::Candidates,
            "detail" => Records::Details,
            other => {
                return Err(iresiphi_error(
                    py,
                    &record(
                        "INVALID_ARGUMENT",
                        "python",
                        format!("evidence kind {other:?} is not primary, candidate or detail"),
                    ),
                ));
            }
        })
    }
    fn limit(&self, py: Python<'_>, rows: Option<usize>) -> PyResult<usize> {
        let cap = self.batch_rows;
        match rows {
            None => Ok(cap),
            Some(0) => Err(iresiphi_error(
                py,
                &record("INVALID_ARGUMENT", "python", "batch rows must be positive"),
            )),
            Some(rows) if rows > cap => Err(iresiphi_error(
                py,
                &record(
                    "RESOURCE_LIMIT",
                    "python",
                    format!("a batch of {rows} rows exceeds the batch row limit {cap}"),
                )
                .with_check("budget".to_string()),
            )),
            Some(rows) => Ok(rows),
        }
    }
    fn table<'a>(py: Python<'_>, report: &'a RunReport, name: &str) -> PyResult<&'a crate::engine::Table> {
        report.tables.get(name).ok_or_else(|| {
            iresiphi_error(
                py,
                &record(
                    "UNKNOWN_OUTPUT",
                    "python",
                    format!("run has no memory result table {name:?}"),
                )
                .with_subject(format!("output:{name}")),
            )
        })
    }
    fn columns(report: &RunReport, name: &str) -> Option<Vec<String>> {
        report.manifest["schema"][name]["columns"].as_array().map(|columns| {
            columns
                .iter()
                .filter_map(|c| c["name"].as_str().map(str::to_owned))
                .collect()
        })
    }
    /// Copy at most `rows` records starting at `offset`, stopping before the
    /// batch's decoded size would exceed the policy's per-batch byte budget.
    /// Returns the copied records and the next offset (`None` at the end).
    fn copy_batch<'py>(
        &self,
        py: Python<'py>,
        records: Records,
        name: &str,
        offset: usize,
        rows: Option<usize>,
    ) -> PyResult<(Bound<'py, PyList>, Option<usize>)> {
        let rows = self.limit(py, rows)?;
        let report = self.report(py)?;
        let list = PyList::empty(py);
        let mut bytes = 0usize;
        let over = |py: Python<'_>, size: usize| {
            iresiphi_error(
                py,
                &record(
                    "RESOURCE_LIMIT",
                    "python",
                    format!(
                        "one record of {size} decoded bytes exceeds the batch byte limit {}",
                        self.batch_bytes
                    ),
                )
                .with_check("budget".to_string()),
            )
        };
        let total;
        let mut index = offset;
        match records {
            Records::Table => {
                let table = Self::table(py, &report, name)?;
                total = table.len();
                let order = Self::columns(&report, name);
                while index < total && index - offset < rows {
                    let row = &table[index];
                    let size: usize = row
                        .iter()
                        .map(|(k, v)| k.len() + 16 + value_bytes(v))
                        .sum();
                    if size > self.batch_bytes {
                        return Err(over(py, size));
                    }
                    if bytes + size > self.batch_bytes {
                        break;
                    }
                    bytes += size;
                    let dict = PyDict::new(py);
                    match &order {
                        Some(order) => {
                            for column in order {
                                if let Some(value) = row.get(column) {
                                    dict.set_item(column, value_to_py(py, value)?)?;
                                }
                            }
                            for (column, value) in row {
                                if !order.contains(column) {
                                    dict.set_item(column, value_to_py(py, value)?)?;
                                }
                            }
                        }
                        None => {
                            for (column, value) in row {
                                dict.set_item(column, value_to_py(py, value)?)?;
                            }
                        }
                    }
                    list.append(dict)?;
                    index += 1;
                }
            }
            Records::Evidence | Records::Candidates | Records::Details => {
                let evidence = match records {
                    Records::Evidence => &report.evidence,
                    Records::Candidates => &report.candidate_evidence,
                    _ => &report.detail_evidence,
                };
                total = evidence.len();
                while index < total && index - offset < rows {
                    let size = serde_json::to_string(&evidence[index])
                        .map(|t| t.len())
                        .unwrap_or(0);
                    if size > self.batch_bytes {
                        return Err(over(py, size));
                    }
                    if bytes + size > self.batch_bytes {
                        break;
                    }
                    bytes += size;
                    list.append(json_to_py(py, &evidence[index])?)?;
                    index += 1;
                }
            }
        }
        Ok((list, (index < total).then_some(index)))
    }
}
fn value_bytes(value: &Value) -> usize {
    match value {
        Value::Null | Value::Boolean { .. } => 1,
        Value::Int64 { value }
        | Value::String { value }
        | Value::Decimal { value, .. }
        | Value::Date { value } => value.len(),
    }
}
#[pymethods]
impl PyRun {
    #[getter]
    fn run_id(&self, py: Python<'_>) -> PyResult<String> {
        Ok(self.report(py)?.run_id.clone())
    }
    /// `succeeded`, `succeeded_with_unresolved`, `failed` or `cancelled`.
    #[getter]
    fn status(&self, py: Python<'_>) -> PyResult<String> {
        Ok(self.report(py)?.status.clone())
    }
    #[getter]
    fn manifest(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_py(py, &self.report(py)?.manifest)
    }
    #[getter]
    fn diagnostics(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_py(py, &self.report(py)?.diagnostics)
    }
    /// Names of the memory result tables.
    #[getter]
    fn outputs(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        Ok(self.report(py)?.tables.keys().cloned().collect())
    }
    /// Typed column schema of a memory result table: a list of
    /// `{"name": ..., "type": {"kind": ..., "precision": ..., "scale": ...}}`.
    fn schema(&self, py: Python<'_>, table: &str) -> PyResult<Py<PyAny>> {
        let report = self.report(py)?;
        Self::table(py, &report, table)?;
        to_py(py, &report.manifest["schema"][table]["columns"])
    }
    fn row_count(&self, py: Python<'_>, table: &str) -> PyResult<usize> {
        let report = self.report(py)?;
        Ok(Self::table(py, &report, table)?.len())
    }
    #[pyo3(signature = (kind="primary"))]
    fn evidence_count(&self, py: Python<'_>, kind: &str) -> PyResult<usize> {
        let report = self.report(py)?;
        Ok(match Self::evidence_kind(py, kind)? {
            Records::Candidates => report.candidate_evidence.len(),
            Records::Details => report.detail_evidence.len(),
            _ => report.evidence.len(),
        })
    }
    /// Largest number of rows one batch may copy.
    #[getter]
    fn batch_row_limit(&self) -> usize {
        self.batch_rows
    }
    /// Copy one batch of rows from `table`. Returns a list of dicts.
    #[pyo3(signature = (table, offset=0, rows=None))]
    fn batch<'py>(
        &self,
        py: Python<'py>,
        table: &str,
        offset: usize,
        rows: Option<usize>,
    ) -> PyResult<Bound<'py, PyList>> {
        Ok(self.copy_batch(py, Records::Table, table, offset, rows)?.0)
    }
    /// Iterate over copied batches of `table`; each step copies one batch.
    #[pyo3(signature = (table, rows=None))]
    fn batches(slf: Py<Self>, py: Python<'_>, table: &str, rows: Option<usize>) -> PyResult<PyBatches> {
        {
            let this = slf.get();
            this.limit(py, rows)?;
            let report = this.report(py)?;
            Self::table(py, &report, table)?;
        }
        Ok(PyBatches {
            run: slf,
            records: Records::Table,
            name: table.to_owned(),
            rows,
            next: Mutex::new(Some(0)),
        })
    }
    /// Copy one batch of evidence records (`primary`, `candidate` or `detail`).
    /// Records are plain JSON; scalar values stay tagged exact text.
    #[pyo3(signature = (kind="primary", offset=0, rows=None))]
    fn evidence<'py>(
        &self,
        py: Python<'py>,
        kind: &str,
        offset: usize,
        rows: Option<usize>,
    ) -> PyResult<Bound<'py, PyList>> {
        let records = Self::evidence_kind(py, kind)?;
        Ok(self.copy_batch(py, records, kind, offset, rows)?.0)
    }
    #[pyo3(signature = (kind="primary", rows=None))]
    fn evidence_batches(
        slf: Py<Self>,
        py: Python<'_>,
        kind: &str,
        rows: Option<usize>,
    ) -> PyResult<PyBatches> {
        let records = Self::evidence_kind(py, kind)?;
        {
            let this = slf.get();
            this.limit(py, rows)?;
            this.report(py)?;
        }
        Ok(PyBatches {
            run: slf,
            records,
            name: kind.to_owned(),
            rows,
            next: Mutex::new(Some(0)),
        })
    }
    /// Release retained tables, evidence and the report. Values already copied
    /// into Python stay valid. Repeated close is harmless.
    fn close(&self) {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).take();
    }
    #[getter]
    fn closed(&self) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_none()
    }
    fn __enter__(slf: Py<Self>) -> Py<Self> {
        slf
    }
    fn __exit__(
        &self,
        _kind: &Bound<'_, PyAny>,
        _value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) -> bool {
        self.close();
        false
    }
    fn __repr__(&self) -> String {
        match self.inner.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            Some(r) => format!("<iresiphi.RunResult {} {}>", r.run_id, r.status),
            None => "<iresiphi.RunResult closed>".into(),
        }
    }
}

/// Bounded iteration: each `next()` copies exactly one batch.
#[pyclass(name = "BatchIterator", module = "iresiphi._native", frozen)]
struct PyBatches {
    run: Py<PyRun>,
    records: Records,
    name: String,
    rows: Option<usize>,
    next: Mutex<Option<usize>>,
}
#[pymethods]
impl PyBatches {
    fn __iter__(slf: Py<Self>) -> Py<Self> {
        slf
    }
    fn __next__<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyList>>> {
        let Some(offset) = *self.next.lock().unwrap_or_else(|e| e.into_inner()) else {
            return Ok(None);
        };
        let (batch, next) =
            self.run
                .get()
                .copy_batch(py, self.records, &self.name, offset, self.rows)?;
        if batch.is_empty() {
            *self.next.lock().unwrap_or_else(|e| e.into_inner()) = None;
            return Ok(None);
        }
        *self.next.lock().unwrap_or_else(|e| e.into_inner()) = next;
        Ok(Some(batch))
    }
}

// ---------------------------------------------------------------------------
// Execution.

/// One execution context: one active run at a time (`RUN_BUSY` otherwise).
#[pyclass(name = "ExecutionContext", module = "iresiphi._native", frozen)]
#[derive(Default)]
struct PyContext {
    inner: Arc<ExecutionContext>,
}
#[pymethods]
impl PyContext {
    #[new]
    fn new() -> Self {
        Self::default()
    }
    /// Run `recipe` to memory results, or deliver a bundle at a NEW `destination`.
    ///
    /// Returns a `RunResult` for `succeeded`/`succeeded_with_unresolved`. A failed
    /// run raises `IresiphiError`; a cancelled run raises `RunCancelled`; a
    /// `KeyboardInterrupt` raises `RunInterrupted`. Each carries the run report.
    #[pyo3(signature = (recipe, bindings, policy, cancel=None, destination=None))]
    fn run(
        &self,
        py: Python<'_>,
        recipe: &PyRecipe,
        bindings: &Bound<'_, PyAny>,
        policy: &Bound<'_, PyAny>,
        cancel: Option<&PyToken>,
        destination: Option<PathBuf>,
    ) -> PyResult<Py<PyRun>> {
        let recipe = recipe.snapshot(py)?;
        let (bindings, policy) = arguments(py, bindings, policy)?;
        let batch_rows = policy.limits.batch_rows.clamp(1, MAX_BATCH_ROWS);
        let batch_bytes = policy.limits.batch_bytes.max(1);
        let token = cancel.map(|c| c.inner.clone()).unwrap_or_default();
        let worker_token = token.clone();
        let context = self.inner.clone();
        let (sender, receiver) = mpsc::channel();
        // The worker owns Rust copies only; it never touches the interpreter.
        let worker = std::thread::Builder::new()
            .name("iresiphi-run".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    match destination {
                        Some(path) => {
                            context.run_bundle(&recipe, &bindings, &policy, &path, &worker_token)
                        }
                        None => context.run(&recipe, &bindings, &policy, &worker_token),
                    }
                }));
                let _ = sender.send(result);
            })
            .map_err(|e| iresiphi_error(py, &record("INTERNAL_ERROR", "execution", e.to_string())))?;
        let mut receiver = Some(receiver);
        let mut signal: Option<PyErr> = None;
        let outcome = loop {
            let waiting = receiver.take().expect("receiver present while waiting");
            let (waiting, received) = py.detach(move || {
                let received = waiting.recv_timeout(SIGNAL_CHECK);
                (waiting, received)
            });
            receiver = Some(waiting);
            match received {
                Ok(outcome) => break Some(outcome),
                Err(mpsc::RecvTimeoutError::Disconnected) => break None,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            // Re-attached: run Python signal handlers. The first interrupt requests
            // cooperative cancellation; the run is still awaited so its worker
            // shutdown completes and its report is preserved.
            if let Err(error) = py.check_signals() {
                token.cancel();
                signal.get_or_insert(error);
            }
        };
        let _ = py.detach(move || worker.join());
        let report = match outcome {
            Some(Ok(report)) => report,
            failure => {
                let detail = match failure {
                    Some(Err(payload)) => payload
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                        .unwrap_or_else(|| "worker panicked".into()),
                    _ => "worker exited without a report".into(),
                };
                let error = record("INTERNAL_ERROR", "execution", detail);
                let err = iresiphi_error(py, &error);
                if let Some(signal) = signal {
                    err.set_cause(py, Some(signal));
                }
                return Err(err);
            }
        };
        let status = report.status.clone();
        let primary = report
            .diagnostics
            .iter()
            .find(|d| d.severity == "error")
            .or_else(|| report.diagnostics.first())
            .cloned();
        let handle = Py::new(
            py,
            PyRun {
                inner: Mutex::new(Some(Arc::new(report))),
                batch_rows,
                batch_bytes,
            },
        )?;
        if let Some(signal) = signal {
            if !signal.is_instance_of::<PyKeyboardInterrupt>(py) {
                // A user signal handler raised something else: keep its type and
                // attach the report so the run is not lost.
                let _ = signal.value(py).setattr("iresiphi_report", handle.clone_ref(py));
                return Err(signal);
            }
            let error = match (status.as_str(), primary) {
                ("cancelled", Some(error)) => error,
                _ => record(
                    "INTERRUPTED",
                    "python",
                    format!(
                        "KeyboardInterrupt arrived after the run finished with status {status}; the result was not revoked"
                    ),
                ),
            };
            let err = raise(py, "RunInterrupted", &error, Some(handle));
            err.set_cause(py, Some(signal));
            return Err(err);
        }
        match status.as_str() {
            "succeeded" | "succeeded_with_unresolved" => Ok(handle),
            "cancelled" => {
                let error = primary
                    .unwrap_or_else(|| record("CANCELLED", "execution", "run cancelled"));
                Err(raise(py, "RunCancelled", &error, Some(handle)))
            }
            _ => {
                let error = primary
                    .unwrap_or_else(|| record("EXECUTION_ERROR", "execution", "run failed"));
                Err(raise(py, "IresiphiError", &error, Some(handle)))
            }
        }
    }
}

/// Read a committed directory bundle back into exact Python rows.
#[pyfunction]
fn read_bundle(py: Python<'_>, path: PathBuf) -> PyResult<Py<PyAny>> {
    let tables = py
        .detach(move || crate::sink::read_bundle(&path))
        .map_err(|e| iresiphi_error(py, &e.completed()))?;
    let result = PyDict::new(py);
    for (name, rows) in &tables {
        let list = PyList::empty(py);
        for row in rows {
            let dict = PyDict::new(py);
            for (column, value) in row {
                dict.set_item(column, value_to_py(py, value)?)?;
            }
            list.append(dict)?;
        }
        result.set_item(name, list)?;
    }
    Ok(result.into_any().unbind())
}

/// Convert one tagged exact value (`{"kind": ..., "value": ...}`) to Python.
#[pyfunction]
fn from_tagged(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let json = py_to_json(value, "").map_err(|e| iresiphi_error(py, &e))?;
    let tagged: Value = serde_json::from_value(json)
        .map_err(|e| iresiphi_error(py, &record("INVALID_VALUE", "python", e.to_string())))?;
    let tagged = crate::validation::canonical_value(&tagged).map_err(|e| iresiphi_error(py, &e.completed()))?;
    Ok(value_to_py(py, &tagged)?.unbind())
}

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyRecipe>()?;
    module.add_class::<PyCatalog>()?;
    module.add_class::<PyToken>()?;
    module.add_class::<PyContext>()?;
    module.add_class::<PyRun>()?;
    module.add_class::<PyBatches>()?;
    module.add_function(wrap_pyfunction!(read_bundle, module)?)?;
    module.add_function(wrap_pyfunction!(from_tagged, module)?)?;
    module.add("FORMAT_VERSION", "0.1")?;
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    module.add("MAX_BATCH_ROWS", MAX_BATCH_ROWS)?;
    module.add("SIGNAL_CHECK_MS", SIGNAL_CHECK.as_millis() as u64)?;
    Ok(())
}
