//! Canonical EVM wallet-domain identities and the narrow nonce-authority port.

use std::collections::BTreeSet;
use std::str::FromStr;

use alloy_primitives::{Address, B256, U256};
use mfm_canonical::limits::{MAX_COMPLETION_RECOVERY_BYTES, MAX_PROVIDER_PROOF_BYTES};
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::{
    AccessFaultCode, ComponentFuture, EffectAdapterCompletion, EffectCapabilityContract,
    EffectContractCompletion, ReadAdapterCompletion, ReadCapabilityContract, Refreshable,
    ResourceAuthorityContract,
};
use mfm_certify::structured::CertifiedAccessAuthorization;
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, RunId, StableId, TenantScopeId};
use mfm_journal::structured::{domain_content_digest, LexicalValueRef, RecordRef};
use mfm_program::structured::{
    RefreshableBinding, RuntimeEffectCapability, RuntimeReadCapability, RuntimeResourceAuthority,
};
use mfm_program_derive::MfmValue;
use mfm_spec::structured::{
    structured_value_contract_ref, StructuredComponentDependency, StructuredComponentKind,
    StructuredLiveComponentContract,
};
use serde::{Deserialize, Serialize};

use crate::{
    wallet::validate_caller_submission_token, ChainInstanceRegistryAttestation,
    EvmChainInstanceBinding, EvmFinalizedHeadObservation, EvmInclusionBlockObservation,
    EvmReceiptLookupObservation, EvmRoutingGenerationRef, EvmSubmissionFailure,
    EvmTransactionLookupObservation, EvmWalletFeeCandidate, EvmWalletReference,
    EvmWalletTransactionTemplate, UnsignedEip1559Envelope, UnsignedWalletCandidate,
    EVM_WALLET_REPLACEMENT_LIMIT,
};

const QUALIFIED_CHAIN_INSTANCE_DOMAIN: &str = "mfm.evm.qualified-chain-instance.v1";
const CHAIN_LINEAGE_DOMAIN: &str = "mfm.evm.chain-lineage.v1";
const WALLET_NONCE_DOMAIN: &str = "mfm.evm.wallet-nonce-domain.v1";
const INTENT_ISSUER_DOMAIN: &str = "mfm.evm.intent-issuer.v1";
const SUBMISSION_INTENT_DOMAIN: &str = "mfm.evm.submission-intent.v3";
const SUBMISSION_SEMANTICS_DOMAIN: &str = "mfm.evm.submission-semantics.v1";
const RESERVATION_KEY_DOMAIN: &str = "mfm.evm.nonce-reservation.v1";
const CANDIDATE_KEY_DOMAIN: &str = "mfm.evm.nonce-candidate.v1";
const COMPLETION_KEY_DOMAIN: &str = "mfm.evm.nonce-completion.v1";
const TRANSACTION_INTENT_DOMAIN: &str = "mfm.evm.transaction-intent.v1";
const CANDIDATE_FAMILY_DOMAIN: &str = "mfm.evm.candidate-family.v1";

/// Maximum number of statically authored observation rounds per candidate.
pub const EVM_WALLET_OBSERVATION_ROUND_LIMIT: u8 = 2;

/// Maximum protocol-valid transaction nonce ([EIP-2681](https://eips.ethereum.org/EIPS/eip-2681)).
///
/// `u64::MAX` (`2^64 - 1`) is permanently invalid as a transaction nonce. Every
/// admitted value therefore has a checked successor only while strictly below
/// this maximum; reserving the maximum exhausts capacity.
pub const EVM_TRANSACTION_NONCE_MAX: u64 = u64::MAX - 1;

/// Protocol-valid EVM transaction nonce with a representable checked successor
/// until capacity is exhausted.
///
/// Values equal to `u64::MAX` are rejected at every construction and decode
/// boundary. Zero through [`EVM_TRANSACTION_NONCE_MAX`] are admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-nonce",
    version = "1",
    schema = "mfm.evm.transaction_nonce"
)]
pub struct TransactionNonce {
    /// Protocol nonce value.
    pub(crate) value: u64,
}

impl<'de> Deserialize<'de> for TransactionNonce {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            value: u64,
        }

        Self::new(Wire::deserialize(deserializer)?.value).map_err(serde::de::Error::custom)
    }
}

impl TransactionNonce {
    /// Admits one protocol-valid nonce.
    pub fn new(value: u64) -> Result<Self, WalletAuthorityContractError> {
        if value > EVM_TRANSACTION_NONCE_MAX {
            return Err(WalletAuthorityContractError::Invalid("transaction_nonce"));
        }
        Ok(Self { value })
    }

    /// Returns the raw nonce value.
    pub const fn get(self) -> u64 {
        self.value
    }

    /// Revalidates the protocol nonce boundary.
    pub const fn validate(self) -> Result<(), WalletAuthorityContractError> {
        if self.value > EVM_TRANSACTION_NONCE_MAX {
            Err(WalletAuthorityContractError::Invalid("transaction_nonce"))
        } else {
            Ok(())
        }
    }

    /// Returns the checked successor when it remains protocol-valid.
    pub fn checked_successor(self) -> Option<Self> {
        self.value
            .checked_add(1)
            .and_then(|next| Self::new(next).ok())
    }
}

impl From<TransactionNonce> for u64 {
    fn from(value: TransactionNonce) -> Self {
        value.value
    }
}

impl FromStr for TransactionNonce {
    type Err = WalletAuthorityContractError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let parsed = u64::from_str(value)
            .map_err(|_| WalletAuthorityContractError::Invalid("transaction_nonce"))?;
        Self::new(parsed)
    }
}

impl std::fmt::Display for TransactionNonce {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.value)
    }
}

/// Error returned while constructing canonical wallet-authority material.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WalletAuthorityContractError {
    /// A public field was not in its canonical representation.
    #[error("invalid wallet-authority field: {0}")]
    Invalid(&'static str),
    /// A fixed collection bound was exceeded.
    #[error("wallet-authority bound exceeded: {0}")]
    BoundExceeded(&'static str),
    /// Canonical content addressing failed.
    #[error("wallet-authority canonical encoding failed")]
    Canonical,
}

/// Full recovery preimages carried as one bounded canonical object. Keeping
/// this closure behind one string keeps the public completion schema bounded
/// while retaining every typed preimage for offline verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompletedRecoveryClosure {
    reservation_request: ReserveEvmNonceRequest,
    completion_request: CompleteEvmNonceRequest,
    activation_requests: Vec<ActivateEvmCandidateRequest>,
    activation_state_inputs: Vec<LexicalValueRef>,
    reservation_state_input: LexicalValueRef,
    completion_state_input: LexicalValueRef,
    reservation: ReservedWalletNonce,
    transaction_intent: EvmTransactionIntent,
    candidate_family: EvmCandidateFamily,
    domain_activation_attestation: WalletNonceDomainActivationAttestation,
    qualified_floor: QualifiedPendingNonceFloor,
    route_generation_ref: EvmWalletReference,
    issuer_namespace_contract_ref: EvmWalletReference,
    observation_rounds: u8,
    submission_semantics_digest: SubmissionSemanticsDigest,
    canonical_terminal_outcome: CanonicalTerminalOutcome,
    terminal_witnesses: TerminalWitnesses,
    sealed_activated_candidates: Vec<ActiveWalletCandidate>,
    provider_completion_attestation: String,
    original_terminal_witnesses_ref: String,
    completion_evidence_ref: EvmWalletReference,
}

fn encode_completed_recovery(
    closure: &CompletedRecoveryClosure,
) -> Result<String, WalletAuthorityContractError> {
    let json =
        serde_json::to_string(closure).map_err(|_| WalletAuthorityContractError::Canonical)?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|_| WalletAuthorityContractError::Canonical)?;
    validate_completion_recovery_bytes(canonical.as_bytes())?;
    String::from_utf8(canonical.as_bytes().to_vec())
        .map_err(|_| WalletAuthorityContractError::Canonical)
}

fn decode_completed_recovery(
    value: &str,
) -> Result<CompletedRecoveryClosure, WalletAuthorityContractError> {
    let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(value.as_bytes())
        .map_err(|_| WalletAuthorityContractError::Canonical)?;
    validate_completion_recovery_bytes(canonical.as_bytes())?;
    serde_json::from_slice(canonical.as_bytes())
        .map_err(|_| WalletAuthorityContractError::Canonical)
}

fn validate_completion_recovery_bytes(bytes: &[u8]) -> Result<(), WalletAuthorityContractError> {
    if bytes.len() > MAX_COMPLETION_RECOVERY_BYTES {
        return Err(WalletAuthorityContractError::BoundExceeded(
            "completion_recovery",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod completion_recovery_limit_tests {
    use super::{validate_completion_recovery_bytes, MAX_COMPLETION_RECOVERY_BYTES};

    #[test]
    fn completion_recovery_accepts_exact_budget_and_rejects_one_byte_over() {
        let exact = vec![b'x'; MAX_COMPLETION_RECOVERY_BYTES];
        validate_completion_recovery_bytes(&exact)
            .expect("exact completion-recovery budget is accepted");

        let one_byte_over = vec![b'x'; MAX_COMPLETION_RECOVERY_BYTES + 1];
        assert!(validate_completion_recovery_bytes(&one_byte_over).is_err());
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "chain-instance-declaration",
    version = "1",
    schema = "mfm.evm.chain_instance_declaration"
)]
/// Immutable declaration of one qualified physical EVM chain instance.
pub struct ChainInstanceDeclaration {
    qualified_chain_registry_lineage_ref: EvmWalletReference,
    never_reused_instance_namespace_id: String,
    chain_id: u64,
    genesis_block_hash: String,
    finalized_block_number: String,
    finalized_block_hash: String,
}

impl ChainInstanceDeclaration {
    /// Constructs a declaration from canonical public chain evidence.
    pub fn new(
        qualified_chain_registry_lineage_ref: EvmWalletReference,
        never_reused_instance_namespace_id: StableId,
        chain_id: u64,
        genesis_block_hash: B256,
        finalized_block_number: U256,
        finalized_block_hash: B256,
    ) -> Result<Self, WalletAuthorityContractError> {
        validate_reference(&qualified_chain_registry_lineage_ref)?;
        if chain_id == 0 || finalized_block_hash == B256::ZERO {
            return Err(WalletAuthorityContractError::Invalid("chain_instance"));
        }
        Ok(Self {
            qualified_chain_registry_lineage_ref,
            never_reused_instance_namespace_id: never_reused_instance_namespace_id
                .as_str()
                .to_owned(),
            chain_id,
            genesis_block_hash: format!("{genesis_block_hash:#x}"),
            finalized_block_number: finalized_block_number.to_string(),
            finalized_block_hash: format!("{finalized_block_hash:#x}"),
        })
    }

    /// Returns the declared EVM chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the exact append-only chain-registry lineage.
    pub const fn chain_registry_lineage_ref(&self) -> &EvmWalletReference {
        &self.qualified_chain_registry_lineage_ref
    }

    /// Returns the never-reused instance namespace.
    pub fn instance_namespace_id(&self) -> &str {
        &self.never_reused_instance_namespace_id
    }

    /// Returns the canonical genesis block hash.
    pub fn genesis_block_hash(&self) -> &str {
        &self.genesis_block_hash
    }

    /// Returns the canonical finalized anchor number.
    pub fn finalized_block_number(&self) -> &str {
        &self.finalized_block_number
    }

    /// Returns the canonical finalized anchor hash.
    pub fn finalized_block_hash(&self) -> &str {
        &self.finalized_block_hash
    }

    /// Revalidates the canonical declaration.
    pub fn validate(&self) -> Result<(), WalletAuthorityContractError> {
        validate_reference(&self.qualified_chain_registry_lineage_ref)?;
        StableId::new(&self.never_reused_instance_namespace_id)
            .map_err(|_| WalletAuthorityContractError::Invalid("chain_namespace_id"))?;
        if self.chain_id == 0 {
            return Err(WalletAuthorityContractError::Invalid("chain_id"));
        }
        validate_hash(&self.genesis_block_hash, true)?;
        let finalized_block_number = U256::from_str(&self.finalized_block_number)
            .map_err(|_| WalletAuthorityContractError::Invalid("finalized_block_number"))?;
        if finalized_block_number.to_string() != self.finalized_block_number {
            return Err(WalletAuthorityContractError::Invalid(
                "finalized_block_number",
            ));
        }
        validate_hash(&self.finalized_block_hash, false)
    }
}

macro_rules! digest_identity {
    ($name:ident, $semantic:literal, $schema:literal) => {
        #[doc = concat!("Canonical ", stringify!($name), " identity.")]
        #[derive(
            Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, MfmValue,
        )]
        #[serde(deny_unknown_fields)]
        #[mfm(namespace = "mfm.evm", name = $semantic, version = "1", schema = $schema)]
        pub struct $name {
            digest: String,
        }

        impl $name {
            fn from_digest(digest: ContentDigest) -> Self {
                Self {
                    digest: digest.as_str().to_owned(),
                }
            }

            /// Returns the stable domain-separated digest identity.
            pub fn as_str(&self) -> &str {
                &self.digest
            }

            /// Revalidates the digest representation.
            pub fn validate(&self) -> Result<(), WalletAuthorityContractError> {
                ContentDigest::from_str(&self.digest)
                    .map(|_| ())
                    .map_err(|_| WalletAuthorityContractError::Invalid(stringify!($name)))
            }
        }
    };
}

digest_identity!(
    QualifiedChainInstanceId,
    "qualified-chain-instance-id",
    "mfm.evm.qualified_chain_instance_id"
);
digest_identity!(
    EvmChainLineageId,
    "chain-lineage-id",
    "mfm.evm.chain_lineage_id"
);
digest_identity!(
    AuthenticatedIntentIssuerId,
    "authenticated-intent-issuer-id",
    "mfm.evm.authenticated_intent_issuer_id"
);
digest_identity!(
    SubmissionIntentId,
    "submission-intent-id",
    "mfm.evm.submission_intent_id"
);
digest_identity!(
    SubmissionSemanticsDigest,
    "submission-semantics-digest",
    "mfm.evm.submission_semantics_digest"
);
digest_identity!(
    EvmNonceReservationKey,
    "nonce-reservation-key",
    "mfm.evm.nonce_reservation_key"
);
digest_identity!(
    EvmCandidateOperationKey,
    "candidate-operation-key",
    "mfm.evm.candidate_operation_key"
);
digest_identity!(
    EvmNonceCompletionKey,
    "nonce-completion-key",
    "mfm.evm.nonce_completion_key"
);

/// Stable uniqueness namespace for one qualified chain lineage and sender.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-nonce-domain",
    version = "1",
    schema = "mfm.evm.wallet_nonce_domain"
)]
pub struct WalletNonceDomain {
    chain_lineage_id: EvmChainLineageId,
    sender: String,
    digest: String,
}

impl WalletNonceDomain {
    /// Returns the qualified physical chain lineage.
    pub const fn chain_lineage_id(&self) -> &EvmChainLineageId {
        &self.chain_lineage_id
    }

    /// Returns the canonical lower-case sender address.
    pub fn sender(&self) -> &str {
        &self.sender
    }

    /// Returns the stable domain identity.
    pub fn as_str(&self) -> &str {
        &self.digest
    }

