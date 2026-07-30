use std::collections::BTreeMap;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{FieldPath, SemanticTypeId};
use mfm_journal::{ProducerBinding, ValueRef};
use mfm_spec::RetainedValueContract;

use super::objects::{derive_value_ref, StagedObject};
use super::{
    AsyncStoreFuture, QualifiedDeploymentAuthority, QualifiedRunStore, Result, RunJournalBackend,
    StoreAuthorityContext, StoreError, StoreIdentity,
};

/// One producer-free canonical support-object proposal.
///
/// Qualification supplies only reviewed bytes and their certified retained-value contract.
/// Producer authority is added exclusively by the store after validating a matching sealed
/// deployment authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedSupportMember {
    field_path: FieldPath,
    canonical: PlainCanonicalJsonBytes,
    value_contract: RetainedValueContract,
}

impl QualifiedSupportMember {
    /// Constructs one producer-free qualified support member.
    pub const fn new(
        field_path: FieldPath,
        canonical: PlainCanonicalJsonBytes,
        value_contract: RetainedValueContract,
    ) -> Self {
        Self {
            field_path,
            canonical,
            value_contract,
        }
    }

    /// Returns the stable field within the qualification scope.
    pub const fn field_path(&self) -> &FieldPath {
        &self.field_path
    }

    /// Returns exact canonical support bytes.
    pub const fn canonical(&self) -> &PlainCanonicalJsonBytes {
        &self.canonical
    }

    /// Returns the complete producer-free retained-value contract.
    pub const fn value_contract(&self) -> &RetainedValueContract {
        &self.value_contract
    }
}

/// Complete transient producer-free support graph from one qualified deployment scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedSupportGraph {
    qualification_scope_id: SemanticTypeId,
    members: BTreeMap<FieldPath, QualifiedSupportMember>,
}

impl QualifiedSupportGraph {
    /// Constructs a uniquely keyed qualified support graph.
    pub fn new(
        qualification_scope_id: SemanticTypeId,
        members: impl IntoIterator<Item = QualifiedSupportMember>,
    ) -> Result<Self> {
        let mut keyed = BTreeMap::new();
        for member in members {
            if keyed.insert(member.field_path.clone(), member).is_some() {
                return Err(StoreError::InvalidObjectAuthority {
                    message: "qualified support graph contains a duplicate field path",
                });
            }
        }
        if keyed.is_empty() {
            return Err(StoreError::InvalidObjectAuthority {
                message: "qualified support graph must contain a member",
            });
        }
        Ok(Self {
            qualification_scope_id,
            members: keyed,
        })
    }

    /// Returns the exact qualification scope.
    pub const fn qualification_scope_id(&self) -> &SemanticTypeId {
        &self.qualification_scope_id
    }

    /// Returns producer-free members in canonical field-path order.
    pub const fn members(&self) -> &BTreeMap<FieldPath, QualifiedSupportMember> {
        &self.members
    }
}

/// Sealed proof of one exact store-admitted qualified support graph.
///
/// The value has no public constructor and is intentionally non-cloneable. Runtime registration
/// may consume its verified references but cannot turn arbitrary references into qualification
/// proof.
pub struct AdmittedSupportGraph {
    authority: StoreAuthorityContext,
    qualification_scope_id: SemanticTypeId,
    members: BTreeMap<FieldPath, AdmittedSupportMember>,
}

/// One exact member retained by a sealed admitted support graph.
///
/// The member has no public constructor. Its reference and bytes were verified together before
/// the backend atomically published the complete support graph.
pub struct AdmittedSupportMember {
    value_ref: ValueRef,
    bytes: Vec<u8>,
}

impl AdmittedSupportMember {
    /// Returns the complete store-authored retained authority.
    pub const fn value_ref(&self) -> &ValueRef {
        &self.value_ref
    }

    /// Returns the exact verified canonical bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl AdmittedSupportGraph {
    pub(super) const fn new(
        authority: StoreAuthorityContext,
        qualification_scope_id: SemanticTypeId,
        members: BTreeMap<FieldPath, AdmittedSupportMember>,
    ) -> Self {
        Self {
            authority,
            qualification_scope_id,
            members,
        }
    }

    /// Returns the exact admitted qualification scope.
    pub const fn qualification_scope_id(&self) -> &SemanticTypeId {
        &self.qualification_scope_id
    }

    /// Returns exact verified members in canonical field-path order.
    pub const fn members(&self) -> &BTreeMap<FieldPath, AdmittedSupportMember> {
        &self.members
    }

    /// Resolves one exact qualified support field.
    pub fn member(&self, field_path: &FieldPath) -> Option<&AdmittedSupportMember> {
        self.members.get(field_path)
    }

    pub(super) fn belongs_to(&self, authority: &StoreAuthorityContext) -> bool {
        self.authority.is_same_instance(authority)
    }
}

