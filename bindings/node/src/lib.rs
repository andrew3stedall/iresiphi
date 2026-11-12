//! Text-only transport delegates all recipe semantics to the core.
use iresiphi::{Bindings, CancellationToken, ExecutionContext, Policy, Recipe};
use napi::{Env, JsString, Task, bindgen_prelude::AsyncTask};
use napi_derive::napi;
use serde_json::{Value, json};

fn envelope(result: Result<Value, iresiphi::Error>) -> String {
    match result {
        Ok(value) => json!({"value": value}),
        Err(error) => json!({"error": error.completed()}),
    }
    .to_string()
}
fn parse<T: serde::de::DeserializeOwned>(
    text: &str,
    code: &str,
    phase: &str,
) -> Result<T, iresiphi::Error> {
    serde_json::from_str(text).map_err(|e| iresiphi::Error::new(code, phase, e.to_string()))
}
#[napi]
pub fn inspect_recipe(
    text: String,
    operation: String,
    bindings: Option<String>,
    policy: Option<String>,
    data_checks: Option<String>,
) -> String {
    envelope((|| {
        let recipe = Recipe::from_json(&text)?;
        let bound = match (bindings, policy) {
            (None, None) => None,
            (Some(b), Some(p)) => Some((
                parse::<Bindings>(&b, "INVALID_BINDING", "binding")?,
                parse::<Policy>(&p, "INVALID_POLICY", "policy")?,
            )),
            _ => {
                return Err(iresiphi::Error::new(
                    "INVALID_ARGUMENT",
                    "node",
                    "bindings and policy must be supplied together",
                ));
            }
        };
        Ok(match operation.as_str() {
            "load" => {
                json!({"json": recipe.to_json()?, "canonical": recipe.canonical_json()?, "hash": recipe.hash()?})
            }
            "validate" => match bound {
                Some((b, p)) => json!(iresiphi::validation::validate_bound(
                    &recipe,
                    &b,
                    &p,
                    data_checks.as_deref().unwrap_or("none")
                )),
                None => {
                    if data_checks.as_deref().unwrap_or("none") != "none" {
                        return Err(iresiphi::Error::new(
                            "MISSING_BINDING",
                            "validation",
                            "data checks require bindings and policy",
                        ));
                    }
                    json!(iresiphi::validate(&recipe))
                }
            },
            "explain" => match bound {
                Some((b, p)) => json!(iresiphi::validation::explain_bound(&recipe, &b, &p)),
                None => json!(iresiphi::explain(&recipe)),
            },
            _ => {
                return Err(iresiphi::Error::new(
                    "INVALID_ARGUMENT",
                    "node",
                    "unknown operation",
                ));
            }
        })
    })())
}
#[napi]
pub struct CancelToken {
    inner: CancellationToken,
}
#[napi]
impl CancelToken {
    #[napi(constructor)]
    pub fn new() -> Self {
        Self {
            inner: CancellationToken::new(),
        }
    }
    #[napi]
    pub fn cancel(&self) {
        self.inner.cancel();
    }
}
pub struct RunTask {
    recipe: Recipe,
    bindings: Bindings,
    policy: Policy,
    destination: Option<String>,
    cancel: CancellationToken,
}
impl Task for RunTask {
    type Output = String;
    type JsValue = JsString;
    fn compute(&mut self) -> napi::Result<String> {
        let context = ExecutionContext::default();
        let report = match &self.destination {
            Some(path) => context.run_bundle(
                &self.recipe,
                &self.bindings,
                &self.policy,
                std::path::Path::new(path),
                &self.cancel,
            ),
            None => context.run(&self.recipe, &self.bindings, &self.policy, &self.cancel),
        };
        Ok(serde_json::to_string(&report).map_err(|e| napi::Error::from_reason(e.to_string()))?)
    }
    fn resolve(&mut self, env: Env, output: String) -> napi::Result<JsString> {
        env.create_string(&output)
    }
}
#[napi]
pub fn execute(
    text: String,
    bindings: String,
    policy: String,
    destination: Option<String>,
    cancel: &CancelToken,
) -> napi::Result<AsyncTask<RunTask>> {
    let prepare = || -> Result<RunTask, iresiphi::Error> {
        Ok(RunTask {
            recipe: Recipe::from_json(&text)?,
            bindings: parse(&bindings, "INVALID_BINDING", "binding")?,
            policy: parse(&policy, "INVALID_POLICY", "policy")?,
            destination,
            cancel: cancel.inner.clone(),
        })
    };
    prepare()
        .map(AsyncTask::new)
        .map_err(|e| napi::Error::from_reason(json!({"error": e.completed()}).to_string()))
}