    /// Revalidates the complete identity and preimage.
    pub fn validate(&self) -> Result<(), WalletAuthorityContractError> {
        self.chain_lineage_id.validate()?;
        validate_address(&self.sender)?;
        let expected = hash(WALLET_NONCE_DOMAIN, &(&self.chain_lineage_id, &self.sender))?;
        if self.digest != expected.as_str() {
            return Err(WalletAuthorityContractError::Invalid("wallet_nonce_domain"));
        }
        Ok(())
    }
}

/// Derives the immutable qualified chain-instance identity.
pub fn derive_qualified_chain_instance_id(
    declaration: &ChainInstanceDeclaration,
) -> Result<QualifiedChainInstanceId, WalletAuthorityContractError> {
    declaration.validate()?;
    Ok(QualifiedChainInstanceId::from_digest(hash(
        QUALIFIED_CHAIN_INSTANCE_DOMAIN,
        declaration,
    )?))
}

/// Derives the stable EVM chain lineage identity.
pub fn derive_evm_chain_lineage_id(
    chain_instance: &QualifiedChainInstanceId,
) -> Result<EvmChainLineageId, WalletAuthorityContractError> {
    chain_instance.validate()?;
    Ok(EvmChainLineageId::from_digest(hash(
        CHAIN_LINEAGE_DOMAIN,
        chain_instance,
    )?))
}

/// Derives the sender-wide nonce uniqueness domain.
pub fn derive_wallet_nonce_domain(
    chain_lineage_id: EvmChainLineageId,
    sender: Address,
) -> Result<WalletNonceDomain, WalletAuthorityContractError> {
    chain_lineage_id.validate()?;
    if sender.is_zero() {
        return Err(WalletAuthorityContractError::Invalid("sender"));
    }
    let sender = format!("{sender:#x}");
    let digest = hash(WALLET_NONCE_DOMAIN, &(&chain_lineage_id, &sender))?;
    Ok(WalletNonceDomain {
        chain_lineage_id,
        sender,
        digest: digest.as_str().to_owned(),
    })
}

/// Derives the authenticated issuer namespace for ordinary caller tokens.
pub fn derive_authenticated_intent_issuer_id(
    tenant_id: &TenantScopeId,
    authenticated_principal_id: &StableId,
    issuer_namespace_contract_ref: &EvmWalletReference,
) -> Result<AuthenticatedIntentIssuerId, WalletAuthorityContractError> {
    issuer_namespace_contract_ref
        .to_content_ref()
        .map_err(|_| WalletAuthorityContractError::Invalid("issuer_namespace_contract_ref"))?;
    Ok(AuthenticatedIntentIssuerId::from_digest(hash(
        INTENT_ISSUER_DOMAIN,
        &(
            tenant_id.as_str(),
            authenticated_principal_id.as_str(),
            issuer_namespace_contract_ref,
        ),
    )?))
}

/// Derives the stable submission intent from its authenticated namespace only.
///
/// Behavior-changing submission material belongs to [`SubmissionSemanticsDigest`]
/// so the same caller token remains the permanent idempotency identity while a
/// changed request is rejected as a semantic conflict.
pub fn derive_submission_intent_id(
    domain: &WalletNonceDomain,
    issuer: &AuthenticatedIntentIssuerId,
    caller_submission_token: &str,
) -> Result<SubmissionIntentId, WalletAuthorityContractError> {
    domain.validate()?;
    issuer.validate()?;
    validate_caller_submission_token(caller_submission_token)
        .map_err(|_| WalletAuthorityContractError::Invalid("submission_token"))?;
    Ok(SubmissionIntentId::from_digest(hash(
        SUBMISSION_INTENT_DOMAIN,
        &(domain, issuer, caller_submission_token),
    )?))
}

/// Derives the complete behavior identity retained alongside an intent.
pub fn derive_submission_semantics_digest(
    transaction_intent: &EvmTransactionIntent,
    candidate_family: &EvmCandidateFamily,
    observation_rounds: u8,
    expansion_contract_ref: &EvmWalletReference,
    route_generation_ref: &EvmWalletReference,
    domain_activation_attestation: &WalletNonceDomainActivationAttestation,
    issuer_namespace_contract_ref: &EvmWalletReference,
) -> Result<SubmissionSemanticsDigest, WalletAuthorityContractError> {
    transaction_intent.validate()?;
    candidate_family.validate(transaction_intent)?;
    if observation_rounds == 0 || observation_rounds > EVM_WALLET_OBSERVATION_ROUND_LIMIT {
        return Err(WalletAuthorityContractError::Invalid("observation_rounds"));
    }
    validate_reference(expansion_contract_ref)?;
    validate_reference(route_generation_ref)?;
    domain_activation_attestation.validate()?;
    validate_reference(issuer_namespace_contract_ref)?;
    // The replacement policy is code-owned but still behavior-changing.  It
    // belongs in the retained semantics closure so a policy-contract change
    // cannot silently reuse an old intent/token digest.
    let replacement_policy_ref = crate::evm_wallet_nonce_policy_ref()
        .map_err(|_| WalletAuthorityContractError::Invalid("replacement_policy"))?;
    Ok(SubmissionSemanticsDigest::from_digest(hash(
        SUBMISSION_SEMANTICS_DOMAIN,
        &(
            transaction_intent,
            candidate_family,
            observation_rounds,
            expansion_contract_ref,
            route_generation_ref,
            domain_activation_attestation,
            issuer_namespace_contract_ref,
            &replacement_policy_ref,
        ),
    )?))
}

/// Derives the permanent reservation operation key.
pub fn derive_evm_nonce_reservation_key(
    domain: &WalletNonceDomain,
    intent: &SubmissionIntentId,
) -> Result<EvmNonceReservationKey, WalletAuthorityContractError> {
    domain.validate()?;
    intent.validate()?;
    Ok(EvmNonceReservationKey::from_digest(hash(
        RESERVATION_KEY_DOMAIN,
        &(domain, intent),
    )?))
}

/// Derives one permanent candidate-activation operation key.
pub fn derive_evm_candidate_operation_key(
    reservation_key: &EvmNonceReservationKey,
    candidate_ordinal: u16,
) -> Result<EvmCandidateOperationKey, WalletAuthorityContractError> {
    reservation_key.validate()?;
    Ok(EvmCandidateOperationKey::from_digest(hash(
        CANDIDATE_KEY_DOMAIN,
        &(reservation_key, candidate_ordinal),
    )?))
}

/// Derives the permanent completion operation key.
pub fn derive_evm_nonce_completion_key(
    reservation_key: &EvmNonceReservationKey,
) -> Result<EvmNonceCompletionKey, WalletAuthorityContractError> {
    reservation_key.validate()?;
    Ok(EvmNonceCompletionKey::from_digest(hash(
        COMPLETION_KEY_DOMAIN,
        reservation_key,
    )?))
}

/// Run-independent nonce-free EVM transaction intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-intent",
    version = "1",
    schema = "mfm.evm.transaction_intent"
)]
pub struct EvmTransactionIntent {
    chain_instance: EvmChainInstanceBinding,
    nonce_domain: WalletNonceDomain,
    template: EvmWalletTransactionTemplate,
    semantic_signer_id: String,
    signing_profile_contract_ref: EvmWalletReference,
    submission_contract_ref: EvmWalletReference,
    terminal_assurance_contract_ref: EvmWalletReference,
    digest: String,
}

impl EvmTransactionIntent {
    /// Returns the qualified chain instance frozen into the intent.
    pub const fn qualified_chain_instance_id(&self) -> &QualifiedChainInstanceId {
        self.chain_instance.qualified_chain_instance_id()
    }

    /// Returns the exact compact chain binding.
    pub const fn chain_instance(&self) -> &EvmChainInstanceBinding {
        &self.chain_instance
    }

    /// Constructs and hashes one immutable nonce-free intent.
    // Each argument is an independently hashed semantic identity; grouping
    // them would add a redundant public configuration type.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chain_instance: EvmChainInstanceBinding,
        nonce_domain: WalletNonceDomain,
        template: EvmWalletTransactionTemplate,
        semantic_signer_id: StableId,
        signing_profile_contract_ref: EvmWalletReference,
        submission_contract_ref: EvmWalletReference,
        terminal_assurance_contract_ref: EvmWalletReference,
    ) -> Result<Self, WalletAuthorityContractError> {
        chain_instance.validate()?;
        nonce_domain.validate()?;
        template
            .validate()
            .map_err(|_| WalletAuthorityContractError::Invalid("transaction_template"))?;
        validate_reference(&signing_profile_contract_ref)?;
        validate_reference(&submission_contract_ref)?;
        validate_reference(&terminal_assurance_contract_ref)?;
        let mut value = Self {
            chain_instance,
            nonce_domain,
            template,
            semantic_signer_id: semantic_signer_id.as_str().to_owned(),
            signing_profile_contract_ref,
            submission_contract_ref,
            terminal_assurance_contract_ref,
            digest: String::new(),
        };
        value.digest = hash(
            TRANSACTION_INTENT_DOMAIN,
            &(
                &value.chain_instance,
                &value.nonce_domain,
                &value.template,
                &value.semantic_signer_id,
                &value.signing_profile_contract_ref,
                &value.submission_contract_ref,
                &value.terminal_assurance_contract_ref,
            ),
        )?
        .as_str()
        .to_owned();
        Ok(value)
    }

    /// Returns the nonce domain.
    pub const fn nonce_domain(&self) -> &WalletNonceDomain {
        &self.nonce_domain
    }

    /// Returns the exact EVM chain id encoded into every candidate.
    pub const fn chain_id(&self) -> u64 {
        self.chain_instance.chain_id()
    }

    /// Returns the immutable transaction template.
    pub const fn template(&self) -> &EvmWalletTransactionTemplate {
        &self.template
    }

    /// Returns the stable semantic signer identity.
    pub fn semantic_signer_id(&self) -> &str {
        &self.semantic_signer_id
    }

    /// Returns the intent digest.
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Returns the deterministic signing profile contract.
    pub const fn signing_profile_contract_ref(&self) -> &EvmWalletReference {
        &self.signing_profile_contract_ref
    }

    /// Returns the semantic submission contract frozen into the intent.
    pub const fn submission_contract_ref(&self) -> &EvmWalletReference {
        &self.submission_contract_ref
    }

    /// Returns the terminal-assurance contract.
    pub const fn terminal_assurance_contract_ref(&self) -> &EvmWalletReference {
        &self.terminal_assurance_contract_ref
    }

    /// Builds the exact unsigned EIP-1559 candidate.
    pub fn unsigned_candidate(
        &self,
        nonce: u64,
        fee: &EvmWalletFeeCandidate,
    ) -> Result<UnsignedEip1559Envelope, WalletAuthorityContractError> {
        TransactionNonce::new(nonce)?;
        UnsignedEip1559Envelope::new(
            U256::from(self.chain_instance.chain_id()),
            U256::from(nonce),
            fee.max_priority_fee_quantity()
                .map_err(|_| WalletAuthorityContractError::Invalid("priority_fee"))?,
            fee.max_fee_quantity()
                .map_err(|_| WalletAuthorityContractError::Invalid("max_fee"))?,
            self.template
                .gas_limit_quantity()
                .map_err(|_| WalletAuthorityContractError::Invalid("gas_limit"))?,
            self.template
                .action()
                .to_alloy()
                .map_err(|_| WalletAuthorityContractError::Invalid("transaction_action"))?,
            self.template
                .value_quantity()
                .map_err(|_| WalletAuthorityContractError::Invalid("transaction_value"))?,
            self.template
                .access_list_alloy()
                .map_err(|_| WalletAuthorityContractError::Invalid("access_list"))?,
            self.template
                .input_bytes()
                .map_err(|_| WalletAuthorityContractError::Invalid("transaction_input"))?,
        )
        .map_err(|_| WalletAuthorityContractError::Invalid("unsigned_candidate"))
    }

    /// Revalidates the complete intent and digest.
    pub fn validate(&self) -> Result<(), WalletAuthorityContractError> {
        let semantic_signer_id = StableId::new(&self.semantic_signer_id)
            .map_err(|_| WalletAuthorityContractError::Invalid("semantic_signer_id"))?;
        let rebuilt = Self::new(
            self.chain_instance.clone(),
            self.nonce_domain.clone(),
            self.template.clone(),
            semantic_signer_id,
            self.signing_profile_contract_ref.clone(),
            self.submission_contract_ref.clone(),
            self.terminal_assurance_contract_ref.clone(),
        )?;
        if rebuilt != *self {
            return Err(WalletAuthorityContractError::Invalid("transaction_intent"));
        }
        Ok(())
    }
}

/// Non-empty declaration-ordered mutation-equivalent fee family.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "candidate-family",
    version = "1",
    schema = "mfm.evm.candidate_family"
)]
pub struct EvmCandidateFamily {
    intent_digest: String,
    candidates: Vec<EvmWalletFeeCandidate>,
    digest: String,
}

impl EvmCandidateFamily {
    /// Constructs the fixed candidate family.
    pub fn new(
        intent: &EvmTransactionIntent,
        candidates: Vec<EvmWalletFeeCandidate>,
    ) -> Result<Self, WalletAuthorityContractError> {
        intent.validate()?;
        if candidates.is_empty() || candidates.len() > EVM_WALLET_REPLACEMENT_LIMIT {
            return Err(WalletAuthorityContractError::BoundExceeded(
                "candidate_family",
            ));
        }
        for pair in candidates.windows(2) {
            let prior_max = pair[0]
                .max_fee_quantity()
                .map_err(|_| WalletAuthorityContractError::Invalid("candidate_fee"))?;
            let next_max = pair[1]
                .max_fee_quantity()
                .map_err(|_| WalletAuthorityContractError::Invalid("candidate_fee"))?;
            let prior_priority = pair[0]
                .max_priority_fee_quantity()
                .map_err(|_| WalletAuthorityContractError::Invalid("candidate_fee"))?;
            let next_priority = pair[1]
                .max_priority_fee_quantity()
                .map_err(|_| WalletAuthorityContractError::Invalid("candidate_fee"))?;
            if next_max <= prior_max || next_priority <= prior_priority {
                return Err(WalletAuthorityContractError::Invalid(
                    "candidate_fee_progression",
                ));
            }
        }
        let intent_digest = intent.digest.clone();
        let digest = hash(CANDIDATE_FAMILY_DOMAIN, &(&intent_digest, &candidates))?
            .as_str()
            .to_owned();
        Ok(Self {
            intent_digest,
            candidates,
            digest,
        })
    }

    /// Returns the declaration-ordered fee candidates.
    pub fn candidates(&self) -> &[EvmWalletFeeCandidate] {
        &self.candidates
    }

