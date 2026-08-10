//! Structured EVM submission values, leaf capabilities, and state contracts.

use std::str::FromStr;

use alloy_primitives::{Address, B256, U256};
use mfm_capabilities::{
    BoundedComponentContract, EffectCapabilityContract, EntryAbsorbing, EntryKeyed,
    ReadCapabilityContract, Refreshable, ResourceAuthorityContract, SignerContract,
};
use mfm_ids::{StableId, TenantScopeId};
use mfm_program::structured::{
    CapabilityExpansion, Direct, Effect, EntryAbsorbingBinding, Never, Pure, Read,
    RefreshableBinding, RequiresCapability, RuntimeEffectCapability, RuntimeReadCapability,
    RuntimeResourceAuthority, RuntimeSigner, SafeFailureMayFail, SafeFailureNotApplicable, State,
};
use mfm_program_derive::{MfmConfig, MfmValue, PublicOutputs};
use mfm_spec::structured::{
    structured_value_contract_ref, StructuredComponentDependency, StructuredComponentKind,
    StructuredLiveComponentContract,
};
use serde::{Deserialize, Serialize};

use crate::{
    derive_authenticated_intent_issuer_id, derive_evm_chain_lineage_id,
    derive_submission_intent_id, derive_submission_semantics_digest, derive_wallet_nonce_domain,
    ActivateCandidateResponse, ActivateEvmCandidateRequest, ActivateWalletCandidateCapability,
    ActiveWalletCandidate, AttestedWalletCandidate, AuthenticatedIntentIssuerId,
    CompleteEvmNonceRequest, CompleteWalletNonceCapability, CompletedWalletNonce,
    EvmCallerSubmissionToken, EvmCandidateFamily, EvmNonceReservationKey, EvmSubmissionFailure,
    EvmTransactionIntent, EvmWalletFeeCandidate, EvmWalletReference, ObservedPendingNonceFloor,
    QualifiedChainInstanceId, QualifiedPendingNonceFloor, ReadEvmWalletNonceStatusRequest,
    ReadWalletNonceStatusCapability, ReserveEvmNonceRequest, ReserveWalletNonceCapability,
    ReservedWalletNonce, SubmissionIntentId, SubmissionSemanticsDigest, WalletNonceDomain,
    WalletNonceDomainActivationAttestation, WalletNonceStatus, EVM_WALLET_OBSERVATION_ROUND_LIMIT,
};

/// Immutable deployment-configured semantics for one EVM submission.
///
/// Authentication-derived tenant, principal, and caller-token material is deliberately absent.
/// The application combines this value with a fresh authorization result to construct
/// [`EvmSubmissionRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue, MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "submission-configuration",
    version = "1",
    schema = "mfm.evm.submission_configuration",
    validate = "validate_submission_configuration"
)]
pub struct EvmSubmissionConfiguration {
    domain_activation_attestation: WalletNonceDomainActivationAttestation,
    issuer_namespace_contract_ref: EvmWalletReference,
    route_generation_ref: EvmWalletReference,
    transaction_intent: EvmTransactionIntent,
    candidate_family: EvmCandidateFamily,
    observation_rounds: u8,
}

impl EvmSubmissionConfiguration {
    /// Constructs and validates one principal-free configured submission.
    pub fn new(
        domain_activation_attestation: WalletNonceDomainActivationAttestation,
        issuer_namespace_contract_ref: EvmWalletReference,
        route_generation_ref: EvmWalletReference,
        transaction_intent: EvmTransactionIntent,
        candidate_family: EvmCandidateFamily,
        observation_rounds: u8,
    ) -> Result<Self, crate::WalletAuthorityContractError> {
        let value = Self {
            domain_activation_attestation,
            issuer_namespace_contract_ref,
            route_generation_ref,
            transaction_intent,
            candidate_family,
            observation_rounds,
        };
        value.validate()?;
        Ok(value)
    }

    /// Returns the immutable transaction intent.
    pub const fn transaction_intent(&self) -> &EvmTransactionIntent {
        &self.transaction_intent
    }

    /// Returns the immutable idempotency namespace contract.
    pub const fn issuer_namespace_contract_ref(&self) -> &EvmWalletReference {
        &self.issuer_namespace_contract_ref
    }

    /// Returns the declaration-ordered candidate family.
    pub const fn candidate_family(&self) -> &EvmCandidateFamily {
        &self.candidate_family
    }

    /// Returns the exact public domain activation attestation.
    pub const fn domain_activation_attestation(&self) -> &WalletNonceDomainActivationAttestation {
        &self.domain_activation_attestation
    }

    /// Returns the qualified route generation.
    pub const fn route_generation_ref(&self) -> &EvmWalletReference {
        &self.route_generation_ref
    }

    /// Returns one or two admitted observation rounds.
    pub const fn observation_rounds(&self) -> u8 {
        self.observation_rounds
    }

    /// Revalidates the configured submission semantics and authority references.
    pub fn validate(&self) -> Result<(), crate::WalletAuthorityContractError> {
        validate_submission_semantics(
            &self.domain_activation_attestation,
            &self.issuer_namespace_contract_ref,
            &self.route_generation_ref,
            &self.transaction_intent,
            &self.candidate_family,
            self.observation_rounds,
        )
    }

    /// Derives and verifies every code-owned semantic selected for production deployment.
    ///
    /// Syntax-valid caller references are insufficient. The signer, signing
    /// profile, broadcast capability, abstract submission state, expansion
    /// recipe, and terminal assurance policy are recomputed from their current
    /// domain contracts.
    pub fn deployment_semantics(
        &self,
        qualified_semantic_signer_id: &StableId,
        qualified_signer_identity: &mfm_signing::PublicSigningIdentity,
    ) -> Result<EvmDeploymentSubmissionSemantics, crate::WalletAuthorityContractError> {
        self.validate()?;
        let signer_contract = EvmCandidateSigner::contract()
            .map_err(|_| crate::WalletAuthorityContractError::Invalid("semantic_signer"))?;
        let semantic_signer_id = crate::derive_evm_semantic_signer_id(qualified_signer_identity)
            .map_err(|_| crate::WalletAuthorityContractError::Invalid("semantic_signer"))?;
        let semantic_signer_contract_ref = signer_contract
            .content_ref()
            .map(EvmWalletReference::from_content_ref)
            .map_err(|_| crate::WalletAuthorityContractError::Invalid("semantic_signer"))?;
        let signing_profile_contract_ref = crate::evm_deterministic_signing_profile_ref()
            .map_err(|_| crate::WalletAuthorityContractError::Invalid("signing_profile"))?;
        let broadcast_contract_ref = BroadcastExactCandidateCapability::contract()
            .and_then(|contract| contract.content_ref().map_err(Into::into))
            .map(EvmWalletReference::from_content_ref)
            .map_err(|_| crate::WalletAuthorityContractError::Invalid("submission_contract"))?;
        let submission_contract_ref = structured_value_contract_ref::<EvmSubmissionConfiguration>()
            .map(EvmWalletReference::from_content_ref)
            .map_err(|_| crate::WalletAuthorityContractError::Invalid("submission_contract"))?;
        let expansion_contract_ref = crate::evm_submission_expansion_policy_ref()
            .map_err(|_| crate::WalletAuthorityContractError::Invalid("submission_expansion"))?;
        let terminal_assurance_contract_ref = crate::evm_wallet_assurance_policy_ref()
            .map_err(|_| crate::WalletAuthorityContractError::Invalid("terminal_assurance"))?;
        let expected_sender = Address::from_str(self.transaction_intent.nonce_domain().sender())
            .map_err(|_| crate::WalletAuthorityContractError::Invalid("sender"))?;
        let qualified_sender = qualified_signer_identity
            .account_id()
            .and_then(|account| Address::from_str(account).ok());
        if qualified_semantic_signer_id != &semantic_signer_id
            || self.transaction_intent.semantic_signer_id() != semantic_signer_id.as_str()
            || qualified_sender != Some(expected_sender)
            || self.transaction_intent.signing_profile_contract_ref()
                != &signing_profile_contract_ref
            || self.transaction_intent.submission_contract_ref() != &broadcast_contract_ref
            || self.transaction_intent.terminal_assurance_contract_ref()
                != &terminal_assurance_contract_ref
        {
            return Err(crate::WalletAuthorityContractError::Invalid(
                "deployment_semantics",
            ));
        }
        Ok(EvmDeploymentSubmissionSemantics {
            semantic_signer_id,
            semantic_signer_contract_ref,
            expected_sender,
            signing_profile_contract_ref,
            broadcast_contract_ref,
            submission_contract_ref,
            expansion_contract_ref,
            terminal_assurance_contract_ref,
            route_generation_ref: self.route_generation_ref.clone(),
            domain_activation_attestation: self.domain_activation_attestation.clone(),
            candidate_family_digest: self.candidate_family.digest().to_owned(),
        })
    }
}

