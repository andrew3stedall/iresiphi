//! Owned Python handles; workers only operate on Rust-owned data.
use crate::{Bindings, ExecutionContext, Policy, Recipe, RunReport};
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
fn invalid(error: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(error.to_string())
}
fn closed() -> PyErr {
    PyRuntimeError::new_err("HANDLE_CLOSED")
}
#[pyclass(name = "Recipe", module = "iresiphi._native")]
struct PyRecipe {
    inner: Option<Recipe>,
}
#[pymethods]
impl PyRecipe {
    #[new]
    fn new(json: &str) -> PyResult<Self> {
        Ok(Self {
            inner: Some(Recipe::from_json(json).map_err(invalid)?),
        })
    }
    fn to_json(&self) -> PyResult<String> {
        self.inner
            .as_ref()
            .ok_or_else(closed)?
            .to_json()
            .map_err(invalid)
    }
    fn validate(&self) -> PyResult<String> {
        serde_json::to_string(&crate::validate(self.inner.as_ref().ok_or_else(closed)?))
            .map_err(invalid)
    }
    fn validate_bound(&self, bindings: &str, policy: &str, data_checks: &str) -> PyResult<String> {
        let bindings: Bindings = serde_json::from_str(bindings).map_err(invalid)?;
        let policy: Policy = serde_json::from_str(policy).map_err(invalid)?;
        serde_json::to_string(&crate::validation::validate_bound(
            self.inner.as_ref().ok_or_else(closed)?,
            &bindings,
            &policy,
            data_checks,
        ))
        .map_err(invalid)
    }
    fn explain_bound(&self, bindings: &str, policy: &str) -> PyResult<String> {
        let bindings: Bindings = serde_json::from_str(bindings).map_err(invalid)?;
        let policy: Policy = serde_json::from_str(policy).map_err(invalid)?;
        serde_json::to_string(&crate::validation::explain_bound(
            self.inner.as_ref().ok_or_else(closed)?,
            &bindings,
            &policy,
        ))
        .map_err(invalid)
    }
    fn explain(&self) -> PyResult<String> {
        serde_json::to_string(&crate::explain(self.inner.as_ref().ok_or_else(closed)?))
            .map_err(invalid)
    }
    fn close(&mut self) {
        self.inner = None;
    }
}
#[pyclass(name = "CancellationToken", module = "iresiphi._native")]
#[derive(Default)]
struct PyCancellation {
    inner: Arc<AtomicBool>,
}
#[pymethods]
impl PyCancellation {
    #[new]
    fn new() -> Self {
        Self::default()
    }
    fn cancel(&self) {
        self.inner.store(true, Ordering::Release);
    }
}
#[pyclass(name = "Result", module = "iresiphi._native")]
struct PyResultHandle {
    inner: Option<RunReport>,
    batch_rows: usize,
    batch_bytes: usize,
}
#[pymethods]
impl PyResultHandle {
    fn close(&mut self) {
        self.inner = None;
    }
    #[getter]
    fn status(&self) -> PyResult<String> {
        Ok(self.inner.as_ref().ok_or_else(closed)?.status.clone())
    }
    fn metadata(&self) -> PyResult<String> {
        let report = self.inner.as_ref().ok_or_else(closed)?;
        serde_json::to_string(&serde_json::json!({"run_id":report.run_id,"status":report.status,"manifest":report.manifest,"diagnostics":report.diagnostics,"tables":report.tables.keys().collect::<Vec<_>>()})).map_err(invalid)
    }
    #[pyo3(signature=(table, offset=0, count=8192))]
    fn batch(&self, table: &str, offset: usize, count: usize) -> PyResult<String> {
        if count > self.batch_rows {
            return Err(invalid(
                "RESOURCE_LIMIT: requested batch exceeds row budget",
            ));
        }
        let report = self.inner.as_ref().ok_or_else(closed)?;
        let rows = report
            .tables
            .get(table)
            .ok_or_else(|| invalid("UNKNOWN_OUTPUT"))?;
        let end = offset.saturating_add(count).min(rows.len());
        let slice = if offset >= rows.len() {
            &rows[0..0]
        } else {
            &rows[offset..end]
        };
        let json = serde_json::to_string(slice).map_err(invalid)?;
        if json.len() > self.batch_bytes {
            return Err(invalid(
                "RESOURCE_LIMIT: requested batch exceeds byte budget",
            ));
        }
        Ok(json)
    }
}
#[pyclass(name = "ExecutionContext", module = "iresiphi._native")]
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
    #[pyo3(signature=(recipe,bindings,policy,cancel=None,destination=None))]
    fn run(
        &self,
        py: Python<'_>,
        recipe: &PyRecipe,
        bindings: &str,
        policy: &str,
        cancel: Option<&PyCancellation>,
        destination: Option<String>,
    ) -> PyResult<PyResultHandle> {
        let recipe = recipe.inner.as_ref().ok_or_else(closed)?.clone();
        let bindings: Bindings = serde_json::from_str(bindings).map_err(invalid)?;
        let policy: Policy = serde_json::from_str(policy).map_err(invalid)?;
        let batch_rows = policy.limits.batch_rows.min(8192);
        let batch_bytes = policy.limits.batch_bytes;
        let context = self.inner.clone();
        let token = cancel.map(|c| c.inner.clone()).unwrap_or_default();
        let worker_token = token.clone();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match destination {
                    Some(path) => context.run_bundle(
                        &recipe,
                        &bindings,
                        &policy,
                        std::path::Path::new(&path),
                        &worker_token,
                    ),
                    None => context.run(&recipe, &bindings, &policy, &worker_token),
                }));
            let _ = sender.send(result);
        });
        let mut signal = None;
        let result = loop {
            match py.detach(|| receiver.recv_timeout(Duration::from_millis(50))) {
                Ok(result) => break result,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = py.detach(|| worker.join());
                    return Err(PyRuntimeError::new_err("WORKER_FAILED"));
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
            if signal.is_none() {
                if let Err(error) = py.check_signals() {
                    token.store(true, Ordering::Release);
                    signal = Some(error);
                }
            }
        };
        py.detach(|| worker.join())
            .map_err(|_| PyRuntimeError::new_err("WORKER_FAILED"))?;
        if let Some(error) = signal {
            return Err(error);
        }
        Ok(PyResultHandle {
            inner: Some(result.map_err(|_| PyRuntimeError::new_err("WORKER_FAILED"))?),
            batch_rows,
            batch_bytes,
        })
    }
}
#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyRecipe>()?;
    module.add_class::<PyCancellation>()?;
    module.add_class::<PyContext>()?;
    module.add_class::<PyResultHandle>()?;
    Ok(())
}
