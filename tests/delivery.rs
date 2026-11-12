//! Cancellation and atomic local bundle delivery (#22, decision 13), verified through
//! visible outputs: reports, manifests, the destination and its parent directory.
#[path = "support/run_fixture.rs"]
mod run_fixture;
use iresiphi::{
    CancellationToken, ExecutionContext, RunPhase,
    model::Value,
    sink::{cleanup_staging, read_bundle},
};
use run_fixture::*;
use std::{
    collections::BTreeMap,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
    sync::{Mutex, atomic::AtomicBool},
};

/// Engine worker threads are process-wide; tests observing them run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());
fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}
fn go() -> AtomicBool {
    AtomicBool::new(false)
}
fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}
fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect()
}

#[test]
fn memory_and_bundle_share_typed_schema_and_manifest_and_csv_round_trips_exactly() {
    let _serial = serial();
    let f = fixture();
    let context = ExecutionContext::default();
    let memory = context.run(&f.recipe, &f.bindings, &f.policy, &go());
    assert_eq!(memory.status, "succeeded_with_unresolved");
    assert_eq!(memory.manifest["complete"], true);
    let bundled = context.run_bundle(&f.recipe, &f.bindings, &f.policy, &f.destination(), &go());
    assert_eq!(
        bundled.status, "succeeded_with_unresolved",
        "{:?}",
        bundled.diagnostics
    );

    let schema = read_json(&f.destination().join("schema.json"));
    assert_eq!(
        schema["tables"]["review"],
        memory.manifest["schema"]["review"]
    );
    assert_eq!(schema["csv"]["null"], "\\N");
    assert_eq!(schema["csv"]["record_ending"], "LF");

    // The bundle's complete-marker manifest is the run's manifest.
    let manifest = read_json(&f.destination().join("manifest.json"));
    assert_eq!(manifest["complete"], true);
    assert_eq!(manifest["run_id"], bundled.run_id);
    for (key, value) in manifest.as_object().unwrap() {
        assert_eq!(&bundled.manifest[key], value, "manifest key {key}");
    }

    // Reading the bundle back yields exactly the typed memory result.
    let tables = read_bundle(&f.destination()).unwrap();
    assert_eq!(tables["review"].len(), 3);
    let by_id = |rows: &Vec<BTreeMap<String, Value>>| -> BTreeMap<String, BTreeMap<String, Value>> {
        rows.iter()
            .map(|row| (serde_json::to_string(&row["id"]).unwrap(), row.clone()))
            .collect()
    };
    assert_eq!(by_id(&tables["review"]), by_id(&memory.tables["review"]));
    assert_eq!(by_id(&tables["review"]), by_id(&bundled.tables["review"]));
    let evidence: Vec<serde_json::Value> = tables["_evidence"]
        .iter()
        .map(|row| match &row["record_json"] {
            Value::String { value } => serde_json::from_str(value).unwrap(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(evidence, memory.evidence);

    // Encoding: one header row, LF endings, \N null, doubled leading backslash.
    let file = schema["csv"]["files"]["review"].as_str().unwrap();
    let text = std::fs::read_to_string(f.destination().join(file)).unwrap();
    assert!(!text.contains('\r'));
    assert!(text.ends_with('\n'));
    let mut reader = csv::Reader::from_reader(text.as_bytes());
    let headers = reader.headers().unwrap().clone();
    let rows: Vec<BTreeMap<String, String>> = reader
        .records()
        .map(|r| {
            headers
                .iter()
                .zip(r.unwrap().iter())
                .map(|(h, v)| (h.to_owned(), v.to_owned()))
                .collect()
        })
        .collect();
    let first = rows.iter().find(|r| r["id"] == "1").unwrap();
    assert_eq!(first["note"], "\\\\x");
    assert_eq!(first["empty"], "");
    assert_eq!(first["slash_n"], "\\\\N");
    assert_eq!(first["quoted"], "a,\"b\"\nc");
    let unmatched = rows.iter().find(|r| r["id"] == "3").unwrap();
    assert_eq!(unmatched["rate"], "\\N");
    assert_eq!(unmatched["note"], "\\\\\\N");
}

#[test]
fn staging_is_owner_only_unpredictable_and_inside_the_approved_parent() {
    let _serial = serial();
    let f = fixture();
    let names = Mutex::new(Vec::new());
    let observe = hook(|phase, _| {
        if phase == RunPhase::Write {
            let stages = staging_entries(f.root());
            assert_eq!(stages.len(), 1, "{stages:?}");
            let name = &stages[0];
            let suffix = name.strip_prefix(".iresiphi-").unwrap();
            assert_eq!(suffix.len(), 32);
            assert!(suffix.bytes().all(|b| b.is_ascii_hexdigit()));
            let meta = std::fs::symlink_metadata(f.root().join(name)).unwrap();
            assert!(meta.is_dir());
            assert_eq!(meta.mode() & 0o777, 0o700);
            assert_eq!(meta.uid(), unsafe { libc::geteuid() });
            names.lock().unwrap().push(name.clone());
        }
    });
    let context = ExecutionContext::default();
    for index in 0..2 {
        let destination = f.root().join(format!("bundle-{index}"));
        let report = context.run_bundle(&f.recipe, &f.bindings, &f.policy, &destination, &observe);
        assert_eq!(
            report.status, "succeeded_with_unresolved",
            "{:?}",
            report.diagnostics
        );
        let meta = std::fs::metadata(&destination).unwrap();
        assert_eq!(meta.mode() & 0o777, 0o700);
        for entry in std::fs::read_dir(&destination).unwrap() {
            let meta = entry.unwrap().metadata().unwrap();
            assert!(meta.is_file());
            assert_eq!(meta.mode() & 0o777, 0o600);
        }
        assert_eq!(
            report.manifest["cleanup"]["leftovers"],
            serde_json::json!([])
        );
    }
    let names = names.into_inner().unwrap();
    assert_eq!(names.len(), 2);
    assert_ne!(names[0], names[1], "staging names are not reused");
    assert!(staging_entries(f.root()).is_empty());
}

/// Cancellation at parse, join and write: a cancelled report, no committed output,
/// incomplete checks, no staging left behind and no engine workers still running.
#[test]
fn cancellation_at_parse_join_and_write_publishes_nothing_and_stops_owned_work() {
    let _serial = serial();
    let f = fixture();
    let context = ExecutionContext::default();
    for phase in [RunPhase::Parse, RunPhase::Join, RunPhase::Write] {
        let token = CancellationToken::new();
        token.cancel_at(phase);
        let workers = Mutex::new(None);
        let observed = hook(|entered, flag| {
            if entered == RunPhase::Join {
                *workers.lock().unwrap() = Some(engine_threads());
            }
            iresiphi::Cancellation::entered(&token, entered);
            if iresiphi::Cancellation::flag(&token).load(std::sync::atomic::Ordering::Acquire) {
                flag.store(true, std::sync::atomic::Ordering::Release);
            }
        });
        let started = std::time::Instant::now();
        let report = context.run_bundle(
            &f.recipe,
            &f.bindings,
            &f.policy,
            &f.destination(),
            &observed,
        );
        let latency = started.elapsed();
        assert_failed_without_results(&report, "cancelled", "CANCELLED");
        assert!(token.is_cancelled());
        assert!(!f.destination().exists(), "{phase:?}");
        assert!(staging_entries(f.root()).is_empty(), "{phase:?}");
        assert_eq!(
            engine_threads(),
            0,
            "{phase:?}: workers joined before reporting"
        );
        match phase {
            RunPhase::Parse => {
                assert_eq!(report.diagnostics[0].phase, "source");
                assert_eq!(report.manifest["checks"]["data"], "incomplete");
            }
            RunPhase::Join => {
                assert!(workers.lock().unwrap().unwrap() > 0, "engine was running");
                assert_eq!(report.manifest["checks"]["data"], "incomplete");
            }
            _ => {
                assert_eq!(report.diagnostics[0].phase, "delivery");
                assert_eq!(report.manifest["delivery"]["status"], "cancelled");
                assert_eq!(
                    report.manifest["cleanup"]["leftovers"],
                    serde_json::json!([])
                );
            }
        }
        eprintln!("cancel at {phase:?}: run returned after {latency:?}");
    }
    // The context is reusable after every cancellation.
    let ok = context.run_bundle(&f.recipe, &f.bindings, &f.policy, &f.destination(), &go());
    assert_eq!(ok.status, "succeeded_with_unresolved");
}

#[test]
fn cancellation_after_commit_cannot_revoke_the_bundle() {
    let _serial = serial();
    let f = fixture();
    let token = CancellationToken::new();
    token.cancel_at(RunPhase::Committed);
    let report = ExecutionContext::default().run_bundle(
        &f.recipe,
        &f.bindings,
        &f.policy,
        &f.destination(),
        &token,
    );
    assert!(token.is_cancelled());
    assert_eq!(
        report.status, "succeeded_with_unresolved",
        "{:?}",
        report.diagnostics
    );
    assert_eq!(report.manifest["complete"], true);
    assert_eq!(report.manifest["post_commit"]["cancel_requested"], true);
    assert_eq!(
        read_json(&f.destination().join("manifest.json"))["complete"],
        true
    );
    assert!(!report.tables.is_empty());
}

#[test]
fn post_commit_reporting_failure_leaves_a_discoverable_bundle_with_its_run_id() {
    let _serial = serial();
    let f = fixture();
    let context = ExecutionContext::default();
    let failing = hook(|phase, _| {
        if phase == RunPhase::Committed {
            panic!("reporting failed after commit");
        }
    });
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        context.run_bundle(
            &f.recipe,
            &f.bindings,
            &f.policy,
            &f.destination(),
            &failing,
        )
    }));
    assert!(outcome.is_err(), "the caller never received a report");
    let manifest = read_json(&f.destination().join("manifest.json"));
    assert_eq!(manifest["complete"], true);
    assert!(manifest["run_id"].as_str().is_some_and(|id| !id.is_empty()));
    assert!(read_bundle(&f.destination()).unwrap()["review"].len() == 3);
    assert!(staging_entries(f.root()).is_empty());
    // The context is not left busy, and a retry refuses to overwrite.
    let retry = context.run_bundle(&f.recipe, &f.bindings, &f.policy, &f.destination(), &go());
    assert_failed_without_results(&retry, "failed", "DESTINATION_EXISTS");
}

