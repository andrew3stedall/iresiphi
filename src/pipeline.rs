//! Deferred execution. A pipeline owns declarations, never source observations.
use crate::{Bindings, Cancellation, ExecutionContext, Policy, Recipe, RunReport};

/// A reusable recipe bound to an execution policy and source locations.
///
/// Construction and explanation never open sources. Each explicit execution
/// reads the then-current sources; this is a plan, not a cached snapshot.
/// `collect` deliberately retains results. It does not promise bounded RSS.
#[derive(Debug, Clone)]
pub struct Pipeline {
    recipe: Recipe,
    bindings: Bindings,
    policy: Policy,
}
impl Pipeline {
    pub fn new(recipe: Recipe, bindings: Bindings, policy: Policy) -> Self {
        Self {
            recipe,
            bindings,
            policy,
        }
    }
    pub fn explain(&self) -> serde_json::Value {
        crate::validation::explain_bound(&self.recipe, &self.bindings, &self.policy)
    }
    pub fn collect<C: Cancellation + ?Sized>(
        &self,
        context: &ExecutionContext,
        cancel: &C,
    ) -> RunReport {
        context.run(&self.recipe, &self.bindings, &self.policy, cancel)
    }
    pub fn run_bundle<C: Cancellation + ?Sized>(
        &self,
        context: &ExecutionContext,
        destination: &std::path::Path,
        cancel: &C,
    ) -> RunReport {
        context.run_bundle(
            &self.recipe,
            &self.bindings,
            &self.policy,
            destination,
            cancel,
        )
    }
}
