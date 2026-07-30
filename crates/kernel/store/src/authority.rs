use std::marker::PhantomData;
use std::sync::Arc;

use mfm_ids::{
    EntryPointId, InvocationIdentity, RunId, SemanticTypeId, StableId, StoreEpoch, StoreScopeId,
    TenantScopeId,
};

use super::{Result, StoreError};

/// Immutable identity of one authoritative store lineage.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StoreIdentity {
    store_scope_id: StoreScopeId,
    store_epoch: StoreEpoch,
}

impl StoreIdentity {
    /// Creates a store identity from its never-reused scope and lineage epoch.
    pub const fn new(store_scope_id: StoreScopeId, store_epoch: StoreEpoch) -> Self {
        Self {
            store_scope_id,
            store_epoch,
        }
    }

    /// Returns the never-reused store scope.
    pub const fn store_scope_id(&self) -> &StoreScopeId {
        &self.store_scope_id
    }

    /// Returns the store lineage epoch.
    pub const fn store_epoch(&self) -> StoreEpoch {
        self.store_epoch
    }
}

mod private {
    pub trait CommittedJournalLoadGrantSealed {}
    pub trait GrantSealed {}
    pub trait RunGrantSealed {}
}

/// Marker for authority to admit one exact logical invocation.
pub enum Admit {}

/// Marker for authority to drive one exact run.
pub enum Drive {}

/// Marker for authority to verify or reproduce one exact run.
pub enum Replay {}

/// Marker for authority to read one run's reviewed public surface.
pub enum ReadPublic {}

/// Marker for authority to inspect one run's transition trace.
pub enum InspectTrace {}

/// Marker for authority to inspect one run's safe access audit.
pub enum InspectAudit {}

/// Marker for authority to export one run and its authorized closure.
pub enum Export {}

/// Closed purpose marker accepted by store authority checks.
pub trait RunAccessGrant: private::GrantSealed {
    /// Stable purpose name used only for binding validation and reviewed diagnostics.
    const PURPOSE: &'static str;
}

/// Closed purpose marker whose authority binds one already admitted run.
pub trait ExistingRunAccessGrant: RunAccessGrant + private::RunGrantSealed {}

/// Closed existing-run purpose permitted to load the complete verified journal.
///
/// Public and inspection authorities use their dedicated store projections and cannot satisfy
/// this bound.
pub trait CommittedJournalLoadGrant:
    ExistingRunAccessGrant + private::CommittedJournalLoadGrantSealed
{
}

macro_rules! impl_run_grant {
    ($marker:ty, $purpose:literal) => {
        impl private::GrantSealed for $marker {}
        impl private::RunGrantSealed for $marker {}
        impl RunAccessGrant for $marker {
            const PURPOSE: &'static str = $purpose;
        }
        impl ExistingRunAccessGrant for $marker {}
    };
}

impl private::GrantSealed for Admit {}
impl RunAccessGrant for Admit {
    const PURPOSE: &'static str = "admit";
}
impl_run_grant!(Drive, "drive");
impl_run_grant!(Replay, "replay");
impl_run_grant!(ReadPublic, "read_public");
impl_run_grant!(InspectTrace, "inspect_trace");
impl_run_grant!(InspectAudit, "inspect_audit");
impl_run_grant!(Export, "export");

macro_rules! impl_committed_journal_load_grant {
    ($marker:ty) => {
        impl private::CommittedJournalLoadGrantSealed for $marker {}
        impl CommittedJournalLoadGrant for $marker {}
    };
}

