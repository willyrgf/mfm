use std::collections::{btree_map::Entry, BTreeMap};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct StoreScopeId(&'static str);

impl StoreScopeId {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TenantScopeId(&'static str);

impl TenantScopeId {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct EntryPointOperationId(&'static str);

impl EntryPointOperationId {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct InvocationIdentity(&'static str);

impl InvocationIdentity {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AdmissionKey {
    tenant_scope_id: TenantScopeId,
    entry_point_operation_id: EntryPointOperationId,
    invocation_identity: InvocationIdentity,
}

impl AdmissionKey {
    pub const fn new(
        tenant_scope_id: TenantScopeId,
        entry_point_operation_id: EntryPointOperationId,
        invocation_identity: InvocationIdentity,
    ) -> Self {
        Self {
            tenant_scope_id,
            entry_point_operation_id,
            invocation_identity,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RunId {
    store_scope_id: StoreScopeId,
    admission_key: AdmissionKey,
}

impl RunId {
    pub const fn derive(store_scope_id: StoreScopeId, admission_key: AdmissionKey) -> Self {
        Self {
            store_scope_id,
            admission_key,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmissionRoot {
    key: AdmissionKey,
    operation_contract_ref: &'static str,
    config_manifest_ref: &'static str,
    genesis_digest: &'static str,
}

impl AdmissionRoot {
    pub const fn new(
        key: AdmissionKey,
        operation_contract_ref: &'static str,
        config_manifest_ref: &'static str,
        genesis_digest: &'static str,
    ) -> Self {
        Self {
            key,
            operation_contract_ref,
            config_manifest_ref,
            genesis_digest,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommitDigest {
    run_sequence: u64,
    candidate_digest: CandidateDigest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JournalHead {
    run_sequence: u64,
    commit_digest: CommitDigest,
}

impl JournalHead {
    pub const fn run_sequence(self) -> u64 {
        self.run_sequence
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AppendRequestId(&'static str);

impl AppendRequestId {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CandidateDigest(&'static str);

impl CandidateDigest {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AuthorizationId(&'static str);

impl AuthorizationId {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitAcknowledgement {
    DirectlyObserved,
    LostAfterCommit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionOutcome {
    NewlyAdmitted { run_id: RunId, head: JournalHead },
    AttachedToExisting { run_id: RunId, head: JournalHead },
    Conflict { run_id: RunId },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JournalAppendOutcome {
    NewlyAppended { head: JournalHead },
    AlreadyCommitted { head: JournalHead },
    OutcomeUnknown,
}

#[derive(Debug)]
pub enum AuthorizationAppendOutcome {
    NewlyAppended {
        head: JournalHead,
        authorization: NewlyAppendedAuthorization,
    },
    AlreadyCommitted {
        head: JournalHead,
    },
    OutcomeUnknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppendRejection {
    RunNotFound,
    HeadMismatch { current: JournalHead },
    AppendRequestConflict { committed: JournalHead },
    SequenceOverflow,
    RunClosed,
    AuthorizationAlreadyExists,
    UnknownAuthorization,
    ObservationAlreadyRecorded,
    AuthorizationNotBeforeClosure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Reconciliation {
    Committed { head: JournalHead },
    NotCommitted,
    Conflict { committed: JournalHead },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransitionAppend {
    run_id: RunId,
    expected_head: JournalHead,
    append_request_id: AppendRequestId,
    candidate_digest: CandidateDigest,
    closes_run: bool,
}

impl TransitionAppend {
    pub const fn new(
        run_id: RunId,
        expected_head: JournalHead,
        append_request_id: AppendRequestId,
        candidate_digest: CandidateDigest,
        closes_run: bool,
    ) -> Self {
        Self {
            run_id,
            expected_head,
            append_request_id,
            candidate_digest,
            closes_run,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthorizationAppend {
    run_id: RunId,
    expected_head: JournalHead,
    append_request_id: AppendRequestId,
    candidate_digest: CandidateDigest,
    authorization_id: AuthorizationId,
}

impl AuthorizationAppend {
    pub const fn new(
        run_id: RunId,
        expected_head: JournalHead,
        append_request_id: AppendRequestId,
        candidate_digest: CandidateDigest,
        authorization_id: AuthorizationId,
    ) -> Self {
        Self {
            run_id,
            expected_head,
            append_request_id,
            candidate_digest,
            authorization_id,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservationAppend {
    run_id: RunId,
    expected_head: JournalHead,
    append_request_id: AppendRequestId,
    candidate_digest: CandidateDigest,
    authorization_id: AuthorizationId,
}

impl ObservationAppend {
    pub const fn new(
        run_id: RunId,
        expected_head: JournalHead,
        append_request_id: AppendRequestId,
        candidate_digest: CandidateDigest,
        authorization_id: AuthorizationId,
    ) -> Self {
        Self {
            run_id,
            expected_head,
            append_request_id,
            candidate_digest,
            authorization_id,
        }
    }
}

#[must_use]
#[derive(Debug)]
pub struct NewlyAppendedAuthorization {
    run_id: RunId,
    authorization_id: AuthorizationId,
    committed_head: JournalHead,
}

impl NewlyAppendedAuthorization {
    pub const fn run_id(&self) -> RunId {
        self.run_id
    }

    pub const fn authorization_id(&self) -> AuthorizationId {
        self.authorization_id
    }

    pub const fn committed_head(&self) -> JournalHead {
        self.committed_head
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunAccessGrant {
    Drive,
    Replay,
    ReadPublic,
    InspectTrace,
    InspectAudit,
    Export,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RunAccessBinding {
    grant: RunAccessGrant,
    store_scope_id: StoreScopeId,
    tenant_scope_id: TenantScopeId,
    run_id: RunId,
}

#[must_use]
#[derive(Debug)]
pub struct RunAccessAuthority {
    binding: RunAccessBinding,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RunAccessAuthorityIssuer {
    store_scope_id: StoreScopeId,
    tenant_scope_id: TenantScopeId,
}

impl RunAccessAuthorityIssuer {
    pub const fn new(store_scope_id: StoreScopeId, tenant_scope_id: TenantScopeId) -> Self {
        Self {
            store_scope_id,
            tenant_scope_id,
        }
    }

    pub const fn issue(&self, grant: RunAccessGrant, run_id: RunId) -> RunAccessAuthority {
        RunAccessAuthority {
            binding: RunAccessBinding {
                grant,
                store_scope_id: self.store_scope_id,
                tenant_scope_id: self.tenant_scope_id,
                run_id,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunAccessRejection {
    WrongGrant,
    WrongStore,
    WrongTenant,
    WrongRun,
    RunNotFound,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CommittedAppend {
    predecessor: JournalHead,
    purpose: AppendPurpose,
    candidate_digest: CandidateDigest,
    head: JournalHead,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AppendPurpose {
    Transition,
    ExternalAccessAuthorization,
    ExternalAccessObservation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AppendCandidate {
    run_id: RunId,
    predecessor: JournalHead,
    append_request_id: AppendRequestId,
    purpose: AppendPurpose,
    candidate_digest: CandidateDigest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AuthorizationState {
    committed_head: JournalHead,
    observed: bool,
}

#[derive(Debug)]
struct RunJournal {
    root: AdmissionRoot,
    head: JournalHead,
    closure: Option<JournalHead>,
    appends: BTreeMap<AppendRequestId, CommittedAppend>,
    authorizations: BTreeMap<AuthorizationId, AuthorizationState>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InternalAppendOutcome {
    NewlyAppended { head: JournalHead },
    AlreadyCommitted { head: JournalHead },
}

#[derive(Debug)]
pub struct PrototypeStore {
    store_scope_id: StoreScopeId,
    admission_index: BTreeMap<AdmissionKey, RunId>,
    runs: BTreeMap<RunId, RunJournal>,
}

impl PrototypeStore {
    pub const fn new(store_scope_id: StoreScopeId) -> Self {
        Self {
            store_scope_id,
            admission_index: BTreeMap::new(),
            runs: BTreeMap::new(),
        }
    }

    pub fn admit_run(&mut self, root: AdmissionRoot) -> AdmissionOutcome {
        let run_id = RunId::derive(self.store_scope_id, root.key);
        if let Some(existing_run_id) = self.admission_index.get(&root.key).copied() {
            let existing = self
                .runs
                .get(&existing_run_id)
                .expect("admission index references an existing run");
            return if existing.root == root {
                AdmissionOutcome::AttachedToExisting {
                    run_id: existing_run_id,
                    head: existing.head,
                }
            } else {
                AdmissionOutcome::Conflict {
                    run_id: existing_run_id,
                }
            };
        }

        let head = JournalHead {
            run_sequence: 1,
            commit_digest: CommitDigest {
                run_sequence: 1,
                candidate_digest: CandidateDigest(root.genesis_digest),
            },
        };
        let previous_run = self.runs.insert(
            run_id,
            RunJournal {
                root,
                head,
                closure: None,
                appends: BTreeMap::new(),
                authorizations: BTreeMap::new(),
            },
        );
        assert!(
            previous_run.is_none(),
            "derived run id must be unique for an admission key"
        );
        let previous_key = self.admission_index.insert(root.key, run_id);
        assert!(
            previous_key.is_none(),
            "admission key uniqueness was checked before insertion"
        );

        AdmissionOutcome::NewlyAdmitted { run_id, head }
    }

    pub fn commit_transition(
        &mut self,
        append: TransitionAppend,
        acknowledgement: CommitAcknowledgement,
    ) -> Result<JournalAppendOutcome, AppendRejection> {
        let closes_run = append.closes_run;
        let outcome = self.append_candidate(
            AppendCandidate {
                run_id: append.run_id,
                predecessor: append.expected_head,
                append_request_id: append.append_request_id,
                purpose: AppendPurpose::Transition,
                candidate_digest: append.candidate_digest,
            },
            |journal| {
                if journal.closure.is_some() {
                    return Err(AppendRejection::RunClosed);
                }
                Ok(())
            },
            move |journal, head| {
                if closes_run {
                    journal.closure = Some(head);
                }
            },
        )?;
        Ok(surface_journal_outcome(outcome, acknowledgement))
    }

    pub fn authorize_external_access(
        &mut self,
        append: AuthorizationAppend,
        acknowledgement: CommitAcknowledgement,
    ) -> Result<AuthorizationAppendOutcome, AppendRejection> {
        let authorization_id = append.authorization_id;
        let outcome = self.append_candidate(
            AppendCandidate {
                run_id: append.run_id,
                predecessor: append.expected_head,
                append_request_id: append.append_request_id,
                purpose: AppendPurpose::ExternalAccessAuthorization,
                candidate_digest: append.candidate_digest,
            },
            |journal| {
                if journal.closure.is_some() {
                    return Err(AppendRejection::RunClosed);
                }
                if journal.authorizations.contains_key(&authorization_id) {
                    return Err(AppendRejection::AuthorizationAlreadyExists);
                }
                Ok(())
            },
            move |journal, head| {
                journal.authorizations.insert(
                    authorization_id,
                    AuthorizationState {
                        committed_head: head,
                        observed: false,
                    },
                );
            },
        )?;

        Ok(match (outcome, acknowledgement) {
            (
                InternalAppendOutcome::NewlyAppended { head },
                CommitAcknowledgement::DirectlyObserved,
            ) => AuthorizationAppendOutcome::NewlyAppended {
                head,
                authorization: NewlyAppendedAuthorization {
                    run_id: append.run_id,
                    authorization_id,
                    committed_head: head,
                },
            },
            (
                InternalAppendOutcome::NewlyAppended { .. },
                CommitAcknowledgement::LostAfterCommit,
            ) => AuthorizationAppendOutcome::OutcomeUnknown,
            (InternalAppendOutcome::AlreadyCommitted { head }, _) => {
                AuthorizationAppendOutcome::AlreadyCommitted { head }
            }
        })
    }

    pub fn observe_external_access(
        &mut self,
        append: ObservationAppend,
        acknowledgement: CommitAcknowledgement,
    ) -> Result<JournalAppendOutcome, AppendRejection> {
        let authorization_id = append.authorization_id;
        let outcome = self.append_candidate(
            AppendCandidate {
                run_id: append.run_id,
                predecessor: append.expected_head,
                append_request_id: append.append_request_id,
                purpose: AppendPurpose::ExternalAccessObservation,
                candidate_digest: append.candidate_digest,
            },
            |journal| {
                let authorization = journal
                    .authorizations
                    .get(&authorization_id)
                    .ok_or(AppendRejection::UnknownAuthorization)?;
                if authorization.observed {
                    return Err(AppendRejection::ObservationAlreadyRecorded);
                }
                if let Some(closure) = journal.closure {
                    if authorization.committed_head.run_sequence >= closure.run_sequence {
                        return Err(AppendRejection::AuthorizationNotBeforeClosure);
                    }
                }
                Ok(())
            },
            move |journal, _| {
                journal
                    .authorizations
                    .get_mut(&authorization_id)
                    .expect("validated authorization remains present")
                    .observed = true;
            },
        )?;
        Ok(surface_journal_outcome(outcome, acknowledgement))
    }

    pub fn reconcile_authorization_append(&self, append: AuthorizationAppend) -> Reconciliation {
        let Some(journal) = self.runs.get(&append.run_id) else {
            return Reconciliation::NotCommitted;
        };
        let Some(committed) = journal.appends.get(&append.append_request_id) else {
            return Reconciliation::NotCommitted;
        };
        if committed.predecessor == append.expected_head
            && committed.purpose == AppendPurpose::ExternalAccessAuthorization
            && committed.candidate_digest == append.candidate_digest
        {
            Reconciliation::Committed {
                head: committed.head,
            }
        } else {
            Reconciliation::Conflict {
                committed: committed.head,
            }
        }
    }

    pub fn current_head(&self, run_id: RunId) -> Option<JournalHead> {
        self.runs.get(&run_id).map(|journal| journal.head)
    }

    pub fn closure_head(&self, run_id: RunId) -> Option<JournalHead> {
        self.runs.get(&run_id).and_then(|journal| journal.closure)
    }

    pub fn authorized_head(
        &self,
        authority: &RunAccessAuthority,
        required_grant: RunAccessGrant,
        run_id: RunId,
    ) -> Result<JournalHead, RunAccessRejection> {
        let binding = authority.binding;
        if binding.grant != required_grant {
            return Err(RunAccessRejection::WrongGrant);
        }
        if binding.store_scope_id != self.store_scope_id {
            return Err(RunAccessRejection::WrongStore);
        }
        let journal = self
            .runs
            .get(&run_id)
            .ok_or(RunAccessRejection::RunNotFound)?;
        if binding.tenant_scope_id != journal.root.key.tenant_scope_id {
            return Err(RunAccessRejection::WrongTenant);
        }
        if binding.run_id != run_id {
            return Err(RunAccessRejection::WrongRun);
        }
        Ok(journal.head)
    }

    fn append_candidate(
        &mut self,
        candidate: AppendCandidate,
        validate: impl FnOnce(&RunJournal) -> Result<(), AppendRejection>,
        apply: impl FnOnce(&mut RunJournal, JournalHead),
    ) -> Result<InternalAppendOutcome, AppendRejection> {
        let journal = self
            .runs
            .get_mut(&candidate.run_id)
            .ok_or(AppendRejection::RunNotFound)?;

        if let Some(committed) = journal.appends.get(&candidate.append_request_id) {
            return if committed.predecessor == candidate.predecessor
                && committed.purpose == candidate.purpose
                && committed.candidate_digest == candidate.candidate_digest
            {
                Ok(InternalAppendOutcome::AlreadyCommitted {
                    head: committed.head,
                })
            } else {
                Err(AppendRejection::AppendRequestConflict {
                    committed: committed.head,
                })
            };
        }
        if journal.head != candidate.predecessor {
            return Err(AppendRejection::HeadMismatch {
                current: journal.head,
            });
        }
        validate(journal)?;

        let next_sequence = journal
            .head
            .run_sequence
            .checked_add(1)
            .ok_or(AppendRejection::SequenceOverflow)?;
        let head = JournalHead {
            run_sequence: next_sequence,
            commit_digest: CommitDigest {
                run_sequence: next_sequence,
                candidate_digest: candidate.candidate_digest,
            },
        };
        let committed = CommittedAppend {
            predecessor: candidate.predecessor,
            purpose: candidate.purpose,
            candidate_digest: candidate.candidate_digest,
            head,
        };
        match journal.appends.entry(candidate.append_request_id) {
            Entry::Vacant(entry) => {
                entry.insert(committed);
            }
            Entry::Occupied(_) => {
                unreachable!("append request uniqueness was checked before insertion")
            }
        }
        journal.head = head;
        apply(journal, head);

        Ok(InternalAppendOutcome::NewlyAppended { head })
    }
}

fn surface_journal_outcome(
    outcome: InternalAppendOutcome,
    acknowledgement: CommitAcknowledgement,
) -> JournalAppendOutcome {
    match (outcome, acknowledgement) {
        (
            InternalAppendOutcome::NewlyAppended { head },
            CommitAcknowledgement::DirectlyObserved,
        ) => JournalAppendOutcome::NewlyAppended { head },
        (InternalAppendOutcome::NewlyAppended { .. }, CommitAcknowledgement::LostAfterCommit) => {
            JournalAppendOutcome::OutcomeUnknown
        }
        (InternalAppendOutcome::AlreadyCommitted { head }, _) => {
            JournalAppendOutcome::AlreadyCommitted { head }
        }
    }
}
