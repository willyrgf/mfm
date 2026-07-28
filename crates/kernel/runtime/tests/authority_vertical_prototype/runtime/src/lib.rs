#![forbid(unsafe_code)]
//! Runtime-owned authority conversion for the vertical prototype.
//!
//! Persisted request and observation wrappers stay runtime-private:
//!
//! ```compile_fail
//! use mfm_runtime_authority_prototype::{
//!     CommittedObservation,
//!     CommittedRequest,
//! };
//! ```
//!
//! Store-issued append authority cannot be cloned:
//!
//! ```compile_fail
//! use mfm_store_authority_prototype::{
//!     AuthorizationAppend,
//!     AuthorizationAppendOutcome,
//!     CommitAcknowledgement,
//!     PrototypeStore,
//! };
//!
//! let mut store = PrototypeStore::new();
//! let outcome = store.append_authorization(
//!     AuthorizationAppend::new("append", "candidate"),
//!     CommitAcknowledgement::DirectlyObserved,
//! );
//! let AuthorizationAppendOutcome::NewlyAppended { authorization, .. } = outcome else {
//!     panic!("new append");
//! };
//! let _duplicate = authorization.clone();
//! ```
//!
//! Consuming the store witness twice is rejected by move checking:
//!
//! ```compile_fail
//! use mfm_program_authority_prototype::RequestView;
//! use mfm_runtime_authority_prototype::authorize_access;
//! use mfm_store_authority_prototype::{
//!     AuthorizationAppend,
//!     AuthorizationAppendOutcome,
//!     CommitAcknowledgement,
//!     PrototypeStore,
//! };
//!
//! let mut store = PrototypeStore::new();
//! let outcome = store.append_authorization(
//!     AuthorizationAppend::new("append", "candidate"),
//!     CommitAcknowledgement::DirectlyObserved,
//! );
//! let AuthorizationAppendOutcome::NewlyAppended { authorization, .. } = outcome else {
//!     panic!("new append");
//! };
//! let request = RequestView::new("request.v1", b"value");
//! let _first = authorize_access(authorization, request);
//! let _second = authorize_access(authorization, request);
//! ```
//!
//! An `AuthorizedAccess` cannot be forged or cloned:
//!
//! ```compile_fail
//! use mfm_runtime_authority_prototype::AuthorizedAccess;
//!
//! let _forged = AuthorizedAccess { request: () };
//! ```
//!
//! ```compile_fail
//! use mfm_program_authority_prototype::RequestView;
//! use mfm_runtime_authority_prototype::authorize_access;
//! use mfm_store_authority_prototype::{
//!     AuthorizationAppend,
//!     AuthorizationAppendOutcome,
//!     CommitAcknowledgement,
//!     PrototypeStore,
//! };
//!
//! let mut store = PrototypeStore::new();
//! let outcome = store.append_authorization(
//!     AuthorizationAppend::new("append", "candidate"),
//!     CommitAcknowledgement::DirectlyObserved,
//! );
//! let AuthorizationAppendOutcome::NewlyAppended { authorization, .. } = outcome else {
//!     panic!("new append");
//! };
//! let access = authorize_access(authorization, RequestView::new("request.v1", b"value"));
//! let _duplicate = access.clone();
//! ```
//!
//! Consuming access twice is rejected by move checking:
//!
//! ```compile_fail
//! use mfm_program_authority_prototype::RequestView;
//! use mfm_runtime_authority_prototype::{authorize_access, AuthorizedAccess};
//! use mfm_store_authority_prototype::{
//!     AuthorizationAppend,
//!     AuthorizationAppendOutcome,
//!     CommitAcknowledgement,
//!     PrototypeStore,
//! };
//!
//! fn accept(_: AuthorizedAccess<'_>) {}
//!
//! let mut store = PrototypeStore::new();
//! let outcome = store.append_authorization(
//!     AuthorizationAppend::new("append", "candidate"),
//!     CommitAcknowledgement::DirectlyObserved,
//! );
//! let AuthorizationAppendOutcome::NewlyAppended { authorization, .. } = outcome else {
//!     panic!("new append");
//! };
//! let access = authorize_access(authorization, RequestView::new("request.v1", b"value"));
//! accept(access);
//! accept(access);
//! ```
//!
//! `ExistingSame`, `OutcomeUnknown`, and reload expose no minting input:
//!
//! ```compile_fail
//! use mfm_program_authority_prototype::RequestView;
//! use mfm_runtime_authority_prototype::authorize_access;
//! use mfm_store_authority_prototype::{
//!     AuthorizationAppend,
//!     AuthorizationAppendOutcome,
//!     CommitAcknowledgement,
//!     PrototypeStore,
//! };
//!
//! let mut store = PrototypeStore::new();
//! let append = AuthorizationAppend::new("append", "candidate");
//! let _ = store.append_authorization(append, CommitAcknowledgement::DirectlyObserved);
//! let outcome = store.append_authorization(append, CommitAcknowledgement::DirectlyObserved);
//! let AuthorizationAppendOutcome::ExistingSame { committed } = outcome else {
//!     panic!("existing append");
//! };
//! let _ = authorize_access(committed, RequestView::new("request.v1", b"value"));
//! ```
//!
//! ```compile_fail
//! use mfm_program_authority_prototype::RequestView;
//! use mfm_runtime_authority_prototype::authorize_access;
//! use mfm_store_authority_prototype::{
//!     AuthorizationAppend,
//!     CommitAcknowledgement,
//!     PrototypeStore,
//! };
//!
//! let mut store = PrototypeStore::new();
//! let outcome = store.append_authorization(
//!     AuthorizationAppend::new("append", "candidate"),
//!     CommitAcknowledgement::LostAfterCommit,
//! );
//! let _ = authorize_access(outcome, RequestView::new("request.v1", b"value"));
//! ```
//!
//! ```compile_fail
//! use mfm_program_authority_prototype::RequestView;
//! use mfm_runtime_authority_prototype::authorize_access;
//! use mfm_store_authority_prototype::{
//!     AuthorizationAppend,
//!     CommitAcknowledgement,
//!     PrototypeStore,
//! };
//!
//! let mut store = PrototypeStore::new();
//! let _ = store.append_authorization(
//!     AuthorizationAppend::new("append", "candidate"),
//!     CommitAcknowledgement::LostAfterCommit,
//! );
//! let reloaded = store.reload_authorization("append").expect("committed");
//! let _ = authorize_access(reloaded, RequestView::new("request.v1", b"value"));
//! ```