/// Exact code-derived semantic tuple admitted for one production deployment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmDeploymentSubmissionSemantics {
    semantic_signer_id: StableId,
    semantic_signer_contract_ref: EvmWalletReference,
    expected_sender: Address,
    signing_profile_contract_ref: EvmWalletReference,
    broadcast_contract_ref: EvmWalletReference,
    submission_contract_ref: EvmWalletReference,
    expansion_contract_ref: EvmWalletReference,
    terminal_assurance_contract_ref: EvmWalletReference,
    route_generation_ref: EvmWalletReference,
    domain_activation_attestation: WalletNonceDomainActivationAttestation,
    candidate_family_digest: String,
}

impl EvmDeploymentSubmissionSemantics {
    /// Returns the canonical semantic signer identity.
    pub const fn semantic_signer_id(&self) -> &StableId {
        &self.semantic_signer_id
    }

    /// Returns the canonical semantic signer contract.
    pub const fn semantic_signer_contract_ref(&self) -> &EvmWalletReference {
        &self.semantic_signer_contract_ref
    }

    /// Returns the exact sender fixed by the nonce domain.
    pub const fn expected_sender(&self) -> Address {
        self.expected_sender
    }

    /// Returns the canonical deterministic signing-profile contract.
    pub const fn signing_profile_contract_ref(&self) -> &EvmWalletReference {
        &self.signing_profile_contract_ref
    }

    /// Returns the canonical exact-broadcast capability contract.
    pub const fn broadcast_contract_ref(&self) -> &EvmWalletReference {
        &self.broadcast_contract_ref
    }

    /// Returns the canonical structured submission-configuration contract.
    pub const fn submission_contract_ref(&self) -> &EvmWalletReference {
        &self.submission_contract_ref
    }

    /// Returns the canonical deterministic submission-expansion contract.
    pub const fn expansion_contract_ref(&self) -> &EvmWalletReference {
        &self.expansion_contract_ref
    }

    /// Returns the canonical terminal assurance-policy contract.
    pub const fn terminal_assurance_contract_ref(&self) -> &EvmWalletReference {
        &self.terminal_assurance_contract_ref
    }

    /// Returns the exact qualified route generation.
    pub const fn route_generation_ref(&self) -> &EvmWalletReference {
        &self.route_generation_ref
    }

    /// Returns the exact wallet-domain activation.
    pub const fn domain_activation_attestation(&self) -> &WalletNonceDomainActivationAttestation {
        &self.domain_activation_attestation
    }

    /// Returns the exact candidate-family identity.
    pub fn candidate_family_digest(&self) -> &str {
        &self.candidate_family_digest
    }
}

fn validate_submission_configuration(
    configuration: &EvmSubmissionConfiguration,
) -> Result<(), String> {
    configuration.validate().map_err(|error| error.to_string())
}

/// Immutable admitted input for one structured EVM submission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "submission-request",
    version = "1",
    schema = "mfm.evm.submission_request"
)]
pub struct EvmSubmissionRequest {
    domain_activation_attestation: WalletNonceDomainActivationAttestation,
    tenant_scope_id: String,
    authenticated_principal_id: String,
    issuer_namespace_contract_ref: EvmWalletReference,
    caller_submission_token: String,
    route_generation_ref: EvmWalletReference,
    transaction_intent: EvmTransactionIntent,
    candidate_family: EvmCandidateFamily,
    observation_rounds: u8,
}

impl EvmSubmissionRequest {
    /// Combines principal-free configuration with fresh authenticated admission identity.
    pub fn from_authorized(
        configuration: EvmSubmissionConfiguration,
        tenant_scope_id: TenantScopeId,
        authenticated_principal_id: StableId,
        caller_submission_token: EvmCallerSubmissionToken,
    ) -> Result<Self, crate::WalletAuthorityContractError> {
        configuration.validate()?;
        let value = Self {
            domain_activation_attestation: configuration.domain_activation_attestation,
            tenant_scope_id: tenant_scope_id.as_str().to_owned(),
            authenticated_principal_id: authenticated_principal_id.as_str().to_owned(),
            issuer_namespace_contract_ref: configuration.issuer_namespace_contract_ref,
            caller_submission_token: caller_submission_token.into_string(),
            route_generation_ref: configuration.route_generation_ref,
            transaction_intent: configuration.transaction_intent,
            candidate_family: configuration.candidate_family,
            observation_rounds: configuration.observation_rounds,
        };
        value.validate()?;
        Ok(value)
    }

    /// Returns the immutable transaction intent.
    pub const fn transaction_intent(&self) -> &EvmTransactionIntent {
        &self.transaction_intent
    }

    /// Returns the admitted tenant scope identity.
    pub fn tenant_scope_id(&self) -> &str {
        &self.tenant_scope_id
    }

    /// Returns the stable authenticated principal identity.
    pub fn authenticated_principal_id(&self) -> &str {
        &self.authenticated_principal_id
    }

    /// Returns the immutable idempotency namespace contract.
    pub const fn issuer_namespace_contract_ref(&self) -> &EvmWalletReference {
        &self.issuer_namespace_contract_ref
    }

    /// Returns the bounded ordinary caller idempotency token.
    pub fn caller_submission_token(&self) -> &str {
        &self.caller_submission_token
    }

    /// Returns the declaration-ordered candidate family.
    pub const fn candidate_family(&self) -> &EvmCandidateFamily {
        &self.candidate_family
    }

    /// Returns the exact public domain activation attestation.
    pub const fn domain_activation_attestation(&self) -> &WalletNonceDomainActivationAttestation {
        &self.domain_activation_attestation
    }

    /// Returns the qualified route generation.
    pub const fn route_generation_ref(&self) -> &EvmWalletReference {
        &self.route_generation_ref
    }

    /// Returns one or two admitted observation rounds.
    pub const fn observation_rounds(&self) -> u8 {
        self.observation_rounds
    }