/// One exact store-authored support member awaiting atomic backend admission.
pub struct PreparedSupportMember {
    field_path: FieldPath,
    staged: StagedObject,
}

impl PreparedSupportMember {
    /// Returns the stable path bound into producer authority.
    pub const fn field_path(&self) -> &FieldPath {
        &self.field_path
    }

    /// Returns the complete store-authored retained authority.
    pub const fn value_ref(&self) -> &ValueRef {
        self.staged.value_ref()
    }

    /// Returns exact canonical bytes to admit or verify.
    pub fn bytes(&self) -> &[u8] {
        self.staged.bytes()
    }
}

/// Complete exact support graph prepared by the store for one atomic backend operation.
pub struct PreparedSupportGraph {
    authority: StoreAuthorityContext,
    qualification_scope_id: SemanticTypeId,
    members: BTreeMap<FieldPath, PreparedSupportMember>,
}

impl PreparedSupportGraph {
    fn prepare(authority: StoreAuthorityContext, graph: QualifiedSupportGraph) -> Result<Self> {
        let qualification_scope_id = graph.qualification_scope_id;
        let mut members = BTreeMap::new();
        for (field_path, member) in graph.members {
            let producer =
                ProducerBinding::qualified_support(&qualification_scope_id, &field_path)?;
            let value_ref = derive_value_ref(
                &member.value_contract,
                &producer,
                member.canonical.as_bytes(),
            )?;
            let staged = StagedObject::new(value_ref, member.canonical.to_vec())?;
            members.insert(
                field_path.clone(),
                PreparedSupportMember { field_path, staged },
            );
        }
        Ok(Self {
            authority,
            qualification_scope_id,
            members,
        })
    }

    /// Returns the authoritative target store lineage.
    pub const fn store_identity(&self) -> &StoreIdentity {
        self.authority.store_identity()
    }

    /// Returns the exact qualification scope.
    pub const fn qualification_scope_id(&self) -> &SemanticTypeId {
        &self.qualification_scope_id
    }

    /// Returns all exact prepared members in canonical path order.
    pub const fn members(&self) -> &BTreeMap<FieldPath, PreparedSupportMember> {
        &self.members
    }
}

/// Store-created verifier handed to a backend for one exact support-graph admission.
pub struct SupportGraphAdmissionVerifier {
    prepared: PreparedSupportGraph,
}

impl SupportGraphAdmissionVerifier {
    fn new(prepared: PreparedSupportGraph) -> Self {
        Self { prepared }
    }

    /// Returns the exact prepared support graph.
    pub const fn prepared(&self) -> &PreparedSupportGraph {
        &self.prepared
    }

    /// Completes the sealed admission result after durable atomic publication.
    pub fn complete(self) -> AdmittedSupportGraph {
        let PreparedSupportGraph {
            authority,
            qualification_scope_id,
            members,
        } = self.prepared;
        AdmittedSupportGraph::new(
            authority,
            qualification_scope_id,
            members
                .into_iter()
                .map(|(path, member)| {
                    (
                        path,
                        AdmittedSupportMember {
                            value_ref: member.staged.value_ref().clone(),
                            bytes: member.staged.bytes().to_vec(),
                        },
                    )
                })
                .collect(),
        )
    }
}

/// Trusted backend seam for atomically admitting qualified support graphs.
pub trait SupportBackend: RunJournalBackend {
    /// Atomically admits or verifies every exact member, then returns the sealed graph.
    fn backend_admit_support_graph<'a>(
        &'a self,
        verifier: SupportGraphAdmissionVerifier,
    ) -> AsyncStoreFuture<'a, AdmittedSupportGraph, Self::Error>;
}

impl<B: SupportBackend> QualifiedRunStore<B> {
    /// Admits one complete producer-free qualified graph before the one-shot history split.
    pub fn admit_support_graph<'a>(
        &'a self,
        authority: &'a QualifiedDeploymentAuthority,
        graph: QualifiedSupportGraph,
    ) -> AsyncStoreFuture<'a, AdmittedSupportGraph, B::Error> {
        let checked = (|| {
            let allowed = self
                .backend()
                .store_authority_context()
                .validate_qualified_deployment(authority)?;
            if allowed != graph.qualification_scope_id() {
                return Err(StoreError::AccessDenied {
                    purpose: "admit_support_graph",
                });
            }
            let prepared = PreparedSupportGraph::prepare(
                self.backend().store_authority_context().clone(),
                graph,
            )?;
            Ok(SupportGraphAdmissionVerifier::new(prepared))
        })();
        match checked {
            Ok(verifier) => self.backend().backend_admit_support_graph(verifier),
            Err(error) => Box::pin(async move { Err(error.into()) }),
        }
    }
}
