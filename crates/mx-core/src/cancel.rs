use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Cheap, cloneable cancellation flag. Hand one clone to a job and keep
/// another for a Ctrl+C handler or a UI "Stop" button.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Returned (inside `anyhow::Error`) when a job stops because its token was
/// cancelled. Check with `err.is::<Cancelled>()`.
#[derive(Debug, thiserror::Error)]
#[error("cancelled")]
pub struct Cancelled;
