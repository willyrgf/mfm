use std::{marker::PhantomData, sync::Arc};

use mfm_ids::ContentRef;
use mfm_values::MfmValue;

use crate::{ProgramError, Result};

/// A typed authoring checkpoint, privately constructed at a checked input boundary.
///
/// Tokens cannot be deserialized or constructed from declaration indices. A child may inherit
/// an installed parent binding, but cannot install a captured parent token itself.
pub struct Checkpoint<T> {
    pub(crate) boundary: ScopedBoundary,
    marker: PhantomData<fn() -> T>,
}

/// Allocation identity prevents a token from a prior expansion aliasing a new scope.
#[derive(Debug, Clone)]
pub(crate) struct ScopeId(Arc<()>);

impl ScopeId {
    pub(crate) fn new() -> Self {
        Self(Arc::new(()))
    }
}

impl PartialEq for ScopeId {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for ScopeId {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScopedBoundary {
    owner: ScopeId,
    pub(crate) offset: usize,
    pub(crate) input: ContentRef,
}

impl ScopedBoundary {
    pub(crate) fn belongs_to(&self, owner: &ScopeId) -> bool {
        &self.owner == owner
    }
    pub(crate) fn require_owner(&self, owner: &ScopeId) -> Result<()> {
        if &self.owner != owner {
            return Err(ProgramError::InvalidContract);
        }
        Ok(())
    }
}

impl<T: MfmValue> Checkpoint<T> {
    pub(crate) fn at(owner: ScopeId, offset: usize, input: ContentRef) -> Self {
        Self {
            boundary: ScopedBoundary {
                owner,
                offset,
                input,
            },
            marker: PhantomData,
        }
    }
}