impl_committed_journal_load_grant!(Drive);
impl_committed_journal_load_grant!(Replay);
impl_committed_journal_load_grant!(Export);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AdmitTarget {
    pub(super) tenant_scope_id: TenantScopeId,
    pub(super) entry_point_id: EntryPointId,
    pub(super) entry_point_operation_id: StableId,
    pub(super) invocation_identity: InvocationIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RunTarget {
    pub(super) tenant_scope_id: TenantScopeId,
    pub(super) run_id: RunId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AccessTarget {
    Admit(AdmitTarget),
    Run(RunTarget),
}

#[derive(Debug)]
struct StoreInstanceSeal;

/// Store-instance-bound authority to admit one qualified deployment support graph.
///
/// The token has no public constructor and intentionally implements neither `Clone` nor
/// serialization. The application composition root may mint it only after process
/// self-attestation and package/adapter qualification have completed.
#[must_use = "qualified deployment authority must be presented to its exact support store"]
pub struct QualifiedDeploymentAuthority {
    identity: StoreIdentity,
    seal: Arc<StoreInstanceSeal>,
    qualification_scope_id: SemanticTypeId,
}

impl QualifiedDeploymentAuthority {
    /// Returns the exact authoritative store lineage.
    pub const fn store_identity(&self) -> &StoreIdentity {
        &self.identity
    }

    /// Returns the sole qualification scope admitted by this decision.
    pub const fn qualification_scope_id(&self) -> &SemanticTypeId {
        &self.qualification_scope_id
    }
}

/// One purpose-bound, store-instance-bound run access decision.
///
/// The token has no public constructor and intentionally implements neither `Clone` nor
/// serialization. Possessing an identifier or journal reference cannot construct this authority.
#[must_use = "run access authority must be presented to its exact store operation"]
pub struct RunAccessAuthority<G: RunAccessGrant> {
    identity: StoreIdentity,
    seal: Arc<StoreInstanceSeal>,
    target: AccessTarget,
    _grant: PhantomData<fn(G) -> G>,
}

impl<G: ExistingRunAccessGrant> RunAccessAuthority<G> {
    /// Returns the exact tenant bound to this run authority.
    pub fn tenant_scope_id(&self) -> &TenantScopeId {
        &self.run_target().tenant_scope_id
    }

    /// Returns the exact run bound to this authority.
    pub fn run_id(&self) -> &RunId {
        &self.run_target().run_id
    }

    fn run_target(&self) -> &RunTarget {
        match &self.target {
            AccessTarget::Run(target) => target,
            AccessTarget::Admit(_) => {
                unreachable!("existing-run grant is constructed only with a run target")
            }
        }
    }
}

impl RunAccessAuthority<Admit> {
    /// Returns the tenant bound to this exact admission decision.
    pub fn tenant_scope_id(&self) -> &TenantScopeId {
        &self.admit_target().tenant_scope_id
    }

    /// Returns the stable entry-point operation bound to this admission decision.
    pub fn entry_point_operation_id(&self) -> &StableId {
        &self.admit_target().entry_point_operation_id
    }

    /// Returns the exact versioned entry point bound to this admission decision.
    pub fn entry_point_id(&self) -> &EntryPointId {
        &self.admit_target().entry_point_id
    }

    /// Returns the canonical invocation identity bound to this admission decision.
    pub fn invocation_identity(&self) -> &InvocationIdentity {
        &self.admit_target().invocation_identity
    }

    fn admit_target(&self) -> &AdmitTarget {
        match &self.target {
            AccessTarget::Admit(target) => target,
            AccessTarget::Run(_) => {
                unreachable!("admission grant is constructed only with an admission target")
            }
        }
    }
}

/// Store-bound issuer retained privately by the application composition root.
///
/// A backend obtains this value paired with its private [`StoreAuthorityContext`]. The issuer is
/// intentionally non-cloneable so application assembly has one obvious policy-to-authority seam.
pub struct RunAccessAuthorityIssuer {
    identity: StoreIdentity,
    seal: Arc<StoreInstanceSeal>,
}

impl RunAccessAuthorityIssuer {
    /// Returns the exact store identity to which this issuer is bound.
    pub const fn store_identity(&self) -> &StoreIdentity {
        &self.identity
    }

    /// Mints authority after the composition root has qualified one deployment support scope.
    pub fn authorize_qualified_deployment(
        &self,
        qualification_scope_id: SemanticTypeId,
    ) -> QualifiedDeploymentAuthority {
        QualifiedDeploymentAuthority {
            identity: self.identity.clone(),
            seal: Arc::clone(&self.seal),
            qualification_scope_id,
        }
    }

    /// Mints authority for one policy-approved logical admission.
    pub fn authorize_admit(
        &self,
        tenant_scope_id: TenantScopeId,
        entry_point_id: EntryPointId,
        entry_point_operation_id: StableId,
        invocation_identity: InvocationIdentity,
    ) -> RunAccessAuthority<Admit> {
        RunAccessAuthority {
            identity: self.identity.clone(),
            seal: Arc::clone(&self.seal),
            target: AccessTarget::Admit(AdmitTarget {
                tenant_scope_id,
                entry_point_id,
                entry_point_operation_id,
                invocation_identity,
            }),
            _grant: PhantomData,
        }
    }

    /// Mints authority for one policy-approved drive operation.
    pub fn authorize_drive(
        &self,
        tenant_scope_id: TenantScopeId,
        run_id: RunId,
    ) -> RunAccessAuthority<Drive> {
        self.authorize_run(tenant_scope_id, run_id)
    }

    /// Mints authority for one policy-approved replay operation.
    pub fn authorize_replay(
        &self,
        tenant_scope_id: TenantScopeId,
        run_id: RunId,
    ) -> RunAccessAuthority<Replay> {
        self.authorize_run(tenant_scope_id, run_id)
    }

    /// Mints authority for one policy-approved public read.
    pub fn authorize_read_public(
        &self,
        tenant_scope_id: TenantScopeId,
        run_id: RunId,
    ) -> RunAccessAuthority<ReadPublic> {
        self.authorize_run(tenant_scope_id, run_id)
    }

    /// Mints authority for one policy-approved transition-trace read.
    pub fn authorize_inspect_trace(
        &self,
        tenant_scope_id: TenantScopeId,
        run_id: RunId,
    ) -> RunAccessAuthority<InspectTrace> {
        self.authorize_run(tenant_scope_id, run_id)
    }

    /// Mints authority for one policy-approved access-audit read.
    pub fn authorize_inspect_audit(
        &self,
        tenant_scope_id: TenantScopeId,
        run_id: RunId,
    ) -> RunAccessAuthority<InspectAudit> {
        self.authorize_run(tenant_scope_id, run_id)
    }

    /// Mints authority for one policy-approved portable export.
    pub fn authorize_export(
        &self,
        tenant_scope_id: TenantScopeId,
        run_id: RunId,
    ) -> RunAccessAuthority<Export> {
        self.authorize_run(tenant_scope_id, run_id)
    }

    fn authorize_run<G: ExistingRunAccessGrant>(
        &self,
        tenant_scope_id: TenantScopeId,
        run_id: RunId,
    ) -> RunAccessAuthority<G> {
        RunAccessAuthority {
            identity: self.identity.clone(),
            seal: Arc::clone(&self.seal),
            target: AccessTarget::Run(RunTarget {
                tenant_scope_id,
                run_id,
            }),
            _grant: PhantomData,
        }
    }
}

/// Private store-instance binding held inside an authority-bearing backend.
///
/// This type is public only so a storage adapter can retain it. Creating another context with the
/// same textual identity creates a different private seal; its tokens cannot authorize this one.
#[doc(hidden)]
#[derive(Clone)]
pub struct StoreAuthorityContext {
    identity: StoreIdentity,
    seal: Arc<StoreInstanceSeal>,
}

impl StoreAuthorityContext {
    /// Creates one backend context paired with its sole application issuer.
    #[doc(hidden)]
    pub fn bootstrap(identity: StoreIdentity) -> (Self, RunAccessAuthorityIssuer) {
        let seal = Arc::new(StoreInstanceSeal);
        (
            Self {
                identity: identity.clone(),
                seal: Arc::clone(&seal),
            },
            RunAccessAuthorityIssuer { identity, seal },
        )
    }

    /// Returns the authoritative store identity.
    pub const fn store_identity(&self) -> &StoreIdentity {
        &self.identity
    }

    pub(super) fn is_same_instance(&self, other: &Self) -> bool {
        self.identity == other.identity && Arc::ptr_eq(&self.seal, &other.seal)
    }

    pub(super) fn validate_admit<'a>(
        &self,
        authority: &'a RunAccessAuthority<Admit>,
    ) -> Result<&'a AdmitTarget> {
        self.validate_common(authority)?;
        Ok(authority.admit_target())
    }

    pub(super) fn validate_run<'a, G: ExistingRunAccessGrant>(
        &self,
        authority: &'a RunAccessAuthority<G>,
    ) -> Result<&'a RunTarget> {
        self.validate_common(authority)?;
        Ok(authority.run_target())
    }

    pub(super) fn validate_qualified_deployment<'a>(
        &self,
        authority: &'a QualifiedDeploymentAuthority,
    ) -> Result<&'a SemanticTypeId> {
        if self.identity != authority.identity || !Arc::ptr_eq(&self.seal, &authority.seal) {
            return Err(StoreError::AccessDenied {
                purpose: "admit_support_graph",
            });
        }
        Ok(&authority.qualification_scope_id)
    }

    fn validate_common<G: RunAccessGrant>(&self, authority: &RunAccessAuthority<G>) -> Result<()> {
        if self.identity != authority.identity || !Arc::ptr_eq(&self.seal, &authority.seal) {
            return Err(StoreError::AccessDenied {
                purpose: G::PURPOSE,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(value: char) -> StoreScopeId {
        StoreScopeId::new(format!(
            "{}{}",
            StoreScopeId::PREFIX,
            value.to_string().repeat(32)
        ))
        .expect("store scope")
    }

    fn tenant(value: char) -> TenantScopeId {
        TenantScopeId::new(format!(
            "{}{}",
            TenantScopeId::PREFIX,
            value.to_string().repeat(32)
        ))
        .expect("tenant scope")
    }

    fn run() -> RunId {
        RunId::parse(
            "run:sha256-jcs-v1:\
             0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("run id")
    }

    #[test]
    fn textual_identity_cannot_substitute_an_instance_seal() {
        let identity = StoreIdentity::new(scope('a'), StoreEpoch::new(7));
        let (left, left_issuer) = StoreAuthorityContext::bootstrap(identity.clone());
        let (right, _) = StoreAuthorityContext::bootstrap(identity);
        let authority = left_issuer.authorize_drive(tenant('b'), run());

        assert!(left.validate_run(&authority).is_ok());
        assert_eq!(
            right.validate_run(&authority),
            Err(StoreError::AccessDenied { purpose: "drive" })
        );
    }

    #[test]
    fn admission_identity_is_frozen_and_checked() {
        let (_, issuer) =
            StoreAuthorityContext::bootstrap(StoreIdentity::new(scope('a'), StoreEpoch::new(1)));
        let authority = issuer.authorize_admit(
            tenant('b'),
            EntryPointId::new("mfm.portfolio/snapshot@1").expect("entry point id"),
            StableId::new("mfm.portfolio/snapshot").expect("operation id"),
            InvocationIdentity::new("01234567-89ab-4cde-8fab-0123456789ab")
                .expect("invocation identity"),
        );
        assert_eq!(
            authority.entry_point_operation_id().as_str(),
            "mfm.portfolio/snapshot"
        );
    }
}