    /// Revalidates all derived identities and immutable object relationships.
    pub fn validate(&self) -> Result<(), crate::WalletAuthorityContractError> {
        validate_submission_semantics(
            &self.domain_activation_attestation,
            &self.issuer_namespace_contract_ref,
            &self.route_generation_ref,
            &self.transaction_intent,
            &self.candidate_family,
            self.observation_rounds,
        )?;
        let tenant = TenantScopeId::new(&self.tenant_scope_id)
            .map_err(|_| crate::WalletAuthorityContractError::Invalid("tenant_scope_id"))?;
        let principal = StableId::new(&self.authenticated_principal_id)
            .map_err(|_| crate::WalletAuthorityContractError::Invalid("principal_id"))?;
        let chain_binding = self
            .domain_activation_attestation
            .current_schema_record
            .chain_instance_attestation
            .binding()?;
        let lineage = derive_evm_chain_lineage_id(chain_binding.qualified_chain_instance_id())?;
        let sender = Address::from_str(self.transaction_intent.nonce_domain().sender())
            .map_err(|_| crate::WalletAuthorityContractError::Invalid("sender"))?;
        let domain = derive_wallet_nonce_domain(lineage, sender)?;
        let issuer = derive_authenticated_intent_issuer_id(
            &tenant,
            &principal,
            &self.issuer_namespace_contract_ref,
        )?;
        let expansion = crate::evm_submission_expansion_policy_ref()
            .map_err(|_| crate::WalletAuthorityContractError::Invalid("expansion_contract"))?;
        derive_submission_intent_id(&domain, &issuer, &self.caller_submission_token)?;
        derive_submission_semantics_digest(
            &self.transaction_intent,
            &self.candidate_family,
            self.observation_rounds,
            &expansion,
            &self.route_generation_ref,
            &self.domain_activation_attestation,
            &self.issuer_namespace_contract_ref,
        )?;
        Ok(())
    }
}

fn validate_submission_semantics(
    domain_activation_attestation: &WalletNonceDomainActivationAttestation,
    issuer_namespace_contract_ref: &EvmWalletReference,
    route_generation_ref: &EvmWalletReference,
    transaction_intent: &EvmTransactionIntent,
    candidate_family: &EvmCandidateFamily,
    observation_rounds: u8,
) -> Result<(), crate::WalletAuthorityContractError> {
    domain_activation_attestation.validate()?;
    transaction_intent.validate()?;
    candidate_family.validate(transaction_intent)?;
    if !(1..=EVM_WALLET_OBSERVATION_ROUND_LIMIT).contains(&observation_rounds) {
        return Err(crate::WalletAuthorityContractError::Invalid(
            "observation_rounds",
        ));
    }
    issuer_namespace_contract_ref
        .to_content_ref()
        .map_err(|_| crate::WalletAuthorityContractError::Invalid("issuer_namespace"))?;
    route_generation_ref
        .to_content_ref()
        .map_err(|_| crate::WalletAuthorityContractError::Invalid("route_generation"))?;
    let activation = &domain_activation_attestation.current_schema_record;
    let chain_binding = activation.chain_instance_attestation.binding()?;
    let lineage = derive_evm_chain_lineage_id(chain_binding.qualified_chain_instance_id())?;
    let sender = Address::from_str(transaction_intent.nonce_domain().sender())
        .map_err(|_| crate::WalletAuthorityContractError::Invalid("sender"))?;
    let domain = derive_wallet_nonce_domain(lineage, sender)?;
    if chain_binding != *transaction_intent.chain_instance()
        || domain != *transaction_intent.nonce_domain()
        || activation.wallet_nonce_domain != domain
        || activation.issuer_namespace_contract_ref != *issuer_namespace_contract_ref
    {
        return Err(crate::WalletAuthorityContractError::Invalid(
            "submission_domain",
        ));
    }
    Ok(())
}

/// Redaction-safe public output after permanent nonce completion.
///
/// The persisted [`CompletedWalletNonce`] remains the complete recovery closure. Only its
/// canonical terminal disposition crosses the public run-result boundary; provider attestations,
/// signed envelopes, and recovery preimages remain internal evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue, PublicOutputs)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "submission-output",
    version = "1",
    schema = "mfm.evm.submission_output"
)]
pub struct EvmSubmissionOutput {
    /// Canonical terminal execution disposition.
    pub execution_disposition: crate::ExecutionDisposition,
}

/// Abstract authored submission state replaced by [`EvmSubmissionExpansion`].
pub struct StructuredSubmitEvmTransactionState;

impl State for StructuredSubmitEvmTransactionState {
    type Input = EvmSubmissionRequest;
    type Output = EvmSubmissionOutput;
    type Failure = EvmSubmissionFailure;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = RequiresCapability<EvmSubmissionExpansion>;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        stable("mfm.evm.state/submit-transaction")
    }
}

/// Pure registered lowering for the abstract submission boundary.
pub enum EvmSubmissionExpansion {}

impl CapabilityExpansion for EvmSubmissionExpansion {
    fn recipe() -> mfm_program::Result<mfm_spec::structured::AuthoredStructuredProgram> {
        crate::submission_expansion::evm_submission_recipe()
    }
}

/// Authors the one abstract EVM submission entry program whose sole state is
/// deterministically replaced by [`EvmSubmissionExpansion`] at certification.
pub fn structured_evm_submission_entry_program(
    operation_id: StableId,
    scope_id: StableId,
) -> mfm_program::Result<mfm_spec::structured::AuthoredStructuredProgram> {
    crate::submission_expansion::evm_submission_entry_program(operation_id, scope_id)
}

/// Exact request for `eth_getTransactionCount(sender, "pending")`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "pending-nonce-request",
    version = "1",
    schema = "mfm.evm.pending_nonce_request"
)]
pub struct EvmPendingNonceRequest {
    /// Exact nonce domain.
    pub nonce_domain: WalletNonceDomain,
    /// Exact qualified route generation.
    pub route_generation_ref: EvmWalletReference,
}

/// Exact request for one activated transaction lookup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-lookup-request",
    version = "1",
    schema = "mfm.evm.transaction_lookup_request"
)]
pub struct EvmTransactionLookupRequest {
    /// Exact qualified route generation.
    pub route_generation_ref: EvmWalletReference,
    /// Exact activated transaction hash.
    pub transaction_hash: String,
}

/// Bounded transaction lookup observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-lookup-observation",
    version = "1",
    schema = "mfm.evm.transaction_lookup_observation"
)]
pub enum EvmTransactionLookupObservation {
    /// The candidate is not currently known by this route.
    Missing,
    /// The exact candidate is known, optionally in a canonical block.
    Found {
        /// Exact transaction hash.
        transaction_hash: String,
        /// Inclusion block number when mined.
        block_number: Option<String>,
        /// Inclusion block hash when mined.
        block_hash: Option<String>,
    },
}

/// Exact request for one activated receipt lookup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "receipt-lookup-request",
    version = "1",
    schema = "mfm.evm.receipt_lookup_request"
)]
pub struct EvmReceiptLookupRequest {
    /// Exact qualified route generation.
    pub route_generation_ref: EvmWalletReference,
    /// Exact activated transaction hash.
    pub transaction_hash: String,
}

/// Bounded receipt lookup observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "receipt-lookup-observation",
    version = "1",
    schema = "mfm.evm.receipt_lookup_observation"
)]
pub enum EvmReceiptLookupObservation {
    /// The receipt is not currently known.
    Missing,
    /// The exact candidate has a mined receipt.
    Found {
        /// Exact transaction hash.
        transaction_hash: String,
        /// Canonical inclusion block number.
        block_number: String,
        /// Canonical inclusion block hash.
        block_hash: String,
        /// EVM status: one for success and zero for revert.
        status: u8,
    },
}

/// Exact finalized-head observation request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "finalized-head-request",
    version = "1",
    schema = "mfm.evm.finalized_head_request"
)]
pub struct EvmFinalizedHeadRequest {
    /// Exact qualified route generation.
    pub route_generation_ref: EvmWalletReference,
}

/// Exact finalized block identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "finalized-head-observation",
    version = "1",
    schema = "mfm.evm.finalized_head_observation"
)]
pub struct EvmFinalizedHeadObservation {
    /// Canonical finalized block number.
    pub block_number: String,
    /// Canonical finalized block hash.
    pub block_hash: String,
}

/// Exact canonical inclusion-block request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "inclusion-block-request",
    version = "1",
    schema = "mfm.evm.inclusion_block_request"
)]
pub struct EvmInclusionBlockRequest {
    /// Exact qualified route generation.
    pub route_generation_ref: EvmWalletReference,
    /// Exact inclusion block number.
    pub block_number: String,
}