    /// Returns the family content digest.
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Revalidates the family against its exact intent.
    pub fn validate(
        &self,
        intent: &EvmTransactionIntent,
    ) -> Result<(), WalletAuthorityContractError> {
        if *self != Self::new(intent, self.candidates.clone())? {
            return Err(WalletAuthorityContractError::Invalid("candidate_family"));
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for EvmCandidateFamily {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            intent_digest: String,
            candidates: Vec<EvmWalletFeeCandidate>,
            digest: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        if wire.candidates.is_empty() || wire.candidates.len() > EVM_WALLET_REPLACEMENT_LIMIT {
            return Err(serde::de::Error::custom("invalid candidate family bound"));
        }
        for pair in wire.candidates.windows(2) {
            let prior_max = pair[0]
                .max_fee_quantity()
                .map_err(serde::de::Error::custom)?;
            let next_max = pair[1]
                .max_fee_quantity()
                .map_err(serde::de::Error::custom)?;
            let prior_priority = pair[0]
                .max_priority_fee_quantity()
                .map_err(serde::de::Error::custom)?;
            let next_priority = pair[1]
                .max_priority_fee_quantity()
                .map_err(serde::de::Error::custom)?;
            if next_max <= prior_max || next_priority <= prior_priority {
                return Err(serde::de::Error::custom(
                    "invalid candidate fee progression",
                ));
            }
        }
        let expected = hash(
            CANDIDATE_FAMILY_DOMAIN,
            &(&wire.intent_digest, &wire.candidates),
        )
        .map_err(serde::de::Error::custom)?;
        if wire.digest != expected.as_str() {
            return Err(serde::de::Error::custom("invalid candidate family digest"));
        }
        Ok(Self {
            intent_digest: wire.intent_digest,
            candidates: wire.candidates,
            digest: wire.digest,
        })
    }
}

/// Public identity of one wallet-nonce store incarnation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-nonce-store-incarnation",
    version = "1",
    schema = "mfm.evm.wallet_nonce_store_incarnation"
)]
pub struct WalletNonceStoreIncarnation {
    /// Stable physical store lineage.
    pub wallet_nonce_store_lineage_id: String,
    /// Monotonic writer epoch.
    pub writer_epoch: u64,
    /// Exact physical target instance.
    pub physical_target_instance_id: String,
    /// Public reference of the non-exportable target-held key.
    pub non_exportable_target_public_key_ref: EvmWalletReference,
    /// Target-attestation contract.
    pub target_attestation_contract_ref: EvmWalletReference,
}

impl WalletNonceStoreIncarnation {
    /// Revalidates one public physical store-incarnation identity.
    pub fn validate(&self) -> Result<(), WalletAuthorityContractError> {
        StableId::new(&self.wallet_nonce_store_lineage_id)
            .map_err(|_| WalletAuthorityContractError::Invalid("store_lineage_id"))?;
        StableId::new(&self.physical_target_instance_id)
            .map_err(|_| WalletAuthorityContractError::Invalid("physical_target_instance_id"))?;
        if self.writer_epoch == 0 {
            return Err(WalletAuthorityContractError::Invalid("writer_epoch"));
        }
        validate_reference(&self.non_exportable_target_public_key_ref)?;
        validate_reference(&self.target_attestation_contract_ref)
    }
}

/// Required replay-exclusion activation disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm",
    name = "replay-exclusion-disposition",
    version = "1",
    schema = "mfm.evm.replay_exclusion_disposition"
)]
pub enum ReplayExclusionDisposition {
    /// Every prior request replay and retry ingress is excluded.
    EveryPriorRequestReplayAndRetryIngressExcluded,
}

/// Required exclusive sender-control activation disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm",
    name = "exclusive-current-control",
    version = "1",
    schema = "mfm.evm.exclusive_current_control"
)]
pub enum ExclusiveCurrentControl {
    /// Every prior sender-capable path is permanently fenced.
    EveryPriorWriterSignerRelayerOperatorStaleDeploymentAndDirectSubmitPathFenced,
}

/// Required prior-effect activation disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm",
    name = "prior-effect-disposition",
    version = "1",
    schema = "mfm.evm.prior_effect_disposition"
)]
pub enum PriorEffectDisposition {
    /// No prior possible-entry effect remains unresolved.
    NoUnresolvedPossibleEntry,
}

/// Required prior-resource activation disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm",
    name = "prior-resource-disposition",
    version = "1",
    schema = "mfm.evm.prior_resource_disposition"
)]
pub enum PriorResourceDisposition {
    /// Every prior allocation and submitted candidate is terminal.
    EveryPriorAllocationAndSubmittedCandidateTerminal,
}

/// Permanent current-schema domain activation record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-nonce-domain-activation-record",
    version = "1",
    schema = "mfm.evm.wallet_nonce_domain_activation_record"
)]
pub struct WalletNonceDomainActivationRecord {
    /// Activation semantic contract.
    pub activation_contract_ref: EvmWalletReference,
    /// Qualified activation-registry lineage.
    pub qualified_activation_registry_lineage_ref: EvmWalletReference,
    /// Stable nonce-store lineage.
    pub wallet_nonce_store_lineage_id: String,
    /// Initially published store incarnation.
    pub initial_store_incarnation_ref: EvmWalletReference,
    /// Permanently bound domain.
    pub wallet_nonce_domain: WalletNonceDomain,
    /// Complete provider-issued qualified chain attestation.
    pub chain_instance_attestation: ChainInstanceRegistryAttestation,
    /// Exact initial route generation used to establish the nonce floor.
    pub initial_route_generation_ref: EvmRoutingGenerationRef,
    /// Provider-issued membership of that route in the attested chain instance.
    pub initial_route_membership_issuance_ref: EvmWalletReference,
    /// Canonical sender address.
    pub sender_identity: String,
    /// Immutable issuer namespace contract.
    pub issuer_namespace_contract_ref: EvmWalletReference,
    /// Replay-exclusion contract.
    pub replay_exclusion_contract_ref: EvmWalletReference,
    /// Exact replay-exclusion disposition.
    pub replay_exclusion_disposition: ReplayExclusionDisposition,
    /// Canonical finalized sender nonce floor.
    pub finalized_sender_nonce_floor: u64,
    /// Finalized floor block number.
    pub finalized_block_number: String,
    /// Finalized floor block hash.
    pub finalized_block_hash: String,
    /// Qualified finalized observation proof.
    pub qualified_observation_proof_ref: EvmWalletReference,
    /// Exhaustive sender-path inventory digest.
    pub exhaustive_sender_path_inventory_digest: String,
    /// Exclusive current-control disposition.
    pub exclusive_current_control: ExclusiveCurrentControl,
    /// Prior effect disposition.
    pub prior_effect_disposition: PriorEffectDisposition,
    /// Prior resource disposition.
    pub prior_resource_disposition: PriorResourceDisposition,
    /// New immutable idempotency epoch.
    pub new_idempotency_epoch: String,
}

impl WalletNonceDomainActivationRecord {
    /// Revalidates all permanent activation invariants.
    pub fn validate(&self) -> Result<(), WalletAuthorityContractError> {
        validate_reference(&self.activation_contract_ref)?;
        validate_reference(&self.qualified_activation_registry_lineage_ref)?;
        StableId::new(&self.wallet_nonce_store_lineage_id)
            .map_err(|_| WalletAuthorityContractError::Invalid("store_lineage_id"))?;
        validate_reference(&self.initial_store_incarnation_ref)?;
        self.wallet_nonce_domain.validate()?;
        self.chain_instance_attestation.validate()?;
        self.initial_route_generation_ref
            .to_content_ref()
            .map_err(|_| WalletAuthorityContractError::Invalid("initial_route_generation"))?;
        validate_reference(&self.initial_route_membership_issuance_ref)?;
        validate_address(&self.sender_identity)?;
        if self.sender_identity != self.wallet_nonce_domain.sender {
            return Err(WalletAuthorityContractError::Invalid("activation_sender"));
        }
        validate_reference(&self.issuer_namespace_contract_ref)?;
        validate_reference(&self.replay_exclusion_contract_ref)?;
        TransactionNonce::new(self.finalized_sender_nonce_floor)?;
        let finalized_block_number = U256::from_str(&self.finalized_block_number)
            .map_err(|_| WalletAuthorityContractError::Invalid("finalized_block_number"))?;
        if finalized_block_number.to_string() != self.finalized_block_number {
            return Err(WalletAuthorityContractError::Invalid(
                "finalized_block_number",
            ));
        }
        validate_hash(&self.finalized_block_hash, false)?;
        validate_reference(&self.qualified_observation_proof_ref)?;
        ContentDigest::from_str(&self.exhaustive_sender_path_inventory_digest)
            .map_err(|_| WalletAuthorityContractError::Invalid("sender_path_inventory"))?;
        StableId::new(&self.new_idempotency_epoch)
            .map_err(|_| WalletAuthorityContractError::Invalid("idempotency_epoch"))?;
        let chain_binding = self.chain_instance_attestation.binding()?;
        let lineage = derive_evm_chain_lineage_id(chain_binding.qualified_chain_instance_id())?;
        let sender = Address::from_str(&self.sender_identity)
            .map_err(|_| WalletAuthorityContractError::Invalid("activation_sender"))?;
        if derive_wallet_nonce_domain(lineage, sender)? != self.wallet_nonce_domain {
            return Err(WalletAuthorityContractError::Invalid(
                "activation_chain_domain",
            ));
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for WalletNonceDomainActivationRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            activation_contract_ref: EvmWalletReference,
            qualified_activation_registry_lineage_ref: EvmWalletReference,
            wallet_nonce_store_lineage_id: String,
            initial_store_incarnation_ref: EvmWalletReference,
            wallet_nonce_domain: WalletNonceDomain,
            chain_instance_attestation: ChainInstanceRegistryAttestation,
            initial_route_generation_ref: EvmRoutingGenerationRef,
            initial_route_membership_issuance_ref: EvmWalletReference,
            sender_identity: String,
            issuer_namespace_contract_ref: EvmWalletReference,
            replay_exclusion_contract_ref: EvmWalletReference,
            replay_exclusion_disposition: ReplayExclusionDisposition,
            finalized_sender_nonce_floor: u64,
            finalized_block_number: String,
            finalized_block_hash: String,
            qualified_observation_proof_ref: EvmWalletReference,
            exhaustive_sender_path_inventory_digest: String,
            exclusive_current_control: ExclusiveCurrentControl,
            prior_effect_disposition: PriorEffectDisposition,
            prior_resource_disposition: PriorResourceDisposition,
            new_idempotency_epoch: String,
        }
        let wire = Wire::deserialize(deserializer)?;
        TransactionNonce::new(wire.finalized_sender_nonce_floor)
            .map_err(serde::de::Error::custom)?;
        Ok(Self {
            activation_contract_ref: wire.activation_contract_ref,
            qualified_activation_registry_lineage_ref: wire
                .qualified_activation_registry_lineage_ref,
            wallet_nonce_store_lineage_id: wire.wallet_nonce_store_lineage_id,
            initial_store_incarnation_ref: wire.initial_store_incarnation_ref,
            wallet_nonce_domain: wire.wallet_nonce_domain,
            chain_instance_attestation: wire.chain_instance_attestation,
            initial_route_generation_ref: wire.initial_route_generation_ref,
            initial_route_membership_issuance_ref: wire.initial_route_membership_issuance_ref,
            sender_identity: wire.sender_identity,
            issuer_namespace_contract_ref: wire.issuer_namespace_contract_ref,
            replay_exclusion_contract_ref: wire.replay_exclusion_contract_ref,
            replay_exclusion_disposition: wire.replay_exclusion_disposition,
            finalized_sender_nonce_floor: wire.finalized_sender_nonce_floor,
            finalized_block_number: wire.finalized_block_number,
            finalized_block_hash: wire.finalized_block_hash,
            qualified_observation_proof_ref: wire.qualified_observation_proof_ref,
            exhaustive_sender_path_inventory_digest: wire.exhaustive_sender_path_inventory_digest,
            exclusive_current_control: wire.exclusive_current_control,
            prior_effect_disposition: wire.prior_effect_disposition,
            prior_resource_disposition: wire.prior_resource_disposition,
            new_idempotency_epoch: wire.new_idempotency_epoch,
        })
    }
}

/// Secret-free public evidence of one permanent domain activation.
///
/// Structural validation proves only canonical consistency. Live authority
/// additionally requires an exact match in the provider-issued, non-persisted
/// activation verifier held by the PostgreSQL wallet authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-nonce-domain-activation-attestation",
    version = "1",
    schema = "mfm.evm.wallet_nonce_domain_activation_attestation"
)]
pub struct WalletNonceDomainActivationAttestation {
    /// Exact activation record identity.
    pub activation_record_ref: EvmWalletReference,
    /// Provider-assigned permanent registry issuance identity.
    pub registry_issuance_ref: EvmWalletReference,
    /// Secret-free activation-registry lineage binding.
    pub activation_registry_lineage_ref: EvmWalletReference,
    /// Exact initial store incarnation observed at issuance.
    pub initial_store_incarnation_ref: EvmWalletReference,
    /// Full current-schema activation record.
    pub current_schema_record: WalletNonceDomainActivationRecord,
}

impl WalletNonceDomainActivationAttestation {
    /// Revalidates the public attestation's canonical structure.
    pub fn validate(&self) -> Result<(), WalletAuthorityContractError> {
        self.current_schema_record.validate()?;
        validate_reference(&self.registry_issuance_ref)?;
        if self.activation_record_ref != canonical_wallet_reference(&self.current_schema_record)?
            || self.activation_registry_lineage_ref
                != self
                    .current_schema_record
                    .qualified_activation_registry_lineage_ref
            || self.initial_store_incarnation_ref
                != self.current_schema_record.initial_store_incarnation_ref
        {
            return Err(WalletAuthorityContractError::Invalid("domain_activation"));
        }
        Ok(())
    }
}

/// Desired exact successor identity for provider-authorized store promotion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-nonce-store-successor",
    version = "1",
    schema = "mfm.evm.wallet_nonce_store_successor"
)]
pub struct WalletNonceStoreSuccessor {
    /// Stable wallet store lineage.
    pub wallet_nonce_store_lineage_id: String,
    /// Exact current incarnation expected by the registry CAS.
    pub expected_current_incarnation_ref: EvmWalletReference,
    /// Fully hydrated successor that the provider must keep closed until CAS.
    pub next_incarnation: WalletNonceStoreIncarnation,
}

impl WalletNonceStoreSuccessor {
    /// Revalidates the desired successor identity without claiming promotion authority.
    pub fn validate(&self) -> Result<(), WalletAuthorityContractError> {
        StableId::new(&self.wallet_nonce_store_lineage_id)
            .map_err(|_| WalletAuthorityContractError::Invalid("store_lineage_id"))?;
        validate_reference(&self.expected_current_incarnation_ref)?;
        self.next_incarnation.validate()?;
        if self.next_incarnation.wallet_nonce_store_lineage_id != self.wallet_nonce_store_lineage_id
        {
            return Err(WalletAuthorityContractError::Invalid("promotion_lineage"));
        }
        Ok(())
    }
}

/// Secret-free public attestation of one exact store-lineage promotion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-nonce-store-promotion-attestation",
    version = "1",
    schema = "mfm.evm.wallet_nonce_store_promotion_attestation"
)]
pub struct WalletNonceStorePromotionAttestation {
    /// Stable physical store lineage.
    pub wallet_nonce_store_lineage_id: String,
    /// Previous immutable incarnation.
    pub previous_incarnation_ref: EvmWalletReference,
    /// Newly current immutable incarnation.
    pub current_incarnation_ref: EvmWalletReference,
    /// Monotonic current writer epoch.
    pub writer_epoch: u64,
    /// Exact provider-authorized successor identity.
    pub successor_ref: EvmWalletReference,
    /// Qualified activation registry lineage.
    pub qualified_activation_registry_lineage_ref: EvmWalletReference,
}

