//! Cooperative run cancellation (decision 13).
//!
//! One token per run is checked between parsing chunks, batches, engine polls and
//! output writes. Cancellation is a request: blocking adapter or operator work may
//! delay it, and no bounded latency is promised. Once a bundle is committed the
//! run cannot be revoked.
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, Ordering},
};

/// Observable points in a run where cooperative work begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunPhase {
    /// Source access and parsing is about to start.
    Parse,
    /// An engine join over admitted sources is about to execute.
    Join,
    /// Run-owned bundle staging exists and output files are about to be written.
    Write,
    /// The bundle has been atomically renamed into place; the run is committed.
    Committed,
}

/// A source of cancellation requests for one run.
///
/// The engine reads [`Cancellation::flag`] at every cooperative check and calls
/// [`Cancellation::entered`] as the run reaches each [`RunPhase`]. `AtomicBool`,
/// `Arc<AtomicBool>` and [`CancellationToken`] implement it.
pub trait Cancellation {
    /// The flag a run polls; `true` requests cancellation.
    fn flag(&self) -> &AtomicBool;
    /// Called on the run's thread as it enters `phase`. The default does nothing.
    fn entered(&self, _phase: RunPhase) {}
}
impl Cancellation for AtomicBool {
    fn flag(&self) -> &AtomicBool {
        self
    }
}
impl<T: Cancellation + ?Sized> Cancellation for Arc<T> {
    fn flag(&self) -> &AtomicBool {
        (**self).flag()
    }
    fn entered(&self, phase: RunPhase) {
        (**self).entered(phase)
    }
}

/// A shareable cancellation token. Clones refer to the same request.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    state: Arc<State>,
}
#[derive(Debug, Default)]
struct State {
    flag: AtomicBool,
    /// 0 = not armed; otherwise `1 + phase` to cancel when that phase is entered.
    armed: AtomicU8,
}
fn phase_code(phase: RunPhase) -> u8 {
    1 + match phase {
        RunPhase::Parse => 0,
        RunPhase::Join => 1,
        RunPhase::Write => 2,
        RunPhase::Committed => 3,
    }
}
impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }
    /// Request cancellation. Safe to call from any thread, any number of times.
    pub fn cancel(&self) {
        self.state.flag.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.state.flag.load(Ordering::Acquire)
    }
    /// Request cancellation as soon as a run using this token enters `phase`.
    /// Deterministic phase-level cancellation, for tests and staged shutdown.
    pub fn cancel_at(&self, phase: RunPhase) {
        self.state.armed.store(phase_code(phase), Ordering::Release);
    }
}
impl Cancellation for CancellationToken {
    fn flag(&self) -> &AtomicBool {
        &self.state.flag
    }
    fn entered(&self, phase: RunPhase) {
        if self.state.armed.load(Ordering::Acquire) == phase_code(phase) {
            self.cancel();
        }
    }
}