/// Exact canonical inclusion-block observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "inclusion-block-observation",
    version = "1",
    schema = "mfm.evm.inclusion_block_observation"
)]
pub struct EvmInclusionBlockObservation {
    /// Canonical inclusion block number.
    pub block_number: String,
    /// Canonical inclusion block hash.
    pub block_hash: String,
}

/// Canonical unsigned candidate descriptor retained without bearer bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "unsigned-wallet-candidate",
    version = "1",
    schema = "mfm.evm.unsigned_wallet_candidate"
)]
pub struct UnsignedWalletCandidate {
    /// Exact transaction intent.
    pub transaction_intent: EvmTransactionIntent,
    /// Permanent reservation key.
    pub semantic_reservation_key: EvmNonceReservationKey,
    /// Permanently allocated nonce.
    pub nonce: u64,
    /// Declaration-ordered candidate ordinal.
    pub candidate_ordinal: u16,
    /// Exact fee candidate.
    pub fee: EvmWalletFeeCandidate,
    /// Alloy-derived signing digest.
    pub unsigned_candidate_digest: String,
}

/// Exact signer-attestation request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "attest-candidate-identity-request",
    version = "1",
    schema = "mfm.evm.attest_candidate_identity_request"
)]
pub struct AttestCandidateIdentityRequest {
    /// Exact unsigned candidate.
    pub candidate: UnsignedWalletCandidate,
}

/// Exact activated candidate broadcast request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "broadcast-exact-candidate-request",
    version = "1",
    schema = "mfm.evm.broadcast_exact_candidate_request"
)]
pub struct BroadcastExactCandidateRequest {
    /// Exact unsigned candidate reconstructed for signing.
    pub unsigned_candidate: UnsignedWalletCandidate,
    /// Exact permanent activated candidate proof.
    pub active_candidate: ActiveWalletCandidate,
    /// Exact qualified route generation.
    pub route_generation_ref: EvmWalletReference,
}

/// Secret-free proof of one exact single broadcast attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "submitted-candidate-proof",
    version = "1",
    schema = "mfm.evm.submitted_candidate_proof"
)]
pub struct SubmittedCandidateProof {
    /// Declaration-ordered candidate ordinal.
    pub candidate_ordinal: u16,
    /// Exact unsigned digest.
    pub unsigned_candidate_digest: String,
    /// Exact deterministic transaction hash.
    pub transaction_hash: String,
    /// Stable semantic signer identity.
    pub semantic_signer_id: String,
    /// Qualified physical signer generation.
    pub signer_generation_ref: EvmWalletReference,
    /// Deterministic signing contract.
    pub signing_contract_ref: EvmWalletReference,
    /// Exact submission contract.
    pub submission_contract_ref: EvmWalletReference,
}

/// Monotonic public head for the composite signer-and-route broadcast lineage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "broadcast-lineage-head",
    version = "1",
    schema = "mfm.evm.broadcast_lineage_head"
)]
pub struct BroadcastLineageHead {
    /// Stable semantic broadcast lineage.
    pub lineage_id: String,
    /// Monotonic generation.
    pub generation: u64,
    /// Public lineage head certificate.
    pub public_lineage_head_ref: EvmWalletReference,
}

impl BroadcastLineageHead {
    /// Revalidates the stable lineage, monotonic generation, and public head reference.
    pub fn validate(&self) -> Result<(), crate::WalletAuthorityContractError> {
        StableId::new(&self.lineage_id)
            .map_err(|_| crate::WalletAuthorityContractError::Invalid("broadcast_lineage_id"))?;
        if self.generation == 0 {
            return Err(crate::WalletAuthorityContractError::Invalid(
                "broadcast_generation",
            ));
        }
        self.public_lineage_head_ref
            .to_content_ref()
            .map_err(|_| crate::WalletAuthorityContractError::Invalid("broadcast_lineage_head"))?;
        Ok(())
    }
}

/// One candidate's complete bounded observation accumulator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "candidate-transaction-observation",
    version = "1",
    schema = "mfm.evm.candidate_transaction_observation"
)]
pub struct CandidateTransactionObservation {
    /// Exact transaction lookup result.
    pub transaction: Option<EvmTransactionLookupObservation>,
    /// Exact receipt lookup result.
    pub receipt: Option<EvmReceiptLookupObservation>,
    /// Exact finalized head when terminal verification was attempted.
    pub finalized_head: Option<EvmFinalizedHeadObservation>,
    /// Exact canonical inclusion block when terminal verification was attempted.
    pub inclusion_block: Option<EvmInclusionBlockObservation>,
}

/// Semantic signer inventory entry used by attestation and broadcast adapters.
pub enum EvmCandidateSigner {}

/// Closed completion returned by the process-qualified semantic signer.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum EvmCandidateSignerCompletion {
    /// Candidate signing completed with an attested candidate.
    Returned(AttestedWalletCandidate),
    /// The signer was operationally unavailable.
    SafeFailure(EvmSubmissionFailure),
    /// The candidate or signer binding violated its integrity contract.
    IntegrityFault,
}

impl BoundedComponentContract for EvmCandidateSigner {
    type Request = AttestCandidateIdentityRequest;
    type Completion = EvmCandidateSignerCompletion;
}

impl SignerContract for EvmCandidateSigner {}

impl RuntimeSigner for EvmCandidateSigner {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        StructuredLiveComponentContract::new(
            StructuredComponentKind::Signer,
            stable("mfm.evm.signer/candidate-identity")?,
            Vec::new(),
        )
        .map_err(Into::into)
    }
}

/// Composite route/signer generation lineage for refreshable broadcast.
pub enum EvmBroadcastResource {}

impl BoundedComponentContract for EvmBroadcastResource {
    type Request = ();
    type Completion = ();
}

impl ResourceAuthorityContract for EvmBroadcastResource {}

impl RuntimeResourceAuthority for EvmBroadcastResource {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        StructuredLiveComponentContract::new(
            StructuredComponentKind::Resource,
            stable("mfm.evm.resource/broadcast-lineage")?,
            Vec::new(),
        )
        .map_err(Into::into)
    }
}

macro_rules! evm_read_capability {
    ($name:ident, $request:ty, $returned:ty, $capability:literal, $adapter_fn:ident) => {
        #[doc = concat!("Typed EVM Read capability for `", $capability, "`.")]
        pub enum $name {}

        impl ReadCapabilityContract for $name {
            type Request = $request;
            type Returned = $returned;
            type SafeFailure = EvmSubmissionFailure;
        }

        impl RuntimeReadCapability for $name {
            fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
                StructuredLiveComponentContract::new_read_capability(
                    stable($capability)?,
                    structured_value_contract_ref::<$request>()?,
                    structured_value_contract_ref::<$returned>()?,
                    structured_value_contract_ref::<EvmSubmissionFailure>()?,
                    $adapter_fn()?.content_ref()?,
                )
                .map_err(Into::into)
            }
        }
    };
}

evm_read_capability!(
    EvmPendingNonceCapability,
    EvmPendingNonceRequest,
    ObservedPendingNonceFloor,
    "mfm.evm.capability/pending-nonce",
    evm_pending_nonce_adapter_contract
);
evm_read_capability!(
    EvmTransactionLookupCapability,
    EvmTransactionLookupRequest,
    EvmTransactionLookupObservation,
    "mfm.evm.capability/transaction-lookup",
    evm_transaction_lookup_adapter_contract
);
evm_read_capability!(
    EvmReceiptLookupCapability,
    EvmReceiptLookupRequest,
    EvmReceiptLookupObservation,
    "mfm.evm.capability/receipt-lookup",
    evm_receipt_lookup_adapter_contract
);
evm_read_capability!(
    EvmFinalizedHeadCapability,
    EvmFinalizedHeadRequest,
    EvmFinalizedHeadObservation,
    "mfm.evm.capability/finalized-head",
    evm_finalized_head_adapter_contract
);
evm_read_capability!(
    EvmInclusionBlockCapability,
    EvmInclusionBlockRequest,
    EvmInclusionBlockObservation,
    "mfm.evm.capability/inclusion-block",
    evm_inclusion_block_adapter_contract
);
evm_read_capability!(
    AttestCandidateIdentityCapability,
    AttestCandidateIdentityRequest,
    AttestedWalletCandidate,
    "mfm.evm.capability/attest-candidate-identity",
    evm_signer_attestation_adapter_contract
);