#[test]
fn existing_destinations_are_refused_on_every_retry_without_overwrite_or_merge() {
    let _serial = serial();
    let f = fixture();
    let context = ExecutionContext::default();
    let first = context.run_bundle(&f.recipe, &f.bindings, &f.policy, &f.destination(), &go());
    assert_eq!(first.status, "succeeded_with_unresolved");
    let before = snapshot(&f.destination());
    for _ in 0..3 {
        let retry = context.run_bundle(&f.recipe, &f.bindings, &f.policy, &f.destination(), &go());
        assert_failed_without_results(&retry, "failed", "DESTINATION_EXISTS");
        assert_eq!(retry.diagnostics[0].phase, "delivery");
        assert_eq!(snapshot(&f.destination()), before);
        assert!(staging_entries(f.root()).is_empty());
    }

    // Caller-owned destinations of every kind are left exactly as they were.
    let caller_dir = f.root().join("caller-dir");
    std::fs::create_dir(&caller_dir).unwrap();
    std::fs::write(caller_dir.join("keep.txt"), "mine").unwrap();
    let caller_file = f.root().join("caller-file");
    std::fs::write(&caller_file, "mine").unwrap();
    let dangling = f.root().join("dangling");
    std::os::unix::fs::symlink(f.root().join("nowhere"), &dangling).unwrap();
    for destination in [&caller_dir, &caller_file, &dangling] {
        let retry = context.run_bundle(&f.recipe, &f.bindings, &f.policy, destination, &go());
        assert_failed_without_results(&retry, "failed", "DESTINATION_EXISTS");
    }
    assert_eq!(snapshot(&caller_dir).len(), 1);
    assert_eq!(std::fs::read(&caller_file).unwrap(), b"mine");
    assert!(!f.root().join("nowhere").exists());
    assert!(staging_entries(f.root()).is_empty());
}

