use std::collections::{BTreeMap, BTreeSet};

use mfm_ids::{ContentRef, FieldPath, RunId, TenantScopeId};
use mfm_journal::{CrossRunSourceRef, CrossRunSourceRefFields, ProducerBinding, RunPhase};
use mfm_spec::RetainedValueContract;

use super::objects::{derive_value_ref, validate_value_contract};
use super::preparation::VerifiedAdmissionSource;
use super::{
    Admit, AsyncStoreFuture, CommittedJournalCommit, CommittedObject, CommittedRunJournal,
    JournalLoadVerifier, Result, RunAccessAuthority, RunHistoryReader, RunJournalBackend,
    StoreAuthorityContext, StoreError, VerifiedAdmissionSources, VerifiedRunView,
};

const MAX_ADMISSION_SOURCES: usize = 4_096;

/// One producer-free closed source root proposed for admission verification.
pub struct ProposedAdmissionSourceRoot {
    field_path: FieldPath,
    source_ref: CrossRunSourceRef,
    root_contract: RetainedValueContract,
    source_role_ref: ContentRef,
}

impl ProposedAdmissionSourceRoot {
    /// Constructs one named source proposal with its complete destination contract and source role.
    pub fn new(
        field_path: FieldPath,
        source_ref: CrossRunSourceRef,
        root_contract: RetainedValueContract,
        source_role_ref: ContentRef,
    ) -> Result<Self> {
        match source_ref.fields()? {
            CrossRunSourceRefFields::EffectiveOutput { .. } => {
                if root_contract.evidence_contract_ref() != &source_role_ref {
                    return Err(StoreError::InvalidSourceClosure);
                }
            }
            CrossRunSourceRefFields::EvidenceOnly {
                certified_evidence_role_ref,
                ..
            } => {
                if certified_evidence_role_ref != source_role_ref {
                    return Err(StoreError::InvalidSourceClosure);
                }
            }
        }
        Ok(Self {
            field_path,
            source_ref,
            root_contract,
            source_role_ref,
        })
    }

    /// Returns the stable destination field in the admission source manifest.
    pub const fn field_path(&self) -> &FieldPath {
        &self.field_path
    }

    /// Returns the exact closed source lineage.
    pub const fn source_ref(&self) -> &CrossRunSourceRef {
        &self.source_ref
    }

    /// Returns the complete certified destination root contract.
    pub const fn root_contract(&self) -> &RetainedValueContract {
        &self.root_contract
    }

    /// Returns the exact certified source role.
    pub const fn source_role_ref(&self) -> &ContentRef {
        &self.source_role_ref
    }
}

/// Canonical nonempty direct source set proposed for one admission.
pub struct ProposedAdmissionSources {
    roots: Vec<ProposedAdmissionSourceRoot>,
}

impl ProposedAdmissionSources {
    /// Constructs a bounded, canonically ordered, duplicate-free direct source set.
    pub fn new(mut roots: Vec<ProposedAdmissionSourceRoot>) -> Result<Self> {
        if roots.is_empty() || roots.len() > MAX_ADMISSION_SOURCES {
            return Err(StoreError::InvalidSourceClosure);
        }
        roots.sort_by(|left, right| left.field_path.cmp(&right.field_path));
        if roots
            .windows(2)
            .any(|pair| pair[0].field_path == pair[1].field_path)
        {
            return Err(StoreError::InvalidSourceClosure);
        }
        Ok(Self { roots })
    }

    /// Iterates direct roots in canonical destination-field order.
    pub fn roots(
        &self,
    ) -> impl ExactSizeIterator<Item = &ProposedAdmissionSourceRoot> + DoubleEndedIterator {
        self.roots.iter()
    }
}

/// Affine store verifier for one exact direct and recursively discovered source closure.
///
/// Backends can only load the exact pending coordinates exposed by this token. Every supplied
/// source is physically and semantically verified before it can extend the traversal.
pub struct AdmissionSourceVerifier {
    authority: StoreAuthorityContext,
    tenant_scope_id: TenantScopeId,
    proposed: ProposedAdmissionSources,
    pending: BTreeSet<RunId>,
    accepted: BTreeMap<RunId, VerifiedRunView>,
}