/// Exact deterministic transaction hash one broadcast is keyed on.
///
/// The hash is derived from the signed candidate bytes before broadcast, so it
/// names the transaction the chain will accept and is a pure projection of the
/// committed request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "broadcast-entry-key",
    version = "1",
    schema = "mfm.evm.broadcast_entry_key"
)]
pub struct EvmBroadcastEntryKey {
    /// Deterministically derived EVM transaction hash.
    pub transaction_hash: String,
}

impl EntryKeyed for BroadcastExactCandidateRequest {
    type EntryKey = EvmBroadcastEntryKey;

    fn entry_key(&self) -> Self::EntryKey {
        EvmBroadcastEntryKey {
            transaction_hash: self
                .active_candidate
                .attested_candidate
                .transaction_hash
                .clone(),
        }
    }
}

/// Typed refreshable, absorbing exact-candidate broadcast capability.
///
/// **What absorbs:** the account nonce. The signed candidate is deterministic,
/// so a repeat submits byte-identical bytes under the same nonce; a chain
/// accepts that transaction at most once, and a node that already holds it
/// reports the same hash rather than admitting a second one.
///
/// **`Returned` is a function of chain post-state, not of one exchange.**
/// [`SubmittedCandidateProof`] names the transaction the chain holds, which a
/// re-invoked adapter recovers by reading the chain rather than by remembering
/// what one submission replied. Nothing in the kernel checks this; a proof
/// synthesized from the attested candidate without a chain read would be a
/// well-typed claim about an event that never happened.
pub enum BroadcastExactCandidateCapability {}

impl EffectCapabilityContract for BroadcastExactCandidateCapability {
    type Request = BroadcastExactCandidateRequest;
    type Returned = SubmittedCandidateProof;
    type SafeFailure = EvmSubmissionFailure;
    type Refresh = Refreshable<BroadcastLineageHead>;
    type Entry = EntryAbsorbing<3>;
}

impl RuntimeEffectCapability for BroadcastExactCandidateCapability {
    type RefreshBinding = RefreshableBinding<EvmBroadcastResource>;
    type EntryBinding = EntryAbsorbingBinding<3>;

    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        StructuredLiveComponentContract::new_effect_capability_refreshable(
            stable("mfm.evm.capability/broadcast-exact-candidate")?,
            structured_value_contract_ref::<BroadcastExactCandidateRequest>()?,
            structured_value_contract_ref::<SubmittedCandidateProof>()?,
            structured_value_contract_ref::<EvmSubmissionFailure>()?,
            structured_value_contract_ref::<BroadcastLineageHead>()?,
            EvmBroadcastResource::contract()?.content_ref()?,
            <EntryAbsorbingBinding<3> as mfm_program::structured::RuntimeEffectEntryBinding<
                EntryAbsorbing<3>,
                BroadcastExactCandidateRequest,
            >>::contract()?,
            evm_broadcast_adapter_contract()?.content_ref()?,
        )
        .map_err(Into::into)
    }
}

/// Returns the semantic pending-nonce adapter contract.
pub fn evm_pending_nonce_adapter_contract() -> mfm_program::Result<StructuredLiveComponentContract>
{
    leaf_adapter("mfm.evm.adapter/pending-nonce", Vec::new())
}

/// Returns the semantic transaction-lookup adapter contract.
pub fn evm_transaction_lookup_adapter_contract(
) -> mfm_program::Result<StructuredLiveComponentContract> {
    leaf_adapter("mfm.evm.adapter/transaction-lookup", Vec::new())
}

/// Returns the semantic receipt-lookup adapter contract.
pub fn evm_receipt_lookup_adapter_contract() -> mfm_program::Result<StructuredLiveComponentContract>
{
    leaf_adapter("mfm.evm.adapter/receipt-lookup", Vec::new())
}

/// Returns the semantic finalized-head adapter contract.
pub fn evm_finalized_head_adapter_contract() -> mfm_program::Result<StructuredLiveComponentContract>
{
    leaf_adapter("mfm.evm.adapter/finalized-head", Vec::new())
}

/// Returns the semantic inclusion-block adapter contract.
pub fn evm_inclusion_block_adapter_contract() -> mfm_program::Result<StructuredLiveComponentContract>
{
    leaf_adapter("mfm.evm.adapter/inclusion-block", Vec::new())
}

/// Returns the semantic signer-attestation adapter contract.
pub fn evm_signer_attestation_adapter_contract(
) -> mfm_program::Result<StructuredLiveComponentContract> {
    leaf_adapter(
        "mfm.evm.adapter/signer-attestation",
        vec![StructuredComponentDependency {
            component_kind: StructuredComponentKind::Signer,
            contract_ref: EvmCandidateSigner::contract()?.content_ref()?,
        }],
    )
}

/// Returns the semantic composite broadcast adapter contract.
pub fn evm_broadcast_adapter_contract() -> mfm_program::Result<StructuredLiveComponentContract> {
    leaf_adapter(
        "mfm.evm.adapter/broadcast-exact-candidate",
        vec![
            StructuredComponentDependency {
                component_kind: StructuredComponentKind::Signer,
                contract_ref: EvmCandidateSigner::contract()?.content_ref()?,
            },
            StructuredComponentDependency {
                component_kind: StructuredComponentKind::Resource,
                contract_ref: EvmBroadcastResource::contract()?.content_ref()?,
            },
        ],
    )
}

fn leaf_adapter(
    id: &str,
    dependencies: Vec<StructuredComponentDependency>,
) -> mfm_program::Result<StructuredLiveComponentContract> {
    StructuredLiveComponentContract::new(
        StructuredComponentKind::Adapter,
        stable(id)?,
        dependencies,
    )
    .map_err(Into::into)
}

macro_rules! pure_state {
    ($name:ident, $input:ty, $output:ty, $id:literal) => {
        pub(crate) struct $name;

        impl State for $name {
            type Input = $input;
            type Output = $output;
            type Failure = Never;
            type Request = ();
            type Returned = ();
            type SafeFailure = ();
            type Execution = Pure;
            type SafeFailureDisposition = SafeFailureNotApplicable;
            type Capability = Direct;

            fn semantic_state_id() -> mfm_program::Result<StableId> {
                stable($id)
            }
        }
    };
}

macro_rules! fallible_pure_state {
    ($name:ident, $input:ty, $output:ty, $id:literal) => {
        pub(crate) struct $name;

        impl State for $name {
            type Input = $input;
            type Output = $output;
            type Failure = EvmSubmissionFailure;
            type Request = ();
            type Returned = ();
            type SafeFailure = ();
            type Execution = Pure;
            type SafeFailureDisposition = SafeFailureNotApplicable;
            type Capability = Direct;

            fn semantic_state_id() -> mfm_program::Result<StableId> {
                stable($id)
            }
        }
    };
}

macro_rules! read_state {
    ($name:ident, $input:ty, $output:ty, $request:ty, $returned:ty, $capability:ty, $id:literal) => {
        pub(crate) struct $name;

        impl State for $name {
            type Input = $input;
            type Output = $output;
            type Failure = EvmSubmissionFailure;
            type Request = $request;
            type Returned = $returned;
            type SafeFailure = EvmSubmissionFailure;
            type Execution = Read<$capability>;
            type SafeFailureDisposition = SafeFailureMayFail;
            type Capability = Direct;

            fn semantic_state_id() -> mfm_program::Result<StableId> {
                stable($id)
            }
        }
    };
}