impl WalletNonceStorePromotionAttestation {
    /// Revalidates public structure without claiming promotion authority.
    pub fn validate(&self) -> Result<(), WalletAuthorityContractError> {
        StableId::new(&self.wallet_nonce_store_lineage_id)
            .map_err(|_| WalletAuthorityContractError::Invalid("store_lineage_id"))?;
        if self.writer_epoch == 0 {
            return Err(WalletAuthorityContractError::Invalid("writer_epoch"));
        }
        validate_reference(&self.previous_incarnation_ref)?;
        validate_reference(&self.current_incarnation_ref)?;
        validate_reference(&self.successor_ref)?;
        validate_reference(&self.qualified_activation_registry_lineage_ref)
    }
}

/// Fresh provider pending-nonce observation before policy qualification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "observed-pending-nonce-floor",
    version = "1",
    schema = "mfm.evm.observed_pending_nonce_floor"
)]
pub struct ObservedPendingNonceFloor {
    /// Exact nonce domain.
    pub nonce_domain: WalletNonceDomain,
    /// Exact qualified route generation.
    pub route_generation_ref: EvmWalletReference,
    /// Strictly decoded pending nonce.
    pub pending_nonce: TransactionNonce,
}

/// Policy-qualified fresh pending nonce floor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "qualified-pending-nonce-floor",
    version = "1",
    schema = "mfm.evm.qualified_pending_nonce_floor"
)]
pub struct QualifiedPendingNonceFloor {
    /// Exact observed pending value.
    pub observed: ObservedPendingNonceFloor,
    /// Immutable pending-floor policy.
    pub pending_floor_policy_ref: EvmWalletReference,
}

/// Producer-bound pending-nonce observation accepted by wallet reservation.
///
/// The origin binds the provider result to the exact chain, sender, route release,
/// policy, request, and committed authorization that caused the observation. Fields
/// are private so callers cannot fabricate a qualified observation from a raw quantity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedPendingNonceObservation {
    /// Qualified pending-nonce floor.
    floor: QualifiedPendingNonceFloor,
    /// Exact chain-instance declaration reference.
    chain_instance_ref: EvmWalletReference,
    /// Authenticated sender address.
    sender: String,
    /// Physical release certificate used for the read.
    physical_release_ref: EvmWalletReference,
    /// Producer run that committed the authorization.
    source_run_id: RunId,
    /// Authorization record that originated the request.
    authorization_ref: RecordRef,
    /// Request object reference retained by the authorization.
    request_ref: ContentRef,
}

impl QualifiedPendingNonceObservation {
    /// Constructs one observation only from an already committed origin tuple.
    #[allow(clippy::too_many_arguments)]
    #[doc(hidden)]
    pub(crate) fn from_committed_origin(
        floor: QualifiedPendingNonceFloor,
        chain_instance_ref: EvmWalletReference,
        sender: Address,
        physical_release_ref: EvmWalletReference,
        source_run_id: RunId,
        authorization_ref: RecordRef,
        request_ref: ContentRef,
    ) -> Result<Self, WalletAuthorityContractError> {
        floor.observed.pending_nonce.validate()?;
        floor.observed.nonce_domain.validate()?;
        physical_release_ref
            .to_content_ref()
            .map_err(|_| WalletAuthorityContractError::Invalid("pending_release"))?;
        let sender_text = format!("{sender:#x}");
        if floor.observed.nonce_domain.sender() != sender_text {
            return Err(WalletAuthorityContractError::Invalid("pending_sender"));
        }
        Ok(Self {
            floor,
            chain_instance_ref,
            sender: format!("{sender:#x}"),
            physical_release_ref,
            source_run_id,
            authorization_ref,
            request_ref,
        })
    }

    /// Builds a fixture-only observation for storage parity tests.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn from_test_origin(
        floor: QualifiedPendingNonceFloor,
        chain_instance_ref: EvmWalletReference,
        sender: Address,
        physical_release_ref: EvmWalletReference,
        source_run_id: RunId,
        authorization_ref: RecordRef,
        request_ref: ContentRef,
    ) -> Result<Self, WalletAuthorityContractError> {
        Self::from_committed_origin(
            floor,
            chain_instance_ref,
            sender,
            physical_release_ref,
            source_run_id,
            authorization_ref,
            request_ref,
        )
    }

    /// Returns the qualified floor for the reservation port.
    pub const fn floor(&self) -> &QualifiedPendingNonceFloor {
        &self.floor
    }

    /// Returns the exact producer run.
    pub const fn source_run_id(&self) -> &RunId {
        &self.source_run_id
    }

    /// Returns the exact chain binding carried by the committed origin.
    pub const fn chain_instance_ref(&self) -> &EvmWalletReference {
        &self.chain_instance_ref
    }

    /// Returns the exact physical release used for the pending read.
    pub const fn physical_release_ref(&self) -> &EvmWalletReference {
        &self.physical_release_ref
    }

    /// Returns the exact persisted request object reference that originated
    /// this provider observation.
    pub const fn request_ref(&self) -> &ContentRef {
        &self.request_ref
    }

    /// Returns the authenticated sender bound to the pending read.
    pub fn sender(&self) -> Result<Address, WalletAuthorityContractError> {
        Address::from_str(&self.sender)
            .map_err(|_| WalletAuthorityContractError::Invalid("pending_sender"))
    }

    /// Revalidates the complete producer-bound origin.
    pub fn validate(&self) -> Result<(), WalletAuthorityContractError> {
        self.floor.observed.pending_nonce.validate()?;
        self.floor.observed.nonce_domain.validate()?;
        validate_reference(&self.floor.observed.route_generation_ref)?;
        validate_reference(&self.floor.pending_floor_policy_ref)?;
        validate_reference(&self.chain_instance_ref)?;
        validate_reference(&self.physical_release_ref)?;
        if self.authorization_ref.run_id != self.source_run_id
            || self.authorization_ref.run_sequence == 0
            || self.request_ref.content_digest().as_str().is_empty()
        {
            return Err(WalletAuthorityContractError::Invalid("pending_origin"));
        }
        validate_address(&self.sender)
    }
}

/// Permanently reserved nonce and exact intent/family identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "reserved-wallet-nonce",
    version = "1",
    schema = "mfm.evm.reserved_wallet_nonce"
)]
pub struct ReservedWalletNonce {
    /// Exact nonce domain.
    pub nonce_domain: WalletNonceDomain,
    /// Permanent activation record.
    pub domain_activation_record_ref: EvmWalletReference,
    /// Allocated EVM account nonce.
    pub nonce: u64,
    /// Stable reservation operation key.
    pub semantic_reservation_key: EvmNonceReservationKey,
    /// Stable authenticated submission intent.
    pub submission_intent_id: SubmissionIntentId,
    /// Exact transaction intent digest.
    pub transaction_intent_digest: String,
    /// Exact candidate family digest.
    pub candidate_family_ref: String,
    /// Original creation-only pending-floor producer reference.
    pub observed_floor_ref: String,
    /// Stable resource lineage reference.
    pub resource_lineage_ref: EvmWalletReference,
    /// Permanent reservation proof.
    pub reservation_evidence_ref: EvmWalletReference,
}

impl<'de> Deserialize<'de> for ReservedWalletNonce {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            nonce_domain: WalletNonceDomain,
            domain_activation_record_ref: EvmWalletReference,
            nonce: u64,
            semantic_reservation_key: EvmNonceReservationKey,
            submission_intent_id: SubmissionIntentId,
            transaction_intent_digest: String,
            candidate_family_ref: String,
            observed_floor_ref: String,
            resource_lineage_ref: EvmWalletReference,
            reservation_evidence_ref: EvmWalletReference,
        }
        let wire = Wire::deserialize(deserializer)?;
        TransactionNonce::new(wire.nonce).map_err(serde::de::Error::custom)?;
        Ok(Self {
            nonce_domain: wire.nonce_domain,
            domain_activation_record_ref: wire.domain_activation_record_ref,
            nonce: wire.nonce,
            semantic_reservation_key: wire.semantic_reservation_key,
            submission_intent_id: wire.submission_intent_id,
            transaction_intent_digest: wire.transaction_intent_digest,
            candidate_family_ref: wire.candidate_family_ref,
            observed_floor_ref: wire.observed_floor_ref,
            resource_lineage_ref: wire.resource_lineage_ref,
            reservation_evidence_ref: wire.reservation_evidence_ref,
        })
    }
}

impl ReservedWalletNonce {
    /// Revalidates the complete immutable reservation closure.
    pub fn validate(&self) -> Result<(), WalletAuthorityContractError> {
        TransactionNonce::new(self.nonce)?;
        self.nonce_domain.validate()?;
        validate_reference(&self.domain_activation_record_ref)?;
        self.semantic_reservation_key.validate()?;
        self.submission_intent_id.validate()?;
        ContentDigest::from_str(&self.transaction_intent_digest)
            .map_err(|_| WalletAuthorityContractError::Invalid("transaction_intent_digest"))?;
        ContentDigest::from_str(&self.candidate_family_ref)
            .map_err(|_| WalletAuthorityContractError::Invalid("candidate_family_ref"))?;
        ContentDigest::from_str(&self.observed_floor_ref)
            .map_err(|_| WalletAuthorityContractError::Invalid("observed_floor_ref"))?;
        validate_reference(&self.resource_lineage_ref)?;
        validate_reference(&self.reservation_evidence_ref)
    }
}

/// Secret-free signer attestation for one exact unsigned candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "attested-wallet-candidate",
    version = "1",
    schema = "mfm.evm.attested_wallet_candidate"
)]
pub struct AttestedWalletCandidate {
    /// Stable reservation key.
    pub semantic_reservation_key: EvmNonceReservationKey,
    /// Declaration-ordered family ordinal.
    pub candidate_ordinal: u16,
    /// Canonical candidate descriptor reference.
    pub candidate_descriptor_ref: EvmWalletReference,
    /// Digest of exact unsigned candidate bytes.
    pub unsigned_candidate_digest: String,
    /// Deterministically derived EVM transaction hash.
    pub transaction_hash: String,
    /// Stable semantic signer identity.
    pub semantic_signer_id: String,
    /// Exact deterministic signing profile.
    pub signing_profile_contract_ref: EvmWalletReference,
    /// Secret-free signer attestation proof.
    pub signer_attestation_ref: EvmWalletReference,
}

/// One permanently activated candidate in the contiguous prefix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "active-wallet-candidate",
    version = "1",
    schema = "mfm.evm.active_wallet_candidate"
)]
pub struct ActiveWalletCandidate {
    /// Exact signer attestation.
    pub attested_candidate: AttestedWalletCandidate,
    /// Permanent activation proof.
    pub activation_evidence_ref: EvmWalletReference,
    /// Provider-issued attestation retained with the activation closure.
    pub provider_activation_attestation: String,
}

/// Closed activation precondition authored by exact pure states.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "candidate-activation-permit",
    version = "1",
    schema = "mfm.evm.candidate_activation_permit"
)]
pub enum CandidateActivationPermit {
    /// First candidate for an empty activated prefix.
    Initial {
        /// Exact expected next ordinal, necessarily zero.
        exact_next_ordinal: u16,
    },
    /// Statically next replacement candidate.
    Replacement {
        /// Exact predecessor activation proof.
        predecessor_activation_ref: EvmWalletReference,
        /// Exact predecessor ordinal.
        predecessor_ordinal: u16,
        /// Exact next ordinal.
        exact_next_ordinal: u16,
        /// Frozen replacement policy.
        replacement_policy_ref: EvmWalletReference,
        /// Producer-bound eligibility digest (observation evidence, not state self-certification).
        eligibility_ref: String,
    },
}

/// Derives the one exact activation permit for the next member of a retained
/// candidate prefix. Retained candidates use the observation-only recovery path.
///
/// `observed_prefix_len` is the exclusive upper bound of activated ordinals that
/// have independent producer observation evidence in the current recovery walk.
/// Certified order requires `observed_prefix_len == next_candidate_ordinal`
/// before any activation (initial or replacement). Replacement
/// therefore cannot admit until every retained activated candidate has been
/// observed (EVM-03/EVM-04).
pub(crate) fn derive_exact_candidate_activation_permit_inner(
    reservation: &ReservedWalletNonce,
    activated_candidates: &[ActiveWalletCandidate],
    next_candidate_ordinal: u16,
    observed_prefix_len: u16,
) -> Result<CandidateActivationPermit, WalletAuthorityContractError> {
    TransactionNonce::new(reservation.nonce)?;
    if activated_candidates
        .iter()
        .enumerate()
        .any(|(ordinal, candidate)| {
            candidate.attested_candidate.semantic_reservation_key
                != reservation.semantic_reservation_key
                || usize::from(candidate.attested_candidate.candidate_ordinal) != ordinal
        })
    {
        return Err(WalletAuthorityContractError::Invalid(
            "candidate_progression",
        ));
    }

    // Affine certified-order walk: no ordinal may activate until every earlier
    // activated ordinal has producer observation evidence in this recovery.
    if observed_prefix_len != next_candidate_ordinal {
        return Err(WalletAuthorityContractError::Invalid(
            "candidate_observation_order",
        ));
    }

    // Retained candidates are handled by the observation-only state path.
    if let Some(retained) = activated_candidates
        .get(usize::from(next_candidate_ordinal))
        .filter(|candidate| {
            candidate.attested_candidate.candidate_ordinal == next_candidate_ordinal
        })
    {
        let _ = retained;
        return Err(WalletAuthorityContractError::Invalid(
            "retained_candidate_requires_observation",
        ));
    }

    if usize::from(next_candidate_ordinal) != activated_candidates.len()
        || activated_candidates.len() >= EVM_WALLET_REPLACEMENT_LIMIT
    {
        return Err(WalletAuthorityContractError::Invalid(
            "candidate_progression",
        ));
    }

    let Some(predecessor) = activated_candidates.last() else {
        return (next_candidate_ordinal == 0)
            .then_some(CandidateActivationPermit::Initial {
                exact_next_ordinal: 0,
            })
            .ok_or(WalletAuthorityContractError::Invalid(
                "candidate_progression",
            ));
    };
    if predecessor
        .attested_candidate
        .candidate_ordinal
        .checked_add(1)
        != Some(next_candidate_ordinal)
    {
        return Err(WalletAuthorityContractError::Invalid(
            "candidate_progression",
        ));
    }

    let replacement_policy_ref = crate::evm_wallet_nonce_policy_ref()
        .map_err(|_| WalletAuthorityContractError::Canonical)?;
    // EVM-04: eligibility is consumptive and binds the full activated prefix,
    // predecessor activation evidence, observed-prefix frontier, and frozen
    // policy. Replacement cannot self-certify without matching this preimage.
    let eligibility_ref = domain_content_digest(
        "mfm.evm.candidate-replacement-eligibility.v3",
        &(
            reservation,
            activated_candidates,
            &predecessor.activation_evidence_ref,
            next_candidate_ordinal,
            observed_prefix_len,
            &replacement_policy_ref,
            "requires_independent_observation_of_every_earlier_activated_candidate",
        ),
    )
    .map_err(|_| WalletAuthorityContractError::Canonical)?
    .as_str()
    .to_owned();
    Ok(CandidateActivationPermit::Replacement {
        predecessor_activation_ref: predecessor.activation_evidence_ref.clone(),
        predecessor_ordinal: predecessor.attested_candidate.candidate_ordinal,
        exact_next_ordinal: next_candidate_ordinal,
        replacement_policy_ref,
        eligibility_ref,
    })
}

