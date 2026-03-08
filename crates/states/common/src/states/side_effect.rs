//! Shared failpoint helpers for proof recovery tests.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Test-only trigger that arms a single post-handler failpoint.
#[derive(Clone)]
pub struct TriggerOnce {
    stop_after_handler_once: Arc<AtomicBool>,
    armed_once: Arc<AtomicBool>,
}

impl TriggerOnce {
    /// Arms a one-shot trigger backed by the supplied atomic failpoint flag.
    pub fn arm(stop_after_handler_once: Arc<AtomicBool>) -> Self {
        Self {
            stop_after_handler_once,
            armed_once: Arc::new(AtomicBool::new(true)),
        }
    }

    pub(crate) fn trigger_if_armed(&self) {
        if self.armed_once.swap(false, Ordering::SeqCst) {
            self.stop_after_handler_once.store(true, Ordering::SeqCst);
        }
    }
}