use mfm_program_authority_prototype::{ObservationView, RequestView};
use mfm_store_authority_prototype::{
    AuthorizationRef, CommittedAuthorization, NewlyAppendedAuthorization,
};

#[derive(Debug)]
struct CommittedRequest<'a> {
    authorization: CommittedAuthorization,
    view: RequestView<'a>,
}

#[derive(Debug)]
struct CommittedObservation<'a> {
    authorization_ref: AuthorizationRef,
    view: ObservationView<'a>,
}

/// Affine authority accepted by one live adapter invocation.
#[must_use]
#[derive(Debug)]
pub struct AuthorizedAccess<'a> {
    request: CommittedRequest<'a>,
}

impl<'a> AuthorizedAccess<'a> {
    /// Borrows the program-facing request value without exposing runtime metadata.
    pub const fn request_view(&self) -> RequestView<'a> {
        self.request.view
    }

    fn authorization_ref(&self) -> AuthorizationRef {
        self.request.authorization.authorization_ref()
    }
}

/// Consumes a directly observed store append witness into affine live access.
pub fn authorize_access<'a>(
    authorization: NewlyAppendedAuthorization,
    request: RequestView<'a>,
) -> AuthorizedAccess<'a> {
    AuthorizedAccess {
        request: CommittedRequest {
            authorization: authorization.into_committed(),
            view: request,
        },
    }
}

/// Live adapters accept and consume one affine access authority.
pub trait LiveAdapter {
    /// Performs at most one boundary operation and returns a borrowed value view.
    fn accept<'response>(
        &'response self,
        access: AuthorizedAccess<'_>,
    ) -> ObservationView<'response>;
}

/// Non-authorizing receipt proving the private observation wrapper was formed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservationReceipt {
    authorization_ref: AuthorizationRef,
    schema_len: usize,
    value_len: usize,
}

impl ObservationReceipt {
    /// Returns the authorization paired with the observation.
    pub const fn authorization_ref(self) -> AuthorizationRef {
        self.authorization_ref
    }

    /// Returns the observed schema length.
    pub const fn schema_len(self) -> usize {
        self.schema_len
    }

    /// Returns the observed value length.
    pub const fn value_len(self) -> usize {
        self.value_len
    }
}

/// Runs one live adapter and privately binds its observation to the authorization.
pub fn commit_live_observation(
    adapter: &impl LiveAdapter,
    access: AuthorizedAccess<'_>,
) -> ObservationReceipt {
    let authorization_ref = access.authorization_ref();
    let view = adapter.accept(access);
    let committed = CommittedObservation {
        authorization_ref,
        view,
    };
    ObservationReceipt {
        authorization_ref: committed.authorization_ref,
        schema_len: committed.view.schema().len(),
        value_len: committed.view.value().len(),
    }
}