/// Derives the exact activation permit used by the qualified wallet-authority
/// adapter.  The public domain surface intentionally omits this constructor;
/// only the storage bridge and in-crate certification tests enable it.
#[doc(hidden)]
#[cfg(any(test, feature = "authority-integration"))]
pub fn derive_exact_candidate_activation_permit(
    reservation: &ReservedWalletNonce,
    activated_candidates: &[ActiveWalletCandidate],
    next_candidate_ordinal: u16,
    observed_prefix_len: u16,
) -> Result<CandidateActivationPermit, WalletAuthorityContractError> {
    derive_exact_candidate_activation_permit_inner(
        reservation,
        activated_candidates,
        next_candidate_ordinal,
        observed_prefix_len,
    )
}

/// Reconstructs and validates one complete retained active-candidate prefix.
pub fn validate_active_wallet_candidate_prefix(
    reservation: &ReservedWalletNonce,
    transaction_intent: &EvmTransactionIntent,
    candidate_family: &EvmCandidateFamily,
    activated_candidates: &[ActiveWalletCandidate],
) -> Result<(), WalletAuthorityContractError> {
    TransactionNonce::new(reservation.nonce)?;
    transaction_intent.validate()?;
    candidate_family.validate(transaction_intent)?;
    if transaction_intent.nonce_domain() != &reservation.nonce_domain
        || transaction_intent.digest() != reservation.transaction_intent_digest
        || candidate_family.digest() != reservation.candidate_family_ref
        || activated_candidates.len() > EVM_WALLET_REPLACEMENT_LIMIT
    {
        return Err(WalletAuthorityContractError::Invalid(
            "candidate_prefix_closure",
        ));
    }

    let mut descriptor_refs = BTreeSet::new();
    let mut unsigned_digests = BTreeSet::new();
    let mut transaction_hashes = BTreeSet::new();
    for (ordinal, candidate) in activated_candidates.iter().enumerate() {
        let attested = &candidate.attested_candidate;
        let fee = candidate_family.candidates().get(ordinal).ok_or(
            WalletAuthorityContractError::Invalid("candidate_prefix_ordinal"),
        )?;
        let envelope = transaction_intent.unsigned_candidate(reservation.nonce, fee)?;
        let unsigned = UnsignedWalletCandidate {
            transaction_intent: transaction_intent.clone(),
            semantic_reservation_key: reservation.semantic_reservation_key.clone(),
            nonce: reservation.nonce,
            candidate_ordinal: u16::try_from(ordinal)
                .map_err(|_| WalletAuthorityContractError::Invalid("candidate_prefix_ordinal"))?,
            fee: fee.clone(),
            unsigned_candidate_digest: format!("{:#x}", envelope.signing_digest()),
        };
        let descriptor_ref = canonical_wallet_reference(&unsigned)?;
        if attested.semantic_reservation_key != reservation.semantic_reservation_key
            || usize::from(attested.candidate_ordinal) != ordinal
            || attested.candidate_descriptor_ref != descriptor_ref
            || attested.unsigned_candidate_digest != unsigned.unsigned_candidate_digest
            || attested.semantic_signer_id != transaction_intent.semantic_signer_id()
            || attested.signing_profile_contract_ref
                != *transaction_intent.signing_profile_contract_ref()
            || validate_reference(&attested.signer_attestation_ref).is_err()
            || validate_reference(&candidate.activation_evidence_ref).is_err()
            || !valid_provider_attestation(&candidate.provider_activation_attestation)
            || !crate::submission::validate_transaction_hash(&attested.transaction_hash)
            || !descriptor_refs.insert(attested.candidate_descriptor_ref.clone())
            || !unsigned_digests.insert(attested.unsigned_candidate_digest.clone())
            || !transaction_hashes.insert(attested.transaction_hash.clone())
        {
            return Err(WalletAuthorityContractError::Invalid(
                "candidate_prefix_closure",
            ));
        }
    }
    Ok(())
}

/// Canonical terminal execution disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm",
    name = "execution-disposition",
    version = "1",
    schema = "mfm.evm.execution_disposition"
)]
pub enum ExecutionDisposition {
    /// The included transaction succeeded.
    Succeeded,
    /// The included transaction reverted.
    Reverted,
}

fn canonical_public_result(disposition: ExecutionDisposition) -> &'static str {
    match disposition {
        ExecutionDisposition::Succeeded => "{\"execution_disposition\":\"succeeded\"}",
        ExecutionDisposition::Reverted => "{\"execution_disposition\":\"reverted\"}",
    }
}

/// Run-independent canonical completion claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "canonical-terminal-outcome",
    version = "1",
    schema = "mfm.evm.canonical_terminal_outcome"
)]
pub struct CanonicalTerminalOutcome {
    /// Exact nonce domain.
    pub nonce_domain: WalletNonceDomain,
    /// Stable reservation key.
    pub semantic_reservation_key: EvmNonceReservationKey,
    /// Stable intent id.
    pub submission_intent_id: SubmissionIntentId,
    /// Exact transaction intent digest.
    pub transaction_intent_digest: String,
    /// Permanently allocated nonce.
    pub nonce: u64,
    /// Winning activated family ordinal.
    pub winning_candidate_ordinal: u16,
    /// Winning permanent activation proof.
    pub winning_activation_evidence_ref: EvmWalletReference,
    /// Winning EVM transaction hash.
    pub transaction_hash: String,
    /// Canonical inclusion block number.
    pub inclusion_block_number: String,
    /// Canonical inclusion block hash.
    pub inclusion_block_hash: String,
    /// Exact terminal assurance policy.
    pub terminal_assurance_contract_ref: EvmWalletReference,
    /// Success or revert.
    pub execution_disposition: ExecutionDisposition,
    /// Full canonical public projection bytes.
    pub canonical_public_result: String,
}

impl<'de> Deserialize<'de> for CanonicalTerminalOutcome {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            nonce_domain: WalletNonceDomain,
            semantic_reservation_key: EvmNonceReservationKey,
            submission_intent_id: SubmissionIntentId,
            transaction_intent_digest: String,
            nonce: u64,
            winning_candidate_ordinal: u16,
            winning_activation_evidence_ref: EvmWalletReference,
            transaction_hash: String,
            inclusion_block_number: String,
            inclusion_block_hash: String,
            terminal_assurance_contract_ref: EvmWalletReference,
            execution_disposition: ExecutionDisposition,
            canonical_public_result: String,
        }
        let wire = Wire::deserialize(deserializer)?;
        TransactionNonce::new(wire.nonce).map_err(serde::de::Error::custom)?;
        Ok(Self {
            nonce_domain: wire.nonce_domain,
            semantic_reservation_key: wire.semantic_reservation_key,
            submission_intent_id: wire.submission_intent_id,
            transaction_intent_digest: wire.transaction_intent_digest,
            nonce: wire.nonce,
            winning_candidate_ordinal: wire.winning_candidate_ordinal,
            winning_activation_evidence_ref: wire.winning_activation_evidence_ref,
            transaction_hash: wire.transaction_hash,
            inclusion_block_number: wire.inclusion_block_number,
            inclusion_block_hash: wire.inclusion_block_hash,
            terminal_assurance_contract_ref: wire.terminal_assurance_contract_ref,
            execution_disposition: wire.execution_disposition,
            canonical_public_result: wire.canonical_public_result,
        })
    }
}

impl CanonicalTerminalOutcome {
    /// Revalidates the complete run-independent terminal claim.
    pub fn validate(&self) -> Result<(), WalletAuthorityContractError> {
        self.nonce_domain.validate()?;
        TransactionNonce::new(self.nonce)?;
        self.semantic_reservation_key.validate()?;
        self.submission_intent_id.validate()?;
        ContentDigest::from_str(&self.transaction_intent_digest)
            .map_err(|_| WalletAuthorityContractError::Invalid("transaction_intent_digest"))?;
        if usize::from(self.winning_candidate_ordinal) >= EVM_WALLET_REPLACEMENT_LIMIT
            || validate_reference(&self.winning_activation_evidence_ref).is_err()
            || !crate::submission::validate_transaction_hash(&self.transaction_hash)
            || !crate::submission::validate_quantity(&self.inclusion_block_number)
            || !crate::submission::validate_transaction_hash(&self.inclusion_block_hash)
            || validate_reference(&self.terminal_assurance_contract_ref).is_err()
            || self.canonical_public_result != canonical_public_result(self.execution_disposition)
        {
            return Err(WalletAuthorityContractError::Invalid(
                "canonical_terminal_outcome",
            ));
        }
        Ok(())
    }
}

/// Complete bounded producer-bound evidence for one terminal claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "terminal-witnesses",
    version = "1",
    schema = "mfm.evm.terminal_witnesses"
)]
pub struct TerminalWitnesses {
    /// Exact transaction lookup committed for the winning candidate.
    pub transaction: EvmTransactionLookupObservation,
    /// Exact receipt committed for the winning candidate.
    pub receipt: EvmReceiptLookupObservation,
    /// Exact finalized head committed after the receipt.
    pub finalized_head: EvmFinalizedHeadObservation,
    /// Fresh canonical lookup of the receipt's inclusion block.
    pub inclusion_block: EvmInclusionBlockObservation,
    /// Exact terminal-assurance policy selected by the transaction intent.
    pub terminal_assurance_contract_ref: EvmWalletReference,
    /// Full canonical public projection proved by these witnesses.
    pub canonical_public_result: String,
}

impl TerminalWitnesses {
    /// Revalidates every witness and cross-link against one canonical claim.
    pub fn validate_against(
        &self,
        outcome: &CanonicalTerminalOutcome,
    ) -> Result<(), WalletAuthorityContractError> {
        outcome.validate()?;
        let EvmTransactionLookupObservation::Found {
            transaction_hash,
            block_number: Some(transaction_block_number),
            block_hash: Some(transaction_block_hash),
        } = &self.transaction
        else {
            return Err(WalletAuthorityContractError::Invalid(
                "terminal_transaction",
            ));
        };
        let EvmReceiptLookupObservation::Found {
            transaction_hash: receipt_transaction_hash,
            block_number: receipt_block_number,
            block_hash: receipt_block_hash,
            status,
        } = &self.receipt
        else {
            return Err(WalletAuthorityContractError::Invalid("terminal_receipt"));
        };
        let finalized_number = U256::from_str(&self.finalized_head.block_number)
            .map_err(|_| WalletAuthorityContractError::Invalid("terminal_finalized_head"))?;
        let inclusion_number = U256::from_str(&self.inclusion_block.block_number)
            .map_err(|_| WalletAuthorityContractError::Invalid("terminal_inclusion_block"))?;
        let expected_status = match outcome.execution_disposition {
            ExecutionDisposition::Succeeded => 1,
            ExecutionDisposition::Reverted => 0,
        };
        if !crate::submission::validate_transaction_hash(transaction_hash)
            || !crate::submission::validate_quantity(transaction_block_number)
            || !crate::submission::validate_transaction_hash(transaction_block_hash)
            || !crate::submission::validate_transaction_hash(receipt_transaction_hash)
            || !crate::submission::validate_quantity(receipt_block_number)
            || !crate::submission::validate_transaction_hash(receipt_block_hash)
            || !matches!(status, 0 | 1)
            || !crate::submission::validate_quantity(&self.finalized_head.block_number)
            || !crate::submission::validate_transaction_hash(&self.finalized_head.block_hash)
            || !crate::submission::validate_quantity(&self.inclusion_block.block_number)
            || !crate::submission::validate_transaction_hash(&self.inclusion_block.block_hash)
            || validate_reference(&self.terminal_assurance_contract_ref).is_err()
            || transaction_hash != receipt_transaction_hash
            || transaction_hash != &outcome.transaction_hash
            || transaction_block_number != receipt_block_number
            || transaction_block_hash != receipt_block_hash
            || receipt_block_number != &self.inclusion_block.block_number
            || receipt_block_hash != &self.inclusion_block.block_hash
            || receipt_block_number != &outcome.inclusion_block_number
            || receipt_block_hash != &outcome.inclusion_block_hash
            || finalized_number < inclusion_number
            || (finalized_number == inclusion_number
                && self.finalized_head.block_hash != self.inclusion_block.block_hash)
            || *status != expected_status
            || self.terminal_assurance_contract_ref != outcome.terminal_assurance_contract_ref
            || self.canonical_public_result != outcome.canonical_public_result
            || self.canonical_public_result
                != canonical_public_result(outcome.execution_disposition)
        {
            return Err(WalletAuthorityContractError::Invalid("terminal_witnesses"));
        }
        Ok(())
    }
}

/// Permanent completion proof and the complete public recovery preimage.
///
/// The sealed activated prefix and terminal witnesses are retained as full
/// content-addressed objects (not digests with missing preimages) so a later
/// process can rehash offline and project the public result without ambient
/// reconstruction (EVM-09).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "completed-wallet-nonce",
    version = "3",
    schema = "mfm.evm.completed_wallet_nonce"
)]
pub struct CompletedWalletNonce {
    /// Exact nonce domain.
    pub nonce_domain: WalletNonceDomain,
    /// Permanently allocated nonce.
    pub nonce: u64,
    /// Stable reservation key.
    pub semantic_reservation_key: EvmNonceReservationKey,
    /// Stable completion key.
    pub semantic_completion_key: EvmNonceCompletionKey,
    /// Canonical run-independent outcome.
    pub canonical_terminal_outcome: CanonicalTerminalOutcome,
    /// Complete terminal-witness preimage proving the outcome.
    pub terminal_witnesses: TerminalWitnesses,
    /// Sealed activated prefix at the completion linearization point.
    pub sealed_activated_candidates: Vec<ActiveWalletCandidate>,
    /// Content digest of [`Self::terminal_witnesses`] (audit provenance).
    pub original_terminal_witnesses_ref: String,
    /// Permanent completion evidence.
    pub completion_evidence_ref: EvmWalletReference,
    /// Provider-issued terminal mutation attestation retained for recovery.
    pub provider_completion_attestation: String,
    /// Canonical, bounded full recovery closure.
    pub recovery_closure: String,
}