impl AdmissionSourceVerifier {
    pub(super) fn new(
        authority: StoreAuthorityContext,
        tenant_scope_id: TenantScopeId,
        proposed: ProposedAdmissionSources,
    ) -> Result<Self> {
        let pending = proposed
            .roots()
            .map(|root| {
                root.source_ref()
                    .identity()
                    .map(|identity| identity.source_run_id)
            })
            .collect::<mfm_journal::Result<BTreeSet<_>>>()?;
        Ok(Self {
            authority,
            tenant_scope_id,
            proposed,
            pending,
            accepted: BTreeMap::new(),
        })
    }

    /// Returns a verifier for the next exact pending source coordinate.
    ///
    /// The returned token has no public constructor and can load only this verifier's store,
    /// tenant, and lexicographically first pending run.
    pub fn pending_source_load_verifier(&self) -> Option<JournalLoadVerifier> {
        self.pending.iter().next().map(|run_id| {
            JournalLoadVerifier::new(
                self.authority.store_identity().clone(),
                self.tenant_scope_id.clone(),
                run_id.clone(),
            )
        })
    }

    /// Verifies and accepts raw rows for the next exact pending source coordinate.
    pub fn verify_pending_source_rows(
        &mut self,
        commits: Vec<CommittedJournalCommit>,
        objects: Vec<CommittedObject>,
    ) -> Result<()> {
        let verifier = self
            .pending_source_load_verifier()
            .ok_or(StoreError::InvalidSourceClosure)?;
        let journal = verifier.verify(commits, objects)?;
        self.accept_verified_source(journal)
    }

    /// Accepts one physically verified source journal and extends recursive traversal.
    pub fn accept_verified_source(&mut self, journal: CommittedRunJournal) -> Result<()> {
        let run_id = journal.run_id().clone();
        if !self.pending.remove(&run_id)
            || journal.store_identity() != self.authority.store_identity()
            || journal.tenant_scope_id() != &self.tenant_scope_id
            || self.accepted.contains_key(&run_id)
        {
            return Err(StoreError::SourceScopeMismatch);
        }
        let view = journal.verify_recorded_history()?;
        if view.run_phase() != RunPhase::Closed {
            return Err(StoreError::InvalidSourceClosure);
        }
        for requirement in view.admission_source_requirements().cross_run_sources() {
            let dependency = requirement.source_run_id();
            if dependency == &run_id {
                return Err(StoreError::InvalidSourceClosure);
            }
            if !self.accepted.contains_key(dependency) {
                self.pending.insert(dependency.clone());
            }
        }
        self.accepted.insert(run_id, view);
        Ok(())
    }

    /// Completes the drained recursive traversal and seals copied destination roots.
    pub fn complete(self) -> Result<VerifiedAdmissionSources> {
        if !self.pending.is_empty() {
            return Err(StoreError::InvalidSourceClosure);
        }
        validate_recursive_closure(&self.accepted)?;

        let mut entries = BTreeMap::new();
        for root in self.proposed.roots {
            let identity = root.source_ref.identity()?;
            let source_view = self
                .accepted
                .get(&identity.source_run_id)
                .ok_or(StoreError::InvalidSourceClosure)?;
            let source_value = selected_source_value(source_view, &root.source_ref)?;
            validate_value_contract(&root.root_contract, source_value.value_ref())?;
            let destination_value_ref = derive_value_ref(
                &root.root_contract,
                &ProducerBinding::source_run(&root.source_ref)?,
                source_value.bytes(),
            )?;
            super::journal::verify_cross_run_source_view(
                &root.source_ref,
                &destination_value_ref,
                source_view,
            )?;
            let entry = VerifiedAdmissionSource {
                source: root.source_ref,
                value_ref: destination_value_ref,
                bytes: source_value.bytes().to_vec(),
                value_contract: root.root_contract,
                source_role_ref: root.source_role_ref,
            };
            if entries.insert(root.field_path, entry).is_some() {
                return Err(StoreError::InvalidSourceClosure);
            }
        }
        Ok(VerifiedAdmissionSources {
            authority: self.authority,
            tenant_scope_id: self.tenant_scope_id,
            entries,
        })
    }
}

fn validate_recursive_closure(sources: &BTreeMap<RunId, VerifiedRunView>) -> Result<()> {
    let mut visiting = BTreeSet::new();
    let mut verified = BTreeSet::new();
    for run_id in sources.keys() {
        validate_source_dag(run_id, sources, &mut visiting, &mut verified)?;
    }
    Ok(())
}

