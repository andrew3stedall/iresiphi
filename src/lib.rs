//! Source-preserving local recipes.
mod admission;
pub mod cancel;
pub mod eval;
pub mod model;
pub mod scalar;
pub mod validation;
pub use model::{Binding, Bindings, Error, Limits, Policy, Recipe};
pub use validation::{explain, validate};
pub mod engine;
pub use cancel::{Cancellation, CancellationToken, RunPhase};
pub use engine::{ExecutionContext, RunReport};
pub mod sink;

pub mod adapters;

#[cfg(feature = "python")]
mod python;
