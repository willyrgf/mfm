#![forbid(unsafe_code)]
//! Store-owned append authority for the vertical authority-flow prototype.
//!
//! The append witness cannot be forged because its fields are store-private:
//!
//! ```compile_fail
//! use mfm_store_authority_prototype::NewlyAppendedAuthorization;
//!
//! let _forged = NewlyAppendedAuthorization {};
//! ```

use std::collections::BTreeMap;

/// Identifies one committed external-access authorization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthorizationRef(u64);

/// Identifies the containing journal commit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommitCoordinate(u64);

/// Non-authorizing information about a committed authorization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommittedAuthorization {
    authorization_ref: AuthorizationRef,
    coordinate: CommitCoordinate,
}

impl CommittedAuthorization {
    /// Returns the authorization reference.
    pub const fn authorization_ref(self) -> AuthorizationRef {
        self.authorization_ref
    }

    /// Returns the commit coordinate.
    pub const fn coordinate(self) -> CommitCoordinate {
        self.coordinate
    }
}

/// One prepared authorization append.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthorizationAppend<'a> {
    append_request_id: &'a str,
    candidate_digest: &'a str,
}

impl<'a> AuthorizationAppend<'a> {
    /// Creates a prepared append from its idempotency identity and candidate digest.
    pub const fn new(append_request_id: &'a str, candidate_digest: &'a str) -> Self {
        Self {
            append_request_id,
            candidate_digest,
        }
    }
}

/// Controls whether a successful commit acknowledgement reaches the caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitAcknowledgement {
    /// The caller directly observes the successful new append.
    DirectlyObserved,
    /// The commit succeeds, but its acknowledgement is lost.
    LostAfterCommit,
}

/// Closed result of an authorization append.
#[must_use]
#[derive(Debug)]
pub enum AuthorizationAppendOutcome {
    /// A directly observed new append carries the one affine witness.
    NewlyAppended {
        /// Public information about the committed authorization.
        committed: CommittedAuthorization,
        /// Store-issued authority available only on this outcome.
        authorization: NewlyAppendedAuthorization,
    },
    /// An exact append identity already committed and carries no authority.
    ExistingSame {
        /// Public information about the pre-existing authorization.
        committed: CommittedAuthorization,
    },
    /// The append identity conflicts with different candidate content.
    Conflict {
        /// Public information about the conflicting committed authorization.
        committed: CommittedAuthorization,
    },
    /// Commit success is unknown to the caller and carries no authority.
    OutcomeUnknown,
    /// The store cannot assign another commit coordinate.
    CoordinateOverflow,
}

/// Affine proof that this caller directly observed a new authorization append.
///
/// This type is intentionally neither `Clone` nor serializable. Only
/// [`PrototypeStore::append_authorization`] can construct it.
#[must_use]
#[derive(Debug)]
pub struct NewlyAppendedAuthorization {
    committed: CommittedAuthorization,
}

impl NewlyAppendedAuthorization {
    /// Returns the committed authorization reference without duplicating authority.
    pub const fn authorization_ref(&self) -> AuthorizationRef {
        self.committed.authorization_ref
    }

    /// Consumes the witness and returns its non-authorizing committed metadata.
    pub fn into_committed(self) -> CommittedAuthorization {
        self.committed
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StoredAuthorization {
    candidate_digest: String,
    committed: CommittedAuthorization,
}

/// Minimal append-only store used only by this compile contract.
#[derive(Debug, Default)]
pub struct PrototypeStore {
    next_coordinate: u64,
    authorizations: BTreeMap<String, StoredAuthorization>,
}

impl PrototypeStore {
    /// Creates an empty prototype store.
    pub const fn new() -> Self {
        Self {
            next_coordinate: 0,
            authorizations: BTreeMap::new(),
        }
    }

    /// Appends an authorization and mints authority only for an observed new commit.
    pub fn append_authorization(
        &mut self,
        append: AuthorizationAppend<'_>,
        acknowledgement: CommitAcknowledgement,
    ) -> AuthorizationAppendOutcome {
        if let Some(existing) = self.authorizations.get(append.append_request_id) {
            return if existing.candidate_digest == append.candidate_digest {
                AuthorizationAppendOutcome::ExistingSame {
                    committed: existing.committed,
                }
            } else {
                AuthorizationAppendOutcome::Conflict {
                    committed: existing.committed,
                }
            };
        }

        let Some(next_coordinate) = self.next_coordinate.checked_add(1) else {
            return AuthorizationAppendOutcome::CoordinateOverflow;
        };
        self.next_coordinate = next_coordinate;
        let committed = CommittedAuthorization {
            authorization_ref: AuthorizationRef(self.next_coordinate),
            coordinate: CommitCoordinate(self.next_coordinate),
        };
        self.authorizations.insert(
            append.append_request_id.to_owned(),
            StoredAuthorization {
                candidate_digest: append.candidate_digest.to_owned(),
                committed,
            },
        );

        match acknowledgement {
            CommitAcknowledgement::DirectlyObserved => AuthorizationAppendOutcome::NewlyAppended {
                committed,
                authorization: NewlyAppendedAuthorization { committed },
            },
            CommitAcknowledgement::LostAfterCommit => AuthorizationAppendOutcome::OutcomeUnknown,
        }
    }

    /// Reloads committed metadata without recreating append authority.
    pub fn reload_authorization(&self, append_request_id: &str) -> Option<CommittedAuthorization> {
        self.authorizations
            .get(append_request_id)
            .map(|stored| stored.committed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_directly_observed_new_append_contains_authority() {
        let mut store = PrototypeStore::new();
        let append = AuthorizationAppend::new("append-a", "candidate-a");

        assert!(matches!(
            store.append_authorization(append, CommitAcknowledgement::LostAfterCommit),
            AuthorizationAppendOutcome::OutcomeUnknown
        ));
        assert!(store.reload_authorization("append-a").is_some());
        assert!(matches!(
            store.append_authorization(append, CommitAcknowledgement::DirectlyObserved),
            AuthorizationAppendOutcome::ExistingSame { .. }
        ));

        assert!(matches!(
            store.append_authorization(
                AuthorizationAppend::new("append-b", "candidate-b"),
                CommitAcknowledgement::DirectlyObserved,
            ),
            AuthorizationAppendOutcome::NewlyAppended { .. }
        ));
    }
}