fn validate_source_dag(
    run_id: &RunId,
    sources: &BTreeMap<RunId, VerifiedRunView>,
    visiting: &mut BTreeSet<RunId>,
    verified: &mut BTreeSet<RunId>,
) -> Result<()> {
    if verified.contains(run_id) {
        return Ok(());
    }
    if !visiting.insert(run_id.clone()) {
        return Err(StoreError::InvalidSourceClosure);
    }
    let view = sources
        .get(run_id)
        .ok_or(StoreError::InvalidSourceClosure)?;
    for requirement in view.admission_source_requirements().cross_run_sources() {
        let dependency = sources
            .get(requirement.source_run_id())
            .ok_or(StoreError::InvalidSourceClosure)?;
        if dependency.store_identity() != view.store_identity()
            || dependency.tenant_scope_id() != view.tenant_scope_id()
        {
            return Err(StoreError::SourceScopeMismatch);
        }
        requirement.verify_closed_source_view(dependency)?;
        validate_source_dag(requirement.source_run_id(), sources, visiting, verified)?;
    }
    visiting.remove(run_id);
    verified.insert(run_id.clone());
    Ok(())
}

fn selected_source_value<'a>(
    source_view: &'a VerifiedRunView,
    source_ref: &CrossRunSourceRef,
) -> Result<&'a CommittedObject> {
    match source_ref.fields()? {
        CrossRunSourceRefFields::EffectiveOutput {
            effective_output_ref,
            ..
        } => {
            let value_ref = source_view
                .output_binding(&effective_output_ref)?
                .fields()?
                .value_ref;
            source_view.retained_value(&value_ref)
        }
        CrossRunSourceRefFields::EvidenceOnly {
            raw_result_or_evidence_ref,
            ..
        } => {
            let expected = raw_result_or_evidence_ref.fields()?;
            source_view
                .journal()
                .objects()
                .find(|object| {
                    object.value_ref().fields().is_ok_and(|actual| {
                        actual.artifact_id == expected.artifact_id
                            && actual.content_digest == expected.content_digest
                            && actual.evidence_hash == expected.evidence_hash
                            && actual.schema_id == expected.schema_id
                            && actual.semantic_type_id == expected.semantic_type_id
                            && actual.role == expected.role
                            && actual.byte_length == expected.byte_length
                            && actual.media_type == expected.media_type
                            && actual.evidence_contract_ref == expected.evidence_contract_ref
                    })
                })
                .ok_or(StoreError::InvalidSourceClosure)
        }
    }
}

/// Trusted backend seam for verifying a complete admission source closure.
pub trait AdmissionSourceBackend: RunJournalBackend {
    /// Loads and verifies every exact pending source, then completes the affine verifier.
    fn backend_verify_admission_sources<'a>(
        &'a self,
        verifier: AdmissionSourceVerifier,
    ) -> AsyncStoreFuture<'a, VerifiedAdmissionSources, Self::Error>;
}

impl<B: AdmissionSourceBackend> RunHistoryReader<B> {
    /// Mints the exact store-sealed empty source set under admission authority.
    pub fn verify_no_admission_sources<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<Admit>,
    ) -> AsyncStoreFuture<'a, VerifiedAdmissionSources, B::Error> {
        let checked: Result<VerifiedAdmissionSources> = (|| {
            let target = self
                .backend()
                .store_authority_context()
                .validate_admit(authority)?;
            Ok(VerifiedAdmissionSources::empty(
                self.backend().store_authority_context().clone(),
                target.tenant_scope_id.clone(),
            ))
        })();
        Box::pin(async move { checked.map_err(Into::into) })
    }

    /// Verifies one nonempty direct source set and its complete recursive closure.
    pub fn verify_admission_sources<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<Admit>,
        proposed: ProposedAdmissionSources,
    ) -> AsyncStoreFuture<'a, VerifiedAdmissionSources, B::Error> {
        let checked = (|| {
            let target = self
                .backend()
                .store_authority_context()
                .validate_admit(authority)?;
            AdmissionSourceVerifier::new(
                self.backend().store_authority_context().clone(),
                target.tenant_scope_id.clone(),
                proposed,
            )
        })();
        match checked {
            Ok(verifier) => self.backend().backend_verify_admission_sources(verifier),
            Err(error) => Box::pin(async move { Err(error.into()) }),
        }
    }
}