macro_rules! reconciling_pure_state {
    ($name:ident, $input:ty, $output:ty, $id:literal) => {
        pub(crate) struct $name;

        impl State for $name {
            type Input = $input;
            type Output = $output;
            type Failure = PendingEvmSubmissionFailure;
            type Request = ();
            type Returned = ();
            type SafeFailure = ();
            type Execution = Pure;
            type SafeFailureDisposition = SafeFailureNotApplicable;
            type Capability = Direct;

            fn semantic_state_id() -> mfm_program::Result<StableId> {
                stable($id)
            }
        }
    };
}

macro_rules! reconciling_read_state {
    ($name:ident, $input:ty, $output:ty, $request:ty, $returned:ty, $capability:ty, $id:literal) => {
        pub(crate) struct $name;

        impl State for $name {
            type Input = $input;
            type Output = $output;
            type Failure = PendingEvmSubmissionFailure;
            type Request = $request;
            type Returned = $returned;
            type SafeFailure = EvmSubmissionFailure;
            type Execution = Read<$capability>;
            type SafeFailureDisposition = SafeFailureMayFail;
            type Capability = Direct;

            fn semantic_state_id() -> mfm_program::Result<StableId> {
                stable($id)
            }
        }
    };
}

macro_rules! reconciling_effect_state {
    ($name:ident, $input:ty, $output:ty, $request:ty, $returned:ty, $capability:ty, $id:literal) => {
        pub(crate) struct $name;

        impl State for $name {
            type Input = $input;
            type Output = $output;
            type Failure = PendingEvmSubmissionFailure;
            type Request = $request;
            type Returned = $returned;
            type SafeFailure = EvmSubmissionFailure;
            type Execution = Effect<$capability>;
            type SafeFailureDisposition = SafeFailureMayFail;
            type Capability = Direct;

            fn semantic_state_id() -> mfm_program::Result<StableId> {
                stable($id)
            }
        }
    };
}

/// Chain- and intent-derived submission context before activation qualification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "derived-submission-domain",
    version = "1",
    schema = "mfm.evm.derived_submission_domain"
)]
pub(crate) struct DerivedSubmissionDomain {
    pub(crate) request: EvmSubmissionRequest,
    pub(crate) qualified_chain_instance_id: QualifiedChainInstanceId,
    pub(crate) nonce_domain: WalletNonceDomain,
}

/// Stable authenticated-intent context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "intent-bound-submission",
    version = "1",
    schema = "mfm.evm.intent_bound_submission"
)]
pub(crate) struct IntentBoundSubmission {
    pub(crate) derived: DerivedSubmissionDomain,
    pub(crate) issuer_id: AuthenticatedIntentIssuerId,
    pub(crate) submission_intent_id: SubmissionIntentId,
    pub(crate) semantics_digest: SubmissionSemanticsDigest,
}

/// Complete stable request material before a wallet status read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "prepared-wallet-submission",
    version = "1",
    schema = "mfm.evm.prepared_wallet_submission"
)]
pub(crate) struct PreparedWalletSubmission {
    pub(crate) intent: IntentBoundSubmission,
    pub(crate) reservation_key: EvmNonceReservationKey,
}

/// Exact status snapshot against which a definite failure is reconciled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-status-baseline",
    version = "1",
    schema = "mfm.evm.wallet_status_baseline"
)]
pub(crate) enum WalletStatusBaseline {
    Absent,
    Reserved {
        resource_head_ref: EvmWalletReference,
        canonical_status_digest: String,
    },
}

/// Definite post-status failure awaiting one authoritative status snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "failure-reconciliation-request",
    version = "1",
    schema = "mfm.evm.failure_reconciliation_request"
)]
pub(crate) struct FailureReconciliationRequest {
    pub(crate) prepared: PreparedWalletSubmission,
    pub(crate) baseline: WalletStatusBaseline,
    pub(crate) original_failure: EvmSubmissionFailure,
}

/// Child-operation failure requiring propagation or one fresh status read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "pending-submission-failure",
    version = "1",
    schema = "mfm.evm.pending_submission_failure"
)]
// Certified state schemas keep their semantic payloads directly nested; a
// heap-indirection field would be a different current-schema contract.
#[allow(clippy::large_enum_variant)]
pub(crate) enum PendingEvmSubmissionFailure {
    Direct {
        failure: EvmSubmissionFailure,
    },
    Reconcile {
        request: FailureReconciliationRequest,
    },
}

/// Exact reservation response that requires a non-absent status read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "post-reserve-prepared-submission",
    version = "1",
    schema = "mfm.evm.post_reserve_prepared_submission"
)]
pub(crate) struct PostReservePreparedSubmission {
    pub(crate) prepared: PreparedWalletSubmission,
    pub(crate) reservation: ReservedWalletNonce,
}

/// Exact incomplete same-intent status bound to the current run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "submission-work",
    version = "2",
    schema = "mfm.evm.submission_work"
)]
pub(crate) struct SubmissionWork {
    pub(crate) prepared: PreparedWalletSubmission,
    pub(crate) reservation: ReservedWalletNonce,
    pub(crate) activated_candidates: Vec<ActiveWalletCandidate>,
    pub(crate) current_candidate: Option<ActiveWalletCandidate>,
    /// Next family ordinal to visit in certified order (recovery starts at 0).
    pub(crate) next_candidate_ordinal: u16,
    /// Exclusive upper bound of activated ordinals observed in this recovery walk.
    ///
    /// Replacement admission requires `observed_prefix_len == activated_candidates.len()`
    /// so every retained activated candidate has independent producer observation
    /// evidence before a later ordinal may activate (EVM-03/EVM-04).
    pub(crate) observed_prefix_len: u16,
    pub(crate) status_baseline: WalletStatusBaseline,
}

/// Route selected for one candidate attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm",
    name = "candidate-slot-route",
    version = "1",
    schema = "mfm.evm.candidate_slot_route"
)]
pub(crate) enum CandidateSlotRoute {
    Activate,
    ObserveRetained,
    Exhausted,
}

/// Closed status decision produced from one committed status observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-status-decision",
    version = "1",
    schema = "mfm.evm.wallet_status_decision"
)]
// This certified closed sum intentionally preserves direct schema payloads.
#[allow(clippy::large_enum_variant)]
pub(crate) enum WalletStatusDecision {
    Completed { completion: CompletedWalletNonce },
    Busy { failure: EvmSubmissionFailure },
    Reserved { work: SubmissionWork },
    Absent,
}

/// Submission context after pending-floor qualification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "qualified-pending-submission",
    version = "1",
    schema = "mfm.evm.qualified_pending_submission"
)]
pub(crate) struct QualifiedPendingSubmission {
    pub(crate) prepared: PreparedWalletSubmission,
    pub(crate) floor: QualifiedPendingNonceFloor,
}

/// Pending observation paired with its exact prepared submission context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "observed-pending-submission",
    version = "1",
    schema = "mfm.evm.observed_pending_submission"
)]
pub(crate) struct ObservedPendingSubmission {
    pub(crate) prepared: PreparedWalletSubmission,
    pub(crate) observed: ObservedPendingNonceFloor,
}

/// Closed statically bounded candidate-slot decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "candidate-slot-decision",
    version = "1",
    schema = "mfm.evm.candidate_slot_decision"
)]
// This certified closed sum intentionally preserves direct schema payloads.
#[allow(clippy::large_enum_variant)]
pub(crate) enum CandidateSlotDecision {
    Execute,
    Skip,
}