impl CompletedWalletNonce {
    /// Constructs a completion with its full bounded recovery preimage.
    #[allow(clippy::too_many_arguments)]
    pub fn with_recovery_closure(
        nonce_domain: WalletNonceDomain,
        nonce: u64,
        semantic_reservation_key: EvmNonceReservationKey,
        semantic_completion_key: EvmNonceCompletionKey,
        canonical_terminal_outcome: CanonicalTerminalOutcome,
        terminal_witnesses: TerminalWitnesses,
        sealed_activated_candidates: Vec<ActiveWalletCandidate>,
        original_terminal_witnesses_ref: String,
        completion_evidence_ref: EvmWalletReference,
        provider_completion_attestation: String,
        reservation: ReservedWalletNonce,
        transaction_intent: EvmTransactionIntent,
        candidate_family: EvmCandidateFamily,
        domain_activation_attestation: WalletNonceDomainActivationAttestation,
        qualified_floor: QualifiedPendingNonceFloor,
        route_generation_ref: EvmWalletReference,
        issuer_namespace_contract_ref: EvmWalletReference,
        observation_rounds: u8,
        submission_semantics_digest: SubmissionSemanticsDigest,
        reservation_request: ReserveEvmNonceRequest,
        completion_request: CompleteEvmNonceRequest,
        activation_requests: Vec<ActivateEvmCandidateRequest>,
        activation_state_inputs: Vec<LexicalValueRef>,
        reservation_state_input: LexicalValueRef,
        completion_state_input: LexicalValueRef,
    ) -> Result<Self, WalletAuthorityContractError> {
        let recovery_closure = encode_completed_recovery(&CompletedRecoveryClosure {
            reservation_request,
            completion_request,
            activation_requests,
            activation_state_inputs,
            reservation_state_input,
            completion_state_input,
            reservation,
            transaction_intent,
            candidate_family,
            domain_activation_attestation,
            qualified_floor,
            route_generation_ref,
            issuer_namespace_contract_ref,
            observation_rounds,
            submission_semantics_digest,
            canonical_terminal_outcome: canonical_terminal_outcome.clone(),
            terminal_witnesses: terminal_witnesses.clone(),
            sealed_activated_candidates: sealed_activated_candidates.clone(),
            provider_completion_attestation: provider_completion_attestation.clone(),
            original_terminal_witnesses_ref: original_terminal_witnesses_ref.clone(),
            completion_evidence_ref: completion_evidence_ref.clone(),
        })?;
        Ok(Self {
            nonce_domain,
            nonce,
            semantic_reservation_key,
            semantic_completion_key,
            canonical_terminal_outcome,
            terminal_witnesses,
            sealed_activated_candidates,
            original_terminal_witnesses_ref,
            completion_evidence_ref,
            provider_completion_attestation,
            recovery_closure,
        })
    }

    /// Rehydrates one completion wrapper from its persisted recovery closure.
    ///
    /// The closure is the complete recovery preimage; the public wrapper fields
    /// are reconstructed from it and then revalidated before the value is
    /// returned. This lets a later process recover without the original outer
    /// value or any ambient provider/store access.
    pub fn from_recovery_closure(
        recovery_closure: &str,
    ) -> Result<Self, WalletAuthorityContractError> {
        let closure = decode_completed_recovery(recovery_closure)?;
        let completion = Self {
            nonce_domain: closure.reservation.nonce_domain.clone(),
            nonce: closure.reservation.nonce,
            semantic_reservation_key: closure.reservation.semantic_reservation_key.clone(),
            semantic_completion_key: closure.completion_request.completion_key.clone(),
            canonical_terminal_outcome: closure.canonical_terminal_outcome.clone(),
            terminal_witnesses: closure.terminal_witnesses.clone(),
            sealed_activated_candidates: closure.sealed_activated_candidates.clone(),
            original_terminal_witnesses_ref: closure.original_terminal_witnesses_ref.clone(),
            completion_evidence_ref: closure.completion_evidence_ref.clone(),
            provider_completion_attestation: closure.provider_completion_attestation.clone(),
            recovery_closure: recovery_closure.to_owned(),
        };
        completion.validate()?;
        Ok(completion)
    }

    /// Projects the canonical public result from persisted closure bytes alone.
    pub fn project_public_result_from_recovery_closure(
        recovery_closure: &str,
    ) -> Result<String, WalletAuthorityContractError> {
        Ok(Self::from_recovery_closure(recovery_closure)?
            .canonical_terminal_outcome
            .canonical_public_result)
    }

    /// Adds the provider attestation returned for the exact terminal mutation.
    ///
    /// The mutation is prepared before its SQL row is inserted, so the provider
    /// proof is attached immediately after that preparation and before the
    /// completion becomes durable.
    pub fn with_provider_completion_attestation(
        mut self,
        provider_completion_attestation: String,
    ) -> Result<Self, WalletAuthorityContractError> {
        if !valid_provider_attestation(&provider_completion_attestation) {
            return Err(WalletAuthorityContractError::Invalid(
                "provider_completion_attestation",
            ));
        }
        let mut closure = decode_completed_recovery(&self.recovery_closure)?;
        closure.provider_completion_attestation = provider_completion_attestation.clone();
        self.recovery_closure = encode_completed_recovery(&closure)?;
        self.provider_completion_attestation = provider_completion_attestation;
        Ok(self)
    }

    /// Returns the exact completion value that was sent to the provider before
    /// its returned attestation was attached to the durable closure.
    ///
    /// The result is a transient verification preimage. It is intentionally
    /// incomplete and must not be persisted or returned as a completion.
    #[doc(hidden)]
    pub fn provider_mutation_preimage(&self) -> Result<Self, WalletAuthorityContractError> {
        let mut preimage = self.clone();
        let mut closure = decode_completed_recovery(&self.recovery_closure)?;
        closure.provider_completion_attestation.clear();
        preimage.recovery_closure = encode_completed_recovery(&closure)?;
        preimage.provider_completion_attestation.clear();
        Ok(preimage)
    }

    /// Revalidates the complete public recovery closure against itself.
    pub fn validate(&self) -> Result<(), WalletAuthorityContractError> {
        self.nonce_domain.validate()?;
        TransactionNonce::new(self.nonce)?;
        self.semantic_reservation_key.validate()?;
        self.semantic_completion_key.validate()?;
        let closure = decode_completed_recovery(&self.recovery_closure)?;
        let reservation = &closure.reservation;
        let reservation_request = &closure.reservation_request;
        let completion_request = &closure.completion_request;
        let transaction_intent = &closure.transaction_intent;
        let candidate_family = &closure.candidate_family;
        let domain_activation_attestation = &closure.domain_activation_attestation;
        let qualified_floor = &closure.qualified_floor;
        if !valid_provider_attestation(&closure.provider_completion_attestation)
            || closure.provider_completion_attestation != self.provider_completion_attestation
        {
            return Err(WalletAuthorityContractError::Invalid(
                "completed_wallet_nonce_provider_attestation",
            ));
        }
        reservation.validate()?;
        completion_request.validate()?;
        transaction_intent.validate()?;
        candidate_family.validate(transaction_intent)?;
        domain_activation_attestation.validate()?;
        qualified_floor.observed.pending_nonce.validate()?;
        qualified_floor.observed.nonce_domain.validate()?;
        validate_reference(&qualified_floor.observed.route_generation_ref)?;
        validate_reference(&qualified_floor.pending_floor_policy_ref)?;
        validate_reference(&closure.route_generation_ref)?;
        validate_reference(&closure.issuer_namespace_contract_ref)?;
        if !(1..=EVM_WALLET_OBSERVATION_ROUND_LIMIT).contains(&closure.observation_rounds) {
            return Err(WalletAuthorityContractError::Invalid("observation_rounds"));
        }
        let activation_record = &domain_activation_attestation.current_schema_record;
        if qualified_floor.observed.nonce_domain != self.nonce_domain
            || transaction_intent.nonce_domain() != &self.nonce_domain
            || activation_record.wallet_nonce_domain != self.nonce_domain
            || activation_record.issuer_namespace_contract_ref
                != closure.issuer_namespace_contract_ref
            || activation_record
                .initial_route_generation_ref
                .to_content_ref()
                .ok()
                .as_ref()
                != closure.route_generation_ref.to_content_ref().ok().as_ref()
        {
            return Err(WalletAuthorityContractError::Invalid(
                "completed_wallet_nonce_scope",
            ));
        }
        if reservation_request.nonce_domain != self.nonce_domain
            || reservation_request.domain_activation_attestation != *domain_activation_attestation
            || reservation_request.issuer_namespace_contract_ref
                != closure.issuer_namespace_contract_ref
            || reservation_request.submission_intent_id != reservation.submission_intent_id
            || reservation_request.submission_semantics_digest
                != closure.submission_semantics_digest
            || reservation_request.observation_rounds != closure.observation_rounds
            || reservation_request.transaction_intent != *transaction_intent
            || reservation_request.candidate_family != *candidate_family
            || reservation_request.qualified_floor != *qualified_floor
            || reservation_request.reservation_key != reservation.semantic_reservation_key
            || completion_request.nonce_domain != self.nonce_domain
            || completion_request.completion_key != self.semantic_completion_key
            || completion_request.current_reservation != *reservation
            || completion_request.canonical_terminal_outcome != self.canonical_terminal_outcome
            || completion_request.terminal_witnesses != self.terminal_witnesses
            || closure.activation_requests.len() != self.sealed_activated_candidates.len()
            || closure.activation_state_inputs.len() != closure.activation_requests.len()
        {
            return Err(WalletAuthorityContractError::Invalid(
                "completed_wallet_nonce_requests",
            ));
        }
        let expansion_contract_ref = crate::evm_submission_expansion_policy_ref()
            .map_err(|_| WalletAuthorityContractError::Invalid("expansion_contract"))?;
        let expected_semantics = derive_submission_semantics_digest(
            transaction_intent,
            candidate_family,
            closure.observation_rounds,
            &expansion_contract_ref,
            &closure.route_generation_ref,
            domain_activation_attestation,
            &closure.issuer_namespace_contract_ref,
        )?;
        let activation_record_ref = canonical_wallet_reference(activation_record)?;
        if expected_semantics != closure.submission_semantics_digest
            || reservation.semantic_reservation_key != self.semantic_reservation_key
            || reservation.nonce_domain != self.nonce_domain
            || reservation.nonce != self.nonce
            || reservation.transaction_intent_digest != transaction_intent.digest()
            || reservation.candidate_family_ref != candidate_family.digest()
            || activation_record_ref != reservation.domain_activation_record_ref
            || qualified_floor.observed.route_generation_ref != closure.route_generation_ref
        {
            return Err(WalletAuthorityContractError::Invalid(
                "completed_wallet_nonce_preimages",
            ));
        }
        let observed_floor_ref = domain_content_digest(
            "mfm.evm.wallet-observed-floor-provenance.v1",
            &(
                &reservation_request.qualified_floor,
                &closure.reservation_state_input,
            ),
        )
        .map_err(|_| WalletAuthorityContractError::Canonical)?;
        let reservation_evidence_ref = domain_content_digest(
            "mfm.evm.wallet-reservation-evidence.v1",
            &(
                reservation_request,
                &closure.reservation_state_input,
                &reservation.resource_lineage_ref,
                reservation.nonce,
            ),
        )
        .map_err(|_| WalletAuthorityContractError::Canonical)?;
        if reservation.observed_floor_ref != observed_floor_ref.as_str()
            || reservation.reservation_evidence_ref.content_digest()
                != reservation_evidence_ref.as_str()
        {
            return Err(WalletAuthorityContractError::Invalid(
                "completed_wallet_nonce_reservation_evidence",
            ));
        }
        for (ordinal, ((request, state_input), candidate)) in closure
            .activation_requests
            .iter()
            .zip(&closure.activation_state_inputs)
            .zip(&self.sealed_activated_candidates)
            .enumerate()
        {
            let expected_permit = derive_exact_candidate_activation_permit_inner(
                reservation,
                &self.sealed_activated_candidates[..ordinal],
                request.next_candidate.candidate_ordinal,
                request.next_candidate.candidate_ordinal,
            )?;
            let expected_activation = domain_content_digest(
                "mfm.evm.wallet-candidate-activation-evidence.v1",
                &(request, state_input, &reservation.resource_lineage_ref),
            )
            .map_err(|_| WalletAuthorityContractError::Canonical)?;
            let expected_operation_key = derive_evm_candidate_operation_key(
                &reservation.semantic_reservation_key,
                request.next_candidate.candidate_ordinal,
            )?;
            if request.next_candidate != candidate.attested_candidate
                || request.nonce_domain != self.nonce_domain
                || request.activation_permit != expected_permit
                || request.candidate_operation_key != expected_operation_key
                || candidate.activation_evidence_ref.content_digest()
                    != expected_activation.as_str()
            {
                return Err(WalletAuthorityContractError::Invalid(
                    "completed_wallet_nonce_activation_evidence",
                ));
            }
        }
        let completion_evidence_ref = domain_content_digest(
            "mfm.evm.wallet-completion-evidence.v1",
            &(
                completion_request,
                &closure.completion_state_input,
                &reservation.resource_lineage_ref,
            ),
        )
        .map_err(|_| WalletAuthorityContractError::Canonical)?;
        if closure.completion_evidence_ref.content_digest() != completion_evidence_ref.as_str() {
            return Err(WalletAuthorityContractError::Invalid(
                "completed_wallet_nonce_completion_evidence",
            ));
        }
        if closure.canonical_terminal_outcome != self.canonical_terminal_outcome
            || closure.terminal_witnesses != self.terminal_witnesses
            || closure.sealed_activated_candidates != self.sealed_activated_candidates
            || closure.original_terminal_witnesses_ref != self.original_terminal_witnesses_ref
            || closure.completion_evidence_ref != self.completion_evidence_ref
        {
            return Err(WalletAuthorityContractError::Invalid(
                "completed_wallet_nonce_terminal_closure",
            ));
        }
        if reservation.submission_intent_id != self.canonical_terminal_outcome.submission_intent_id
        {
            return Err(WalletAuthorityContractError::Invalid(
                "completed_wallet_nonce_intent",
            ));
        }
        validate_active_wallet_candidate_prefix(
            reservation,
            transaction_intent,
            candidate_family,
            &self.sealed_activated_candidates,
        )?;
        self.canonical_terminal_outcome.validate()?;
        self.terminal_witnesses
            .validate_against(&self.canonical_terminal_outcome)?;
        let witnesses_ref = canonical_wallet_reference(&self.terminal_witnesses)?;
        if self.nonce_domain != self.canonical_terminal_outcome.nonce_domain
            || self.nonce != self.canonical_terminal_outcome.nonce
            || self.semantic_reservation_key
                != self.canonical_terminal_outcome.semantic_reservation_key
            || derive_evm_nonce_completion_key(&self.semantic_reservation_key)?
                != self.semantic_completion_key
            || self.original_terminal_witnesses_ref != witnesses_ref.content_digest()
            || validate_reference(&self.completion_evidence_ref).is_err()
            || !valid_provider_attestation(&self.provider_completion_attestation)
            || self.sealed_activated_candidates.is_empty()
            || self.sealed_activated_candidates.len() > EVM_WALLET_REPLACEMENT_LIMIT
            || !self.sealed_activated_candidates.iter().any(|candidate| {
                candidate.attested_candidate.candidate_ordinal
                    == self.canonical_terminal_outcome.winning_candidate_ordinal
                    && candidate.attested_candidate.transaction_hash
                        == self.canonical_terminal_outcome.transaction_hash
                    && candidate.activation_evidence_ref
                        == self
                            .canonical_terminal_outcome
                            .winning_activation_evidence_ref
            })
        {
            return Err(WalletAuthorityContractError::Invalid(
                "completed_wallet_nonce_closure",
            ));
        }
        for (ordinal, candidate) in self.sealed_activated_candidates.iter().enumerate() {
            if usize::from(candidate.attested_candidate.candidate_ordinal) != ordinal
                || candidate.attested_candidate.semantic_reservation_key
                    != self.semantic_reservation_key
                || validate_reference(&candidate.activation_evidence_ref).is_err()
                || !valid_provider_attestation(&candidate.provider_activation_attestation)
            {
                return Err(WalletAuthorityContractError::Invalid(
                    "completed_wallet_nonce_prefix",
                ));
            }
        }
        Ok(())
    }
}

