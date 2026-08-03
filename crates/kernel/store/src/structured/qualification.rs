use mfm_ids::ContentRef;
use mfm_journal::structured::{AccessKind, HistoryObject};

use super::fold::StructuredStoreError;

/// Whether one physical-binding check is replaying retained history or
/// qualifying newly proposed append material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicalBindingVerificationMode {
    /// Verify an immutable certificate and release relation already retained
    /// in RunHistory. Historical releases remain valid in this mode.
    RetainedHistory,
    /// Qualify a newly proposed authorization or supersession against the
    /// deployment's exact current release.
    CurrentCandidate,
}

/// Public callback-free context for qualifying one access binding certificate.
pub struct PhysicalBindingAuthorization<'a> {
    /// Retained-history replay or current-candidate qualification.
    pub verification_mode: PhysicalBindingVerificationMode,
    /// Read or Effect access kind.
    pub access_kind: AccessKind,
    /// Exact semantic capability contract.
    pub capability_contract_ref: &'a ContentRef,
    /// Exact process-qualified capability implementation.
    pub capability_implementation_ref: &'a ContentRef,
    /// Exact semantic adapter contract.
    pub adapter_contract_ref: &'a ContentRef,
    /// Exact process-qualified adapter implementation.
    pub adapter_implementation_ref: &'a ContentRef,
    /// Immutable routing policy selected at run admission.
    pub admitted_routing_policy_ref: &'a ContentRef,
    /// Stable admitted resource-lineage contract for this access, when any.
    pub stable_resource_lineage_contract_ref: Option<&'a ContentRef>,
    /// Folded minimum non-rollback head for a refreshed attempt.
    pub minimum_lineage_head_ref: Option<&'a ContentRef>,
    /// Exact preceding physical release for a refreshed attempt.
    pub previous_physical_binding_ref: Option<&'a ContentRef>,
}

/// Public callback-free context for qualifying supersession evidence.
pub struct PhysicalBindingSupersession<'a> {
    /// Retained-history replay or current-candidate qualification.
    pub verification_mode: PhysicalBindingVerificationMode,
    /// Exact semantic capability contract of the authorized Effect.
    pub capability_contract_ref: &'a ContentRef,
    /// Exact semantic adapter contract of the authorized Effect.
    pub adapter_contract_ref: &'a ContentRef,
    /// Exact process-qualified adapter implementation of the authorized Effect.
    pub adapter_implementation_ref: &'a ContentRef,
    /// Exact authorized physical binding certificate.
    pub authorized_binding_ref: &'a ContentRef,
    /// Stable admitted refresh lineage.
    pub stable_resource_lineage_contract_ref: &'a ContentRef,
    /// Claimed monotonic public lineage head.
    pub public_lineage_head_ref: &'a ContentRef,
}

/// Purpose-limited verifier for secret-free physical-binding certificates.
///
/// Implementations may inspect only immutable public certificate and lineage
/// data. They receive no invoker, target session, credential, writer, mutation
/// permit, fence issuer, or domain-specific resource authority.
pub trait PublicPhysicalBindingVerifier: Send + Sync {
    /// Verifies exact implementation membership and any required non-rollback
    /// descendant relation for one proposed authorization.
    fn verify_authorization(
        &self,
        context: &PhysicalBindingAuthorization<'_>,
        certificate: &HistoryObject,
    ) -> std::result::Result<(), StructuredStoreError>;

    /// Verifies that one Effect supersession proof belongs to the authorized
    /// binding and advances only its admitted stable lineage.
    fn verify_supersession(
        &self,
        context: &PhysicalBindingSupersession<'_>,
        public_lineage_head: &HistoryObject,
        evidence: &HistoryObject,
    ) -> std::result::Result<(), StructuredStoreError>;
}
