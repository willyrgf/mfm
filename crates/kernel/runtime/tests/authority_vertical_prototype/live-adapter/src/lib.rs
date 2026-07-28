#![forbid(unsafe_code)]
//! Live-adapter endpoint for the vertical authority-flow prototype.

use std::cell::Cell;

use mfm_program_authority_prototype::ObservationView;
use mfm_runtime_authority_prototype::{AuthorizedAccess, LiveAdapter};

/// A deterministic adapter that records acceptance of one borrowed request.
#[derive(Debug)]
pub struct PrototypeLiveAdapter {
    response: Vec<u8>,
    last_request_len: Cell<Option<usize>>,
}

impl PrototypeLiveAdapter {
    /// Creates an adapter returning the supplied response bytes.
    pub fn new(response: impl Into<Vec<u8>>) -> Self {
        Self {
            response: response.into(),
            last_request_len: Cell::new(None),
        }
    }

    /// Returns the length of the last accepted request value.
    pub fn last_request_len(&self) -> Option<usize> {
        self.last_request_len.get()
    }
}

impl LiveAdapter for PrototypeLiveAdapter {
    fn accept<'response>(
        &'response self,
        access: AuthorizedAccess<'_>,
    ) -> ObservationView<'response> {
        let request = access.request_view();
        self.last_request_len.set(Some(request.value().len()));
        ObservationView::new("prototype.observation.v1", &self.response)
    }
}