fn valid_provider_attestation(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_PROVIDER_PROOF_BYTES
        && value.bytes().all(|byte| byte.is_ascii_graphic())
}

#[cfg(test)]
mod provider_attestation_tests {
    use super::{valid_provider_attestation, MAX_PROVIDER_PROOF_BYTES};

    #[test]
    fn provider_attestation_accepts_exact_budget_and_rejects_one_byte_over() {
        let exact = "x".repeat(MAX_PROVIDER_PROOF_BYTES);
        assert!(valid_provider_attestation(&exact));

        let one_byte_over = format!("{exact}x");
        assert!(!valid_provider_attestation(&one_byte_over));
    }
}

/// One transactionally consistent wallet-nonce status snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-nonce-status",
    version = "1",
    schema = "mfm.evm.wallet_nonce_status"
)]
// The public status is a direct current-schema value; heap-indirection fields
// would alter the certified MfmValue contract.
#[allow(clippy::large_enum_variant)]
pub enum WalletNonceStatus {
    /// The exact intent has no reservation and no foreign intent is active.
    Absent,
    /// Another incomplete intent owns the domain; no payload is disclosed.
    Busy,
    /// The exact intent has an incomplete reservation.
    Reserved {
        /// Permanent reservation.
        reservation: ReservedWalletNonce,
        /// Complete nonce-free transaction intent.
        transaction_intent: EvmTransactionIntent,
        /// Complete bounded candidate family.
        candidate_family: EvmCandidateFamily,
        /// Complete contiguous activated prefix.
        activated_candidates: Vec<ActiveWalletCandidate>,
        /// Exact current candidate, equal to the last prefix member when present.
        current_candidate: Option<ActiveWalletCandidate>,
        /// Current public resource head.
        resource_head_ref: EvmWalletReference,
    },
    /// The exact intent has a canonical permanent completion.
    Completed {
        /// Permanent reservation.
        reservation: ReservedWalletNonce,
        /// Complete nonce-free transaction intent.
        transaction_intent: EvmTransactionIntent,
        /// Complete bounded candidate family.
        candidate_family: EvmCandidateFamily,
        /// Sealed activated prefix.
        activated_candidates: Vec<ActiveWalletCandidate>,
        /// Canonical permanent completion.
        completion: CompletedWalletNonce,
        /// Current public resource head.
        resource_head_ref: EvmWalletReference,
    },
}

/// Exact purpose-limited status request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "read-wallet-nonce-status-request",
    version = "1",
    schema = "mfm.evm.read_wallet_nonce_status_request"
)]
pub struct ReadEvmWalletNonceStatusRequest {
    /// Exact nonce domain.
    pub nonce_domain: WalletNonceDomain,
    /// Secret-free permanent activation attestation.
    pub domain_activation_attestation: WalletNonceDomainActivationAttestation,
    /// Immutable issuer namespace included in the behavior identity.
    pub issuer_namespace_contract_ref: EvmWalletReference,
    /// Stable reservation key.
    pub semantic_reservation_key: EvmNonceReservationKey,
    /// Stable authenticated intent.
    pub submission_intent_id: SubmissionIntentId,
    /// Complete behavior identity for this intent.
    pub submission_semantics_digest: SubmissionSemanticsDigest,
    /// Exact transaction intent digest.
    pub transaction_intent_digest: String,
    /// Exact candidate family digest.
    pub candidate_family_ref: String,
    /// Complete transaction-intent preimage for behavior revalidation.
    pub transaction_intent: EvmTransactionIntent,
    /// Complete candidate-family preimage for behavior revalidation.
    pub candidate_family: EvmCandidateFamily,
    /// Exact bounded observation-round policy.
    pub observation_rounds: u8,
    /// Exact route generation included in the behavior identity.
    pub route_generation_ref: EvmWalletReference,
}

/// Exact idempotent reservation request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "reserve-evm-nonce-request",
    version = "1",
    schema = "mfm.evm.reserve_nonce_request"
)]
pub struct ReserveEvmNonceRequest {
    /// Exact nonce domain.
    pub nonce_domain: WalletNonceDomain,
    /// Secret-free permanent activation attestation.
    pub domain_activation_attestation: WalletNonceDomainActivationAttestation,
    /// Immutable issuer namespace included in the behavior identity.
    pub issuer_namespace_contract_ref: EvmWalletReference,
    /// Stable authenticated intent.
    pub submission_intent_id: SubmissionIntentId,
    /// Complete behavior identity for this intent.
    pub submission_semantics_digest: SubmissionSemanticsDigest,
    /// Complete nonce-free transaction intent.
    pub transaction_intent: EvmTransactionIntent,
    /// Complete bounded candidate family.
    pub candidate_family: EvmCandidateFamily,
    /// Exact observation-round policy included in the semantics digest.
    pub observation_rounds: u8,
    /// Stable reservation key.
    pub reservation_key: EvmNonceReservationKey,
    /// Fresh qualified pending floor.
    pub qualified_floor: QualifiedPendingNonceFloor,
}

/// Definite no-mutation reservation disposition or permanent result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "reserve-wallet-nonce-response",
    version = "1",
    schema = "mfm.evm.reserve_wallet_nonce_response"
)]
// The response payload remains directly nested in its certified schema.
#[allow(clippy::large_enum_variant)]
pub enum ReserveWalletNonceResponse {
    /// Permanent reservation applied or resolved identically.
    Reserved {
        /// Exact permanent reservation proof.
        reservation: ReservedWalletNonce,
    },
    /// Another intent is incomplete.
    NonceDomainBusy,
    /// Provider pending state diverged from the exclusive local lineage.
    NonceLineageDiverged,
    /// The nonce cannot advance without overflow.
    NonceCapacityExhausted,
}

/// Exact idempotent candidate activation request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "activate-evm-candidate-request",
    version = "1",
    schema = "mfm.evm.activate_candidate_request"
)]
pub struct ActivateEvmCandidateRequest {
    /// Exact nonce domain.
    pub nonce_domain: WalletNonceDomain,
    /// Stable candidate operation key.
    pub candidate_operation_key: EvmCandidateOperationKey,
    /// Exact attested next candidate.
    pub next_candidate: AttestedWalletCandidate,
    /// Exact closed progression permit.
    pub activation_permit: CandidateActivationPermit,
}

/// Payload-free no-mutation progression conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "candidate-progression-conflict",
    version = "1",
    schema = "mfm.evm.candidate_progression_conflict"
)]
pub struct CandidateProgressionConflict {}

/// Candidate activation result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "activate-candidate-response",
    version = "1",
    schema = "mfm.evm.activate_candidate_response"
)]
// The response payload remains directly nested in its certified schema.
#[allow(clippy::large_enum_variant)]
pub enum ActivateCandidateResponse {
    /// Permanent candidate activation applied or resolved identically.
    Activated {
        /// Exact permanent candidate activation proof.
        candidate: ActiveWalletCandidate,
    },
    /// The exact candidate was already retained by an earlier activation
    /// commit. Callers must observe its chain status; they must not broadcast
    /// it again.
    AlreadyRetained {
        /// Exact retained candidate proof.
        candidate: ActiveWalletCandidate,
    },
    /// Another contender changed the prefix first.
    CandidateProgressionConflict,
}

/// Exact idempotent canonical completion request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "complete-evm-nonce-request",
    version = "1",
    schema = "mfm.evm.complete_nonce_request"
)]
pub struct CompleteEvmNonceRequest {
    /// Exact nonce domain.
    pub nonce_domain: WalletNonceDomain,
    /// Stable completion key.
    pub completion_key: EvmNonceCompletionKey,
    /// Exact current reservation.
    pub current_reservation: ReservedWalletNonce,
    /// Canonical run-independent outcome.
    pub canonical_terminal_outcome: CanonicalTerminalOutcome,
    /// Complete bounded producer-bound terminal witness closure.
    pub terminal_witnesses: TerminalWitnesses,
}

impl CompleteEvmNonceRequest {
    /// Revalidates the complete canonical completion request closure.
    pub fn validate(&self) -> Result<(), WalletAuthorityContractError> {
        self.nonce_domain.validate()?;
        self.completion_key.validate()?;
        self.current_reservation.validate()?;
        self.current_reservation
            .semantic_reservation_key
            .validate()?;
        self.current_reservation.submission_intent_id.validate()?;
        ContentDigest::from_str(&self.current_reservation.transaction_intent_digest)
            .map_err(|_| WalletAuthorityContractError::Invalid("transaction_intent_digest"))?;
        ContentDigest::from_str(&self.current_reservation.candidate_family_ref)
            .map_err(|_| WalletAuthorityContractError::Invalid("candidate_family_ref"))?;
        ContentDigest::from_str(&self.current_reservation.observed_floor_ref)
            .map_err(|_| WalletAuthorityContractError::Invalid("observed_floor_ref"))?;
        validate_reference(&self.current_reservation.domain_activation_record_ref)?;
        validate_reference(&self.current_reservation.resource_lineage_ref)?;
        validate_reference(&self.current_reservation.reservation_evidence_ref)?;
        self.canonical_terminal_outcome.validate()?;
        self.terminal_witnesses
            .validate_against(&self.canonical_terminal_outcome)?;
        if self.current_reservation.nonce_domain != self.nonce_domain
            || self.canonical_terminal_outcome.nonce_domain != self.nonce_domain
            || self.current_reservation.semantic_reservation_key
                != self.canonical_terminal_outcome.semantic_reservation_key
            || self.current_reservation.submission_intent_id
                != self.canonical_terminal_outcome.submission_intent_id
            || self.current_reservation.transaction_intent_digest
                != self.canonical_terminal_outcome.transaction_intent_digest
            || self.current_reservation.nonce != self.canonical_terminal_outcome.nonce
            || derive_evm_nonce_completion_key(&self.current_reservation.semantic_reservation_key)?
                != self.completion_key
        {
            return Err(WalletAuthorityContractError::Invalid(
                "completion_request_closure",
            ));
        }
        Ok(())
    }
}

/// Canonical completion response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "complete-wallet-nonce-response",
    version = "1",
    schema = "mfm.evm.complete_wallet_nonce_response"
)]
pub enum CompleteWalletNonceResponse {
    /// Permanent completion applied or resolved identically.
    Completed {
        /// Exact permanent completion proof.
        completion: CompletedWalletNonce,
    },
}

/// Monotonic public head for refreshable wallet-nonce mutations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "wallet-nonce-store-lineage-head",
    version = "1",
    schema = "mfm.evm.wallet_nonce_store_lineage_head"
)]
pub struct WalletNonceStoreLineageHead {
    /// Stable physical store lineage.
    pub wallet_nonce_store_lineage_id: String,
    /// Monotonic writer epoch.
    pub writer_epoch: u64,
    /// Current public lineage head certificate.
    pub public_lineage_head_ref: EvmWalletReference,
}

impl WalletNonceStoreLineageHead {
    /// Revalidates the monotonic public resource-head identity.
    pub fn validate(&self) -> Result<(), WalletAuthorityContractError> {
        StableId::new(&self.wallet_nonce_store_lineage_id)
            .map_err(|_| WalletAuthorityContractError::Invalid("store_lineage_id"))?;
        if self.writer_epoch == 0 {
            return Err(WalletAuthorityContractError::Invalid("writer_epoch"));
        }
        validate_reference(&self.public_lineage_head_ref)
    }
}

/// Narrow target-bound wallet nonce authority used only by registered adapters.
pub trait WalletNonceAuthority:
    mfm_authority_seal::WalletNonceAuthoritySeal + Send + Sync + 'static
{
    /// Returns the exact provider-qualified activation sealed into this authority.
    fn domain_activation_attestation(&self) -> &WalletNonceDomainActivationAttestation;

    /// Returns the provider-issued current physical store incarnation.
    ///
    /// This is intentionally separate from the activation's historical initial
    /// incarnation. Promotion changes this value without rewriting activation.
    fn current_incarnation_ref(&self) -> EvmWalletReference;

    /// Reads one transactionally consistent exact-intent status.
    fn read_status<'a>(
        &'a self,
        _state_input_ref: &'a LexicalValueRef,
        _request: &'a ReadEvmWalletNonceStatusRequest,
    ) -> ComponentFuture<'a, ReadAdapterCompletion<WalletNonceStatus, EvmSubmissionFailure>> {
        Box::pin(std::future::ready(ReadAdapterCompletion::SafeFailure(
            EvmSubmissionFailure::NonceLineageDiverged,
        )))
    }

    /// Reads status only after consuming the exact committed Runtime
    /// authorization that originated the request.
    fn read_status_authorized<'a>(
        &'a self,
        state_input_ref: &'a LexicalValueRef,
        request: &'a ReadEvmWalletNonceStatusRequest,
        authorization: CertifiedAccessAuthorization,
    ) -> ComponentFuture<'a, ReadAdapterCompletion<WalletNonceStatus, EvmSubmissionFailure>> {
        let _ = (state_input_ref, request, authorization);
        Box::pin(std::future::ready(ReadAdapterCompletion::SafeFailure(
            EvmSubmissionFailure::NonceLineageDiverged,
        )))
    }

    /// Applies a reservation after the live adapter has paired the request
    /// with producer-bound pending-floor evidence.
    fn reserve_qualified<'a>(
        &'a self,
        state_input_ref: &'a LexicalValueRef,
        observation: &'a QualifiedPendingNonceObservation,
        request: &'a ReserveEvmNonceRequest,
        authorization: CertifiedAccessAuthorization,
    ) -> ComponentFuture<
        'a,
        EffectAdapterCompletion<
            ReserveWalletNonceResponse,
            EvmSubmissionFailure,
            WalletNonceStoreLineageHead,
        >,
    >;

    /// Explicitly denied legacy mutation entry point.
    ///
    /// Candidate mutation is only available through the Runtime-authorized
    /// method. This retained method exists solely so stale external callers
    /// receive a typed denial instead of finding an ambient mutation path.
    #[doc(hidden)]
    fn activate_candidate<'a>(
        &'a self,
        _state_input_ref: &'a LexicalValueRef,
        _request: &'a ActivateEvmCandidateRequest,
    ) -> ComponentFuture<
        'a,
        EffectAdapterCompletion<
            ActivateCandidateResponse,
            EvmSubmissionFailure,
            WalletNonceStoreLineageHead,
        >,
    > {
        Box::pin(std::future::ready(EffectAdapterCompletion::SafeFailure(
            EvmSubmissionFailure::NonceLineageDiverged,
        )))
    }

    /// Explicitly denied legacy mutation entry point.
    #[doc(hidden)]
    fn complete<'a>(
        &'a self,
        _state_input_ref: &'a LexicalValueRef,
        _request: &'a CompleteEvmNonceRequest,
    ) -> ComponentFuture<
        'a,
        EffectAdapterCompletion<
            CompleteWalletNonceResponse,
            EvmSubmissionFailure,
            WalletNonceStoreLineageHead,
        >,
    > {
        Box::pin(std::future::ready(EffectAdapterCompletion::SafeFailure(
            EvmSubmissionFailure::NonceLineageDiverged,
        )))
    }

    /// Applies one candidate activation with the exact committed Runtime
    /// authorization still attached to the authority call.
    fn activate_candidate_authorized<'a>(
        &'a self,
        state_input_ref: &'a LexicalValueRef,
        request: &'a ActivateEvmCandidateRequest,
        authorization: CertifiedAccessAuthorization,
    ) -> ComponentFuture<
        'a,
        EffectAdapterCompletion<
            ActivateCandidateResponse,
            EvmSubmissionFailure,
            WalletNonceStoreLineageHead,
        >,
    >;

    /// Applies one completion with the exact committed Runtime authorization
    /// still attached to the authority call.
    fn complete_authorized<'a>(
        &'a self,
        state_input_ref: &'a LexicalValueRef,
        request: &'a CompleteEvmNonceRequest,
        authorization: CertifiedAccessAuthorization,
    ) -> ComponentFuture<
        'a,
        EffectAdapterCompletion<
            CompleteWalletNonceResponse,
            EvmSubmissionFailure,
            WalletNonceStoreLineageHead,
        >,
    >;

    /// Derives the public history object for supersession evidence through
    /// this same sealed target-bound authority.
    fn supersession_head<'a>(
        &'a self,
        evidence: &'a WalletNonceStoreLineageHead,
    ) -> ComponentFuture<'a, Option<mfm_journal::structured::HistoryObject>>;
}