/// Flat bounded submission state carried between authored candidate slots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "submission-progress",
    version = "1",
    schema = "mfm.evm.submission_progress"
)]
pub(crate) struct SubmissionProgress {
    pub(crate) work: Option<SubmissionWork>,
    pub(crate) completion: Option<CompletedWalletNonce>,
    pub(crate) failure: Option<EvmSubmissionFailure>,
}

/// Closed final projection selected after all statically authored slots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "submission-terminal-decision",
    version = "1",
    schema = "mfm.evm.submission_terminal_decision"
)]
// This certified closed sum intentionally preserves direct schema payloads.
#[allow(clippy::large_enum_variant)]
pub(crate) enum SubmissionTerminalDecision {
    Completed { completion: CompletedWalletNonce },
    Exhausted { failure: EvmSubmissionFailure },
    Failed { failure: EvmSubmissionFailure },
}

/// Exact unsigned candidate plus current reservation context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "candidate-work",
    version = "1",
    schema = "mfm.evm.candidate_work"
)]
pub(crate) struct CandidateWork {
    pub(crate) work: SubmissionWork,
    pub(crate) unsigned_candidate: UnsignedWalletCandidate,
    pub(crate) attested_candidate: Option<AttestedWalletCandidate>,
}

/// Exact attested candidate with one pure progression permit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "permitted-candidate-work",
    version = "1",
    schema = "mfm.evm.permitted_candidate_work"
)]
pub(crate) struct PermittedCandidateWork {
    pub(crate) candidate: CandidateWork,
    pub(crate) activation_permit: crate::CandidateActivationPermit,
}

/// Complete immutable candidate activation request and its source context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "prepared-candidate-activation",
    version = "1",
    schema = "mfm.evm.prepared_candidate_activation"
)]
pub(crate) struct PreparedCandidateActivation {
    pub(crate) candidate: CandidateWork,
    pub(crate) activation_request: ActivateEvmCandidateRequest,
}

/// Activated and broadcast-ready candidate context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "active-candidate-work",
    version = "1",
    schema = "mfm.evm.active_candidate_work"
)]
pub(crate) struct ActiveCandidateWork {
    pub(crate) candidate: CandidateWork,
    pub(crate) active_candidate: ActiveWalletCandidate,
}

/// Closed result of serialized candidate activation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "candidate-activation-decision",
    version = "1",
    schema = "mfm.evm.candidate_activation_decision"
)]
// This certified closed sum intentionally preserves direct schema payloads.
#[allow(clippy::large_enum_variant)]
pub(crate) enum CandidateActivationDecision {
    Reconcile,
    Activated { active: ActiveCandidateWork },
}

/// Common child-operation result after one candidate attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "candidate-resolution",
    version = "1",
    schema = "mfm.evm.candidate_resolution"
)]
// This certified closed sum intentionally preserves direct schema payloads.
#[allow(clippy::large_enum_variant)]
pub(crate) enum CandidateResolution {
    Resume { work: SubmissionWork },
    Completed { completion: CompletedWalletNonce },
}

/// Broadcast candidate plus explicit observation accumulator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "candidate-observation-work",
    version = "1",
    schema = "mfm.evm.candidate_observation_work"
)]
pub(crate) struct CandidateObservationWork {
    pub(crate) active: ActiveCandidateWork,
    pub(crate) submitted: SubmittedCandidateProof,
    pub(crate) next_round: u8,
    pub(crate) observation: CandidateTransactionObservation,
}

/// Closed gate for one of two statically authored observation rounds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "observation-round-decision",
    version = "1",
    schema = "mfm.evm.observation_round_decision"
)]
pub(crate) enum ObservationRoundDecision {
    Observe,
    Skip,
}

/// Closed terminal-evidence gate after bounded candidate observations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "terminal-evidence-decision",
    version = "1",
    schema = "mfm.evm.terminal_evidence_decision"
)]
pub(crate) enum TerminalEvidenceDecision {
    Reconcile,
    ObserveFinality,
}

/// Finality and inclusion observations ready for pure terminal verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "terminal-evidence-work",
    version = "1",
    schema = "mfm.evm.terminal_evidence_work"
)]
pub(crate) struct TerminalEvidenceWork {
    pub(crate) candidate: CandidateObservationWork,
}

/// Canonical completion request plus current candidate context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "completion-work",
    version = "1",
    schema = "mfm.evm.completion_work"
)]
pub(crate) struct CompletionWork {
    pub(crate) request: CompleteEvmNonceRequest,
    pub(crate) submission_work: SubmissionWork,
}

/// Closed projection from permanent completion into root success or failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "completed-projection",
    version = "1",
    schema = "mfm.evm.completed_projection"
)]
// This certified closed sum intentionally preserves direct schema payloads.
#[allow(clippy::large_enum_variant)]
pub(crate) enum CompletedProjection {
    Success { output: EvmSubmissionOutput },
    Failure { failure: EvmSubmissionFailure },
}

impl mfm_program::structured::ClosedSum for WalletStatusDecision {}
impl mfm_program::structured::ClosedSum for PendingEvmSubmissionFailure {}
impl mfm_program::structured::ClosedSum for CandidateSlotRoute {}
impl mfm_program::structured::ClosedSum for CandidateSlotDecision {}
impl mfm_program::structured::ClosedSum for SubmissionTerminalDecision {}
impl mfm_program::structured::ClosedSum for CandidateActivationDecision {}
impl mfm_program::structured::ClosedSum for CandidateResolution {}
impl mfm_program::structured::ClosedSum for ObservationRoundDecision {}
impl mfm_program::structured::ClosedSum for TerminalEvidenceDecision {}
impl mfm_program::structured::ClosedSum for CompletedProjection {}

