//! Semantic open consumes the complete certification-owned registry; a caller
//! cannot substitute the store's callback-free verification snapshot.
use std::sync::Arc;

use mfm_store::structured::{
    qualify_and_open_structured_store, PhysicalObligationChecker, ProgramVerificationRegistry,
    StructuredHistoryBackend,
};

async fn substitute_program_trust<B: StructuredHistoryBackend>(
    backend: B,
    programs: ProgramVerificationRegistry,
    physical: Arc<dyn PhysicalObligationChecker>,
) {
    let _ = qualify_and_open_structured_store(backend, programs, physical).await;
}

fn main() {}