/// A destination that appears while the bundle is being staged is never replaced:
/// the no-replace rename itself refuses it.
#[test]
fn destination_created_during_staging_is_not_replaced() {
    let _serial = serial();
    let f = fixture();
    let racer = hook(|phase, _| {
        if phase == RunPhase::Write {
            std::fs::create_dir(f.destination()).unwrap();
            std::fs::write(f.destination().join("theirs.txt"), "theirs").unwrap();
        }
    });
    let report = ExecutionContext::default().run_bundle(
        &f.recipe,
        &f.bindings,
        &f.policy,
        &f.destination(),
        &racer,
    );
    assert_failed_without_results(&report, "failed", "DESTINATION_EXISTS");
    assert_eq!(
        snapshot(&f.destination()),
        BTreeMap::from([("theirs.txt".to_owned(), b"theirs".to_vec())])
    );
    assert!(staging_entries(f.root()).is_empty());
}

/// When owned staging cannot be removed, the report names the exact leftover path and
/// error; the explicit cleanup removes it later once the run is inactive.
#[test]
fn cleanup_failure_is_reported_with_leftover_paths_and_recoverable() {
    let _serial = serial();
    let f = fixture();
    let parent = f.root().join("out");
    std::fs::create_dir(&parent).unwrap();
    let destination = parent.join("bundle");
    let lock = hook(|phase, flag| {
        if phase == RunPhase::Write {
            std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o500)).unwrap();
            flag.store(true, std::sync::atomic::Ordering::Release);
        }
    });
    let report = ExecutionContext::default().run_bundle(
        &f.recipe,
        &f.bindings,
        &f.policy,
        &destination,
        &lock,
    );
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_failed_without_results(&report, "cancelled", "CANCELLED");
    let leftovers = report.manifest["cleanup"]["leftovers"].as_array().unwrap();
    assert_eq!(leftovers.len(), 1, "{leftovers:?}");
    let stages = staging_entries(&parent);
    assert_eq!(stages.len(), 1);
    assert_eq!(
        leftovers[0]["path"],
        parent.join(&stages[0]).to_string_lossy().as_ref()
    );
    assert!(!leftovers[0]["error"].as_str().unwrap().is_empty());
    assert!(!destination.exists());

    let cleanup = cleanup_staging(&parent, &f.policy).unwrap();
    assert_eq!(cleanup.removed, vec![parent.join(&stages[0])]);
    assert!(cleanup.leftovers.is_empty());
    assert!(staging_entries(&parent).is_empty());
}