/// Domain-owned effect contract used by the live wallet binding.
///
/// Keeping the authorization-origin conversion beside the private observation
/// constructor means a live adapter cannot mint producer-bound pending evidence
/// from a raw floor. The binding only supplies the committed record and the
/// already checked authorization payload selected by Runtime.
pub trait WalletEffectSpec: RuntimeEffectCapability {
    /// Returns the semantic adapter contract for this capability.
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract>;

    /// Invokes the ordinary unqualified effect path.
    fn invoke<'a>(
        authority: &'a dyn WalletNonceAuthority,
        state_input_ref: &'a LexicalValueRef,
        request: &'a Self::Request,
    ) -> ComponentFuture<'a, EffectContractCompletion<Self>>;

    /// Invokes the effect after Runtime has supplied the exact committed
    /// authorization origin.
    fn invoke_authorized<'a>(
        authority: &'a dyn WalletNonceAuthority,
        state_input_ref: &'a LexicalValueRef,
        request: &'a Self::Request,
        authorization: CertifiedAccessAuthorization,
        integrity_fault: &'a AccessFaultCode,
    ) -> ComponentFuture<'a, EffectContractCompletion<Self>> {
        let _ = (authority, state_input_ref, request, authorization);
        Box::pin(std::future::ready(EffectAdapterCompletion::IntegrityFault(
            integrity_fault.clone(),
        )))
    }
}

impl WalletEffectSpec for ReserveWalletNonceCapability {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        reserve_wallet_nonce_adapter_contract()
    }

    fn invoke<'a>(
        authority: &'a dyn WalletNonceAuthority,
        state_input_ref: &'a LexicalValueRef,
        request: &'a ReserveEvmNonceRequest,
    ) -> ComponentFuture<'a, EffectContractCompletion<ReserveWalletNonceCapability>> {
        let _ = (authority, state_input_ref, request);
        Box::pin(std::future::ready(EffectAdapterCompletion::SafeFailure(
            EvmSubmissionFailure::NonceLineageDiverged,
        )))
    }

    fn invoke_authorized<'a>(
        authority: &'a dyn WalletNonceAuthority,
        state_input_ref: &'a LexicalValueRef,
        request: &'a ReserveEvmNonceRequest,
        authorization: CertifiedAccessAuthorization,
        integrity_fault: &'a AccessFaultCode,
    ) -> ComponentFuture<'a, EffectContractCompletion<ReserveWalletNonceCapability>> {
        let chain_instance_ref = match request
            .domain_activation_attestation
            .current_schema_record
            .chain_instance_attestation
            .content_ref()
        {
            Ok(reference) => reference,
            Err(_) => {
                return Box::pin(std::future::ready(EffectAdapterCompletion::IntegrityFault(
                    integrity_fault.clone(),
                )));
            }
        };
        let sender = match Address::from_str(request.nonce_domain.sender()) {
            Ok(sender) => sender,
            Err(_) => {
                return Box::pin(std::future::ready(EffectAdapterCompletion::IntegrityFault(
                    integrity_fault.clone(),
                )));
            }
        };
        let observation = QualifiedPendingNonceObservation::from_committed_origin(
            request.qualified_floor.clone(),
            chain_instance_ref,
            sender,
            authority.current_incarnation_ref(),
            authorization.authorization_ref().run_id.clone(),
            authorization.authorization_ref().clone(),
            authorization.request_value_ref().clone(),
        );
        let Ok(observation) = observation else {
            return Box::pin(std::future::ready(EffectAdapterCompletion::IntegrityFault(
                integrity_fault.clone(),
            )));
        };
        Box::pin(async move {
            authority
                .reserve_qualified(state_input_ref, &observation, request, authorization)
                .await
        })
    }
}

impl WalletEffectSpec for ActivateWalletCandidateCapability {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        activate_wallet_candidate_adapter_contract()
    }

    fn invoke<'a>(
        _authority: &'a dyn WalletNonceAuthority,
        _state_input_ref: &'a LexicalValueRef,
        _request: &'a ActivateEvmCandidateRequest,
    ) -> ComponentFuture<'a, EffectContractCompletion<ActivateWalletCandidateCapability>> {
        // Mutation has no unqualified path. Runtime must supply the affine
        // committed authorization to `invoke_authorized`.
        Box::pin(std::future::ready(EffectAdapterCompletion::SafeFailure(
            EvmSubmissionFailure::NonceLineageDiverged,
        )))
    }

    fn invoke_authorized<'a>(
        authority: &'a dyn WalletNonceAuthority,
        state_input_ref: &'a LexicalValueRef,
        request: &'a ActivateEvmCandidateRequest,
        authorization: CertifiedAccessAuthorization,
        integrity_fault: &'a AccessFaultCode,
    ) -> ComponentFuture<'a, EffectContractCompletion<ActivateWalletCandidateCapability>> {
        if authorization.authorization_ref().run_id.as_str().is_empty()
            || authorization.state_input_ref() != state_input_ref
        {
            return Box::pin(std::future::ready(EffectAdapterCompletion::IntegrityFault(
                integrity_fault.clone(),
            )));
        }
        authority.activate_candidate_authorized(state_input_ref, request, authorization)
    }
}

impl WalletEffectSpec for CompleteWalletNonceCapability {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        complete_wallet_nonce_adapter_contract()
    }

    fn invoke<'a>(
        _authority: &'a dyn WalletNonceAuthority,
        _state_input_ref: &'a LexicalValueRef,
        _request: &'a CompleteEvmNonceRequest,
    ) -> ComponentFuture<'a, EffectContractCompletion<CompleteWalletNonceCapability>> {
        // Mutation has no unqualified path. Runtime must supply the affine
        // committed authorization to `invoke_authorized`.
        Box::pin(std::future::ready(EffectAdapterCompletion::SafeFailure(
            EvmSubmissionFailure::NonceLineageDiverged,
        )))
    }

    fn invoke_authorized<'a>(
        authority: &'a dyn WalletNonceAuthority,
        state_input_ref: &'a LexicalValueRef,
        request: &'a CompleteEvmNonceRequest,
        authorization: CertifiedAccessAuthorization,
        integrity_fault: &'a AccessFaultCode,
    ) -> ComponentFuture<'a, EffectContractCompletion<CompleteWalletNonceCapability>> {
        if authorization.authorization_ref().run_id.as_str().is_empty()
            || authorization.state_input_ref() != state_input_ref
        {
            return Box::pin(std::future::ready(EffectAdapterCompletion::IntegrityFault(
                integrity_fault.clone(),
            )));
        }
        authority.complete_authorized(state_input_ref, request, authorization)
    }
}

/// Semantic resource lineage for the cross-run wallet nonce authority.
pub enum WalletNonceAuthorityResource {}

impl mfm_capabilities::BoundedComponentContract for WalletNonceAuthorityResource {
    type Request = ();
    type Completion = ();
}

impl ResourceAuthorityContract for WalletNonceAuthorityResource {}

impl RuntimeResourceAuthority for WalletNonceAuthorityResource {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        StructuredLiveComponentContract::new(
            StructuredComponentKind::Resource,
            stable("mfm.evm.resource/wallet-nonce-authority")?,
            Vec::new(),
        )
        .map_err(Into::into)
    }
}

macro_rules! wallet_read_capability {
    ($name:ident, $request:ty, $returned:ty, $id:literal, $adapter:literal) => {
        #[doc = concat!("Typed Read capability for `", $id, "`.")]
        pub enum $name {}

        impl ReadCapabilityContract for $name {
            type Request = $request;
            type Returned = $returned;
            type SafeFailure = EvmSubmissionFailure;
        }

        impl RuntimeReadCapability for $name {
            fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
                StructuredLiveComponentContract::new_read_capability(
                    stable($id)?,
                    structured_value_contract_ref::<$request>()?,
                    structured_value_contract_ref::<$returned>()?,
                    structured_value_contract_ref::<EvmSubmissionFailure>()?,
                    wallet_adapter_contract($adapter)?.content_ref()?,
                )
                .map_err(Into::into)
            }
        }
    };
}

macro_rules! wallet_effect_capability {
    ($name:ident, $request:ty, $returned:ty, $id:literal, $adapter:literal) => {
        #[doc = concat!("Typed refreshable Effect capability for `", $id, "`.")]
        pub enum $name {}

        impl EffectCapabilityContract for $name {
            type Request = $request;
            type Returned = $returned;
            type SafeFailure = EvmSubmissionFailure;
            type Refresh = Refreshable<WalletNonceStoreLineageHead>;
        }

        impl RuntimeEffectCapability for $name {
            type RefreshBinding = RefreshableBinding<WalletNonceAuthorityResource>;

            fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
                StructuredLiveComponentContract::new_effect_capability_refreshable(
                    stable($id)?,
                    structured_value_contract_ref::<$request>()?,
                    structured_value_contract_ref::<$returned>()?,
                    structured_value_contract_ref::<EvmSubmissionFailure>()?,
                    structured_value_contract_ref::<WalletNonceStoreLineageHead>()?,
                    WalletNonceAuthorityResource::contract()?.content_ref()?,
                    wallet_adapter_contract($adapter)?.content_ref()?,
                )
                .map_err(Into::into)
            }
        }
    };
}

wallet_read_capability!(
    ReadWalletNonceStatusCapability,
    ReadEvmWalletNonceStatusRequest,
    WalletNonceStatus,
    "mfm.evm.capability/read-wallet-nonce-status",
    "mfm.evm.adapter/read-wallet-nonce-status"
);
wallet_effect_capability!(
    ReserveWalletNonceCapability,
    ReserveEvmNonceRequest,
    ReserveWalletNonceResponse,
    "mfm.evm.capability/reserve-wallet-nonce",
    "mfm.evm.adapter/reserve-wallet-nonce"
);
wallet_effect_capability!(
    ActivateWalletCandidateCapability,
    ActivateEvmCandidateRequest,
    ActivateCandidateResponse,
    "mfm.evm.capability/activate-wallet-candidate",
    "mfm.evm.adapter/activate-wallet-candidate"
);
wallet_effect_capability!(
    CompleteWalletNonceCapability,
    CompleteEvmNonceRequest,
    CompleteWalletNonceResponse,
    "mfm.evm.capability/complete-wallet-nonce",
    "mfm.evm.adapter/complete-wallet-nonce"
);

/// Returns the semantic wallet-status adapter contract.
pub fn read_wallet_nonce_status_adapter_contract(
) -> mfm_program::Result<StructuredLiveComponentContract> {
    wallet_adapter_contract("mfm.evm.adapter/read-wallet-nonce-status")
}

/// Returns the semantic nonce-reservation adapter contract.
pub fn reserve_wallet_nonce_adapter_contract(
) -> mfm_program::Result<StructuredLiveComponentContract> {
    wallet_adapter_contract("mfm.evm.adapter/reserve-wallet-nonce")
}

/// Returns the semantic candidate-activation adapter contract.
pub fn activate_wallet_candidate_adapter_contract(
) -> mfm_program::Result<StructuredLiveComponentContract> {
    wallet_adapter_contract("mfm.evm.adapter/activate-wallet-candidate")
}

/// Returns the semantic nonce-completion adapter contract.
pub fn complete_wallet_nonce_adapter_contract(
) -> mfm_program::Result<StructuredLiveComponentContract> {
    wallet_adapter_contract("mfm.evm.adapter/complete-wallet-nonce")
}

fn wallet_adapter_contract(id: &str) -> mfm_program::Result<StructuredLiveComponentContract> {
    StructuredLiveComponentContract::new(
        StructuredComponentKind::Adapter,
        stable(id)?,
        vec![StructuredComponentDependency {
            component_kind: StructuredComponentKind::Resource,
            contract_ref: WalletNonceAuthorityResource::contract()?.content_ref()?,
        }],
    )
    .map_err(Into::into)
}

/// Returns the exact content reference for any canonical public wallet value.
pub fn canonical_wallet_reference<T>(
    value: &T,
) -> Result<EvmWalletReference, WalletAuthorityContractError>
where
    T: mfm_values::MfmValue,
{
    let json = serde_json::to_string(value).map_err(|_| WalletAuthorityContractError::Canonical)?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|_| WalletAuthorityContractError::Canonical)?;
    let schema = T::schema_id().map_err(|_| WalletAuthorityContractError::Canonical)?;
    let reference = ContentRef::new(
        schema,
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            sha256_digest_bytes(canonical.as_bytes()),
        ),
    )
    .map_err(|_| WalletAuthorityContractError::Canonical)?;
    Ok(EvmWalletReference::from_content_ref(reference))
}

fn hash<T: Serialize>(
    domain: &str,
    value: &T,
) -> Result<ContentDigest, WalletAuthorityContractError> {
    domain_content_digest(domain, value).map_err(|_| WalletAuthorityContractError::Canonical)
}

fn validate_reference(value: &EvmWalletReference) -> Result<(), WalletAuthorityContractError> {
    value
        .to_content_ref()
        .map(|_| ())
        .map_err(|_| WalletAuthorityContractError::Invalid("content_reference"))
}

fn validate_address(value: &str) -> Result<(), WalletAuthorityContractError> {
    let address =
        Address::from_str(value).map_err(|_| WalletAuthorityContractError::Invalid("address"))?;
    if address.is_zero() || value != format!("{address:#x}") {
        return Err(WalletAuthorityContractError::Invalid("address"));
    }
    Ok(())
}

fn validate_hash(value: &str, allow_zero: bool) -> Result<(), WalletAuthorityContractError> {
    let hash =
        B256::from_str(value).map_err(|_| WalletAuthorityContractError::Invalid("block_hash"))?;
    if (!allow_zero && hash == B256::ZERO) || value != format!("{hash:#x}") {
        return Err(WalletAuthorityContractError::Invalid("block_hash"));
    }
    Ok(())
}

fn stable(value: &str) -> mfm_program::Result<StableId> {
    StableId::new(value).map_err(|error| mfm_program::ProgramError::Authoring(error.to_string()))
}