pure_state!(
    DeriveTransactionIntentState,
    EvmSubmissionRequest,
    DerivedSubmissionDomain,
    "mfm.evm.state/derive-transaction-intent"
);
fallible_pure_state!(
    DeriveSubmissionIntentIdState,
    DerivedSubmissionDomain,
    IntentBoundSubmission,
    "mfm.evm.state/derive-submission-intent-id"
);
fallible_pure_state!(
    DeriveEvmNonceReservationKeyState,
    IntentBoundSubmission,
    PreparedWalletSubmission,
    "mfm.evm.state/derive-nonce-reservation-key"
);
reconciling_read_state!(
    ReadWalletNonceStatusState,
    PreparedWalletSubmission,
    WalletStatusDecision,
    ReadEvmWalletNonceStatusRequest,
    WalletNonceStatus,
    ReadWalletNonceStatusCapability,
    "mfm.evm.state/read-wallet-nonce-status"
);
reconciling_read_state!(
    ReadPostReserveWalletNonceStatusState,
    PostReservePreparedSubmission,
    WalletStatusDecision,
    ReadEvmWalletNonceStatusRequest,
    WalletNonceStatus,
    ReadWalletNonceStatusCapability,
    "mfm.evm.state/read-post-reserve-wallet-nonce-status"
);
read_state!(
    ReadReservationStatusAfterFailureState,
    FailureReconciliationRequest,
    WalletStatusDecision,
    ReadEvmWalletNonceStatusRequest,
    WalletNonceStatus,
    ReadWalletNonceStatusCapability,
    "mfm.evm.state/read-reservation-status-after-failure"
);
read_state!(
    ReadCandidateStatusAfterFailureState,
    FailureReconciliationRequest,
    SubmissionProgress,
    ReadEvmWalletNonceStatusRequest,
    WalletNonceStatus,
    ReadWalletNonceStatusCapability,
    "mfm.evm.state/read-candidate-status-after-failure"
);
reconciling_read_state!(
    ReadCandidateWalletNonceStatusState,
    PreparedWalletSubmission,
    CandidateResolution,
    ReadEvmWalletNonceStatusRequest,
    WalletNonceStatus,
    ReadWalletNonceStatusCapability,
    "mfm.evm.state/read-candidate-wallet-nonce-status"
);
reconciling_read_state!(
    ReadObservedCandidateStatusState,
    CandidateObservationWork,
    CandidateResolution,
    ReadEvmWalletNonceStatusRequest,
    WalletNonceStatus,
    ReadWalletNonceStatusCapability,
    "mfm.evm.state/read-observed-candidate-status"
);
reconciling_read_state!(
    ReadExhaustionStatusState,
    FailureReconciliationRequest,
    CandidateResolution,
    ReadEvmWalletNonceStatusRequest,
    WalletNonceStatus,
    ReadWalletNonceStatusCapability,
    "mfm.evm.state/read-exhaustion-status"
);
reconciling_read_state!(
    ObservePendingNonceState,
    PreparedWalletSubmission,
    ObservedPendingSubmission,
    EvmPendingNonceRequest,
    ObservedPendingNonceFloor,
    EvmPendingNonceCapability,
    "mfm.evm.state/observe-pending-nonce"
);
reconciling_pure_state!(
    QualifyPendingNonceFloorState,
    ObservedPendingSubmission,
    QualifiedPendingSubmission,
    "mfm.evm.state/qualify-pending-nonce-floor"
);
reconciling_effect_state!(
    ReserveWalletNonceState,
    QualifiedPendingSubmission,
    PostReservePreparedSubmission,
    ReserveEvmNonceRequest,
    crate::ReserveWalletNonceResponse,
    ReserveWalletNonceCapability,
    "mfm.evm.state/reserve-wallet-nonce"
);
pure_state!(
    SelectCandidateSlotState,
    SubmissionProgress,
    CandidateSlotDecision,
    "mfm.evm.state/select-candidate-slot"
);
pure_state!(
    SelectCandidateAttemptRouteState,
    SubmissionWork,
    CandidateSlotRoute,
    "mfm.evm.state/select-candidate-attempt-route"
);
pure_state!(
    CollapseCandidateResolutionState,
    CandidateResolution,
    SubmissionProgress,
    "mfm.evm.state/collapse-candidate-resolution"
);
fallible_pure_state!(
    ExtractSubmissionWorkState,
    SubmissionProgress,
    SubmissionWork,
    "mfm.evm.state/extract-submission-work"
);
pure_state!(
    PrepareExhaustionReconciliationState,
    SubmissionWork,
    FailureReconciliationRequest,
    "mfm.evm.state/prepare-exhaustion-reconciliation"
);
pure_state!(
    CollapseWalletStatusState,
    WalletStatusDecision,
    SubmissionProgress,
    "mfm.evm.state/collapse-wallet-status"
);
reconciling_pure_state!(
    BuildUnsignedCandidateState,
    SubmissionWork,
    CandidateWork,
    "mfm.evm.state/build-unsigned-candidate"
);
reconciling_pure_state!(
    PrepareRetainedCandidateObservationState,
    SubmissionWork,
    CandidateObservationWork,
    "mfm.evm.state/prepare-retained-candidate-observation"
);
reconciling_read_state!(
    AttestCandidateIdentityState,
    CandidateWork,
    CandidateWork,
    AttestCandidateIdentityRequest,
    AttestedWalletCandidate,
    AttestCandidateIdentityCapability,
    "mfm.evm.state/attest-candidate-identity"
);
reconciling_pure_state!(
    DeriveCandidateActivationPermitState,
    CandidateWork,
    PermittedCandidateWork,
    "mfm.evm.state/derive-candidate-activation-permit"
);
reconciling_pure_state!(
    DeriveEvmCandidateOperationKeyState,
    PermittedCandidateWork,
    PreparedCandidateActivation,
    "mfm.evm.state/derive-candidate-operation-key"
);
reconciling_effect_state!(
    ActivateWalletCandidateState,
    PreparedCandidateActivation,
    CandidateActivationDecision,
    ActivateEvmCandidateRequest,
    ActivateCandidateResponse,
    ActivateWalletCandidateCapability,
    "mfm.evm.state/activate-wallet-candidate"
);
reconciling_effect_state!(
    BroadcastExactCandidateState,
    ActiveCandidateWork,
    CandidateObservationWork,
    BroadcastExactCandidateRequest,
    SubmittedCandidateProof,
    BroadcastExactCandidateCapability,
    "mfm.evm.state/broadcast-exact-candidate"
);
pure_state!(
    SelectObservationRoundState,
    CandidateObservationWork,
    ObservationRoundDecision,
    "mfm.evm.state/select-observation-round"
);
pure_state!(
    MarkActivationReconcileState,
    PreparedCandidateActivation,
    PreparedWalletSubmission,
    "mfm.evm.state/mark-activation-reconcile"
);
pure_state!(
    MarkCandidateCompletedState,
    CompletedWalletNonce,
    CandidateResolution,
    "mfm.evm.state/mark-candidate-completed"
);
reconciling_read_state!(
    ObserveActivatedTransactionState,
    CandidateObservationWork,
    CandidateObservationWork,
    EvmTransactionLookupRequest,
    EvmTransactionLookupObservation,
    EvmTransactionLookupCapability,
    "mfm.evm.state/observe-activated-transaction"
);
reconciling_read_state!(
    ObserveCandidateReceiptState,
    CandidateObservationWork,
    CandidateObservationWork,
    EvmReceiptLookupRequest,
    EvmReceiptLookupObservation,
    EvmReceiptLookupCapability,
    "mfm.evm.state/observe-candidate-receipt"
);
pure_state!(
    SelectTerminalEvidenceState,
    CandidateObservationWork,
    TerminalEvidenceDecision,
    "mfm.evm.state/select-terminal-evidence"
);
reconciling_read_state!(
    ObserveFinalizedHeadState,
    CandidateObservationWork,
    TerminalEvidenceWork,
    EvmFinalizedHeadRequest,
    EvmFinalizedHeadObservation,
    EvmFinalizedHeadCapability,
    "mfm.evm.state/observe-finalized-head"
);
reconciling_read_state!(
    ObserveCanonicalInclusionState,
    TerminalEvidenceWork,
    TerminalEvidenceWork,
    EvmInclusionBlockRequest,
    EvmInclusionBlockObservation,
    EvmInclusionBlockCapability,
    "mfm.evm.state/observe-canonical-inclusion"
);
reconciling_pure_state!(
    VerifyCanonicalInclusionState,
    TerminalEvidenceWork,
    CompletionWork,
    "mfm.evm.state/verify-canonical-inclusion"
);
reconciling_effect_state!(
    CompleteWalletNonceState,
    CompletionWork,
    CompletedWalletNonce,
    CompleteEvmNonceRequest,
    crate::CompleteWalletNonceResponse,
    CompleteWalletNonceCapability,
    "mfm.evm.state/complete-wallet-nonce"
);
pure_state!(
    ProjectCompletedWalletDispositionState,
    CompletedWalletNonce,
    CompletedProjection,
    "mfm.evm.state/project-completed-wallet-disposition"
);
pure_state!(
    SelectSubmissionTerminalState,
    SubmissionProgress,
    SubmissionTerminalDecision,
    "mfm.evm.state/select-submission-terminal"
);

pub(crate) fn stable(value: &str) -> mfm_program::Result<StableId> {
    StableId::new(value).map_err(|error| mfm_program::ProgramError::Authoring(error.to_string()))
}

pub(crate) fn validate_transaction_hash(value: &str) -> bool {
    B256::from_str(value).is_ok_and(|hash| hash != B256::ZERO && value == format!("{hash:#x}"))
}

pub(crate) fn validate_quantity(value: &str) -> bool {
    U256::from_str(value).is_ok_and(|quantity| value == quantity.to_string())
}
use mfm_values::CanonicalJsonPersistedSchema;