/// Explicit cleanup removes only recognizable, inactive run-owned staging and never
/// recurses into or follows caller-owned look-alikes.
#[test]
fn explicit_cleanup_removes_only_inactive_run_owned_staging() {
    let _serial = serial();
    let f = fixture();
    let root = f.root();
    let hex = |c: char| std::iter::repeat_n(c, 32).collect::<String>();
    // Caller-owned look-alikes.
    let precious = root.join(format!(".iresiphi-{}", hex('a')));
    std::fs::create_dir(&precious).unwrap();
    std::fs::set_permissions(&precious, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(precious.join("precious.txt"), "mine").unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    std::fs::write(elsewhere.path().join("table-0.csv"), "mine").unwrap();
    let link = root.join(format!(".iresiphi-{}", hex('b')));
    std::os::unix::fs::symlink(elsewhere.path(), &link).unwrap();
    let plain = root.join(format!(".iresiphi-{}", hex('c')));
    std::fs::write(&plain, "mine").unwrap();
    let short = root.join(".iresiphi-123");
    std::fs::create_dir(&short).unwrap();
    // An abandoned stage, as left by a process that died while writing.
    let abandoned = root.join(format!(".iresiphi-{}", hex('d')));
    std::fs::create_dir(&abandoned).unwrap();
    std::fs::set_permissions(&abandoned, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(abandoned.join("table-0.csv"), "id\n").unwrap();
    std::fs::write(abandoned.join("schema.json"), "{}").unwrap();

    // A stage that belongs to a run still in progress is left alone.
    let active = Mutex::new(None);
    let during = hook(|phase, _| {
        if phase == RunPhase::Write {
            let report = cleanup_staging(root, &f.policy).unwrap();
            *active.lock().unwrap() = Some(report);
        }
    });
    let run = ExecutionContext::default().run_bundle(
        &f.recipe,
        &f.bindings,
        &f.policy,
        &f.destination(),
        &during,
    );
    assert_eq!(
        run.status, "succeeded_with_unresolved",
        "{:?}",
        run.diagnostics
    );
    let during = active.into_inner().unwrap().unwrap();
    assert!(
        during
            .skipped
            .iter()
            .any(|s| s.reason == "active" && !s.path.ends_with(format!(".iresiphi-{}", hex('d')))),
        "{:?}",
        during.skipped
    );
    assert!(during.removed.contains(&abandoned));
    assert_eq!(read_bundle(&f.destination()).unwrap()["review"].len(), 3);

    assert!(!abandoned.exists());
    assert_eq!(
        std::fs::read(precious.join("precious.txt")).unwrap(),
        b"mine"
    );
    assert_eq!(
        std::fs::read(elsewhere.path().join("table-0.csv")).unwrap(),
        b"mine"
    );
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(std::fs::read(&plain).unwrap(), b"mine");
    assert!(short.is_dir());

    // Cleanup is confined to approved roots.
    let outside = tempfile::tempdir().unwrap();
    assert_eq!(
        cleanup_staging(outside.path(), &f.policy).unwrap_err().code,
        "POLICY_DENIED"
    );
}

/// Process death before commit leaves no destination (only scratch that explicit
/// cleanup recognizes); death after commit leaves a complete, discoverable bundle.
#[test]
fn process_interruption_before_and_after_commit() {
    if let Ok(phase) = std::env::var("IRESIPHI_DIE_AT") {
        let f_root = std::env::var("IRESIPHI_DIE_ROOT").unwrap();
        let mut f = fixture();
        let root = Path::new(&f_root).to_owned();
        for (name, binding) in f.bindings.iter_mut() {
            let file = binding.path.file_name().unwrap().to_owned();
            std::fs::copy(&binding.path, root.join(&file)).unwrap();
            binding.path = root.join(file);
            let _ = name;
        }
        f.policy.allowed_read_roots = vec![root.clone()];
        f.policy.allowed_output_roots = vec![root.clone()];
        let target = if phase == "write" {
            RunPhase::Write
        } else {
            RunPhase::Committed
        };
        let die = hook(move |entered, _| {
            if entered == target {
                std::process::abort();
            }
        });
        ExecutionContext::default().run_bundle(
            &f.recipe,
            &f.bindings,
            &f.policy,
            &root.join("bundle"),
            &die,
        );
        unreachable!("the child process dies inside the run");
    }
    // The parent's retries start engine workers in this process; serialize them
    // with the tests that count live workers.
    let _serial = serial();
    let f = fixture();
    for phase in ["write", "commit"] {
        let root = f.root().join(phase);
        std::fs::create_dir(&root).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "process_interruption_before_and_after_commit",
                "--exact",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("IRESIPHI_DIE_AT", phase)
            .env("IRESIPHI_DIE_ROOT", &root)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(!status.success(), "{phase}: child must die");
        let mut policy = f.policy.clone();
        policy.allowed_read_roots = vec![root.clone()];
        policy.allowed_output_roots = vec![root.clone()];
        let mut bindings = f.bindings.clone();
        for binding in bindings.values_mut() {
            binding.path = root.join(binding.path.file_name().unwrap());
        }
        let destination = root.join("bundle");
        let context = ExecutionContext::default();
        if phase == "write" {
            assert!(!destination.exists(), "nothing published before commit");
            assert_eq!(
                staging_entries(&root).len(),
                1,
                "process death leaves scratch"
            );
            let cleanup = cleanup_staging(&root, &policy).unwrap();
            assert_eq!(cleanup.removed.len(), 1, "{:?}", cleanup.skipped);
            assert!(staging_entries(&root).is_empty());
            let retry = context.run_bundle(&f.recipe, &bindings, &policy, &destination, &go());
            assert_eq!(
                retry.status, "succeeded_with_unresolved",
                "{:?}",
                retry.diagnostics
            );
        } else {
            let manifest = read_json(&destination.join("manifest.json"));
            assert_eq!(manifest["complete"], true);
            assert!(manifest["run_id"].is_string());
            assert!(staging_entries(&root).is_empty());
            let before = snapshot(&destination);
            let retry = context.run_bundle(&f.recipe, &bindings, &policy, &destination, &go());
            assert_failed_without_results(&retry, "failed", "DESTINATION_EXISTS");
            assert_eq!(snapshot(&destination), before);
        }
    }
}
