//! Sealed deployment qualification and live transport ownership for one exact EVM wallet family.

use std::fmt;
use std::str::FromStr;

use alloy_primitives::Address;
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes, RecoverabilityContract};
use mfm_evm::{
    evm_wallet_assurance_policy_ref, evm_wallet_finality_policy_ref, evm_wallet_nonce_policy_ref,
    EvmRoutingGenerationRef, EvmSubmitTransactionRequest, EvmWalletAttemptResult,
    EvmWalletInitialNonceDescriptor, EvmWalletReference, EVM_SUBMIT_TRANSACTION_OPERATION_ID,
    EVM_WALLET_REVERTED_TERMINAL_OUTCOME, EVM_WALLET_SUCCEEDED_TERMINAL_OUTCOME,
};
use mfm_executor::{
    observation_completion_closure_bytes, tombstone_completion_closure_bytes,
    DeliveryAttemptOutcome, DeliveryAuditFrontierRef, EffectIdentity, ReferenceTerminalProof,
    ResourcePolicyBinding, ReturnedOutcome, SchemaQualifiedCanonicalValue, TerminalTombstone,
    VerifiedExecutorBinding,
};
use mfm_ids::{
    AttemptId, ContentDigest, ContentRef, DigestAlgorithm, EffectKey, RequestDigest, SchemaId,
};
use mfm_program::boundary_content_ref;
use mfm_signing::{
    GenerationGuardedSignerDescriptor, VerifiedGenerationGuardedSignerBinding,
    SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID, SECP256K1_RFC6979_LOW_S_PROFILE_ID,
};
use serde_json::json;

use crate::transport::EvmJsonRpcTransport;
use crate::{
    evm_already_known_classifier_ref, wallet_rpc::PreparedWalletOutcomes, EvmWalletLiveError,
};
use mfm_values::MfmValue;

/// Sealed qualification that exactly matches one wallet deployment.
///
/// Construction resolves only public immutable descriptors and performs no
/// route, signer, executor-ledger, or target IO. The canonical proof, content
/// identity, and debug representation contain no endpoint or authorization.
/// The live object separately retains a private clone of the exact transport,
/// sharing its runtime and route catalog, for use by the executor.
///
/// ```
/// use mfm_evm::EvmSubmitTransactionRequest;
/// use mfm_evm_live::{EvmWalletLiveError, EvmWalletRequestQualification};
///
/// fn qualify_for_admission(
///     qualification: &EvmWalletRequestQualification,
///     request: &EvmSubmitTransactionRequest,
/// ) -> Result<(), EvmWalletLiveError> {
///     qualification.verify_request(request)
/// }
/// ```
#[derive(Clone)]
pub struct EvmWalletRequestQualification {
    transport: EvmJsonRpcTransport,
    executor_binding: VerifiedExecutorBinding,
    resource_policy_binding: ResourcePolicyBinding,
    object_evidence_contract_ref: ContentRef,
    routing_catalog_ref: ContentRef,
    route_chain_map: Vec<(EvmRoutingGenerationRef, u64)>,
    route_generation_ref: EvmRoutingGenerationRef,
    chain_id: u64,
    signer_descriptor: GenerationGuardedSignerDescriptor,
    sender: Address,
    wallet_domain_ref: ContentRef,
    initial_nonce_descriptor: EvmWalletInitialNonceDescriptor,
    already_known_classifier_ref: EvmWalletReference,
    finality_policy_ref: EvmWalletReference,
    assurance_policy_ref: EvmWalletReference,
    generation_fence_ref: ContentRef,
    canonical: PlainCanonicalJsonBytes,
    reference: ContentRef,
}

impl EvmWalletRequestQualification {
    /// Qualifies one exact public wallet deployment and retains its transport.
    ///
    /// This does not open the signer or perform provider IO. The retained
    /// transport is excluded from the canonical proof and public diagnostics.
    #[allow(clippy::too_many_arguments)]
    pub fn qualify(
        transport: &EvmJsonRpcTransport,
        route_generation_ref: EvmRoutingGenerationRef,
        executor_binding: VerifiedExecutorBinding,
        signer_binding: &VerifiedGenerationGuardedSignerBinding,
        initial_nonce_descriptor: EvmWalletInitialNonceDescriptor,
        object_evidence_contract_ref: ContentRef,
    ) -> Result<Self, EvmWalletLiveError> {
        validate_executor_semantics(&executor_binding, &object_evidence_contract_ref)?;
        let routing_catalog = transport.routing_catalog_descriptor();
        let routing_catalog_ref = routing_catalog
            .content_ref()
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let route_chain_map = transport
            .routing_generation_descriptors()
            .map(|(reference, descriptor)| {
                let content_ref = descriptor
                    .content_ref()
                    .map_err(|_| EvmWalletLiveError::InvalidContract)?;
                if reference
                    .to_content_ref()
                    .map_err(|_| EvmWalletLiveError::InvalidContract)?
                    != content_ref
                    || descriptor.chain_id() == 0
                {
                    return Err(EvmWalletLiveError::InvalidContract);
                }
                Ok((reference.clone(), descriptor.chain_id()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if route_chain_map
            .iter()
            .map(|(reference, _)| reference)
            .ne(routing_catalog.ordered_generation_refs().iter())
        {
            return Err(EvmWalletLiveError::InvalidContract);
        }
        let chain_id = route_chain_map
            .iter()
            .find_map(|(reference, chain_id)| {
                (reference == &route_generation_ref).then_some(*chain_id)
            })
            .ok_or(EvmWalletLiveError::InvalidContract)?;
        let selected_route_ref = route_generation_ref
            .to_content_ref()
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;

        let ownership = executor_binding
            .resource_ownership()
            .ok_or(EvmWalletLiveError::InvalidContract)?;
        let wallet_domain_ref = ownership.external_resource_domain_ref().clone();
        let generation_fence_ref = ownership
            .destination_fencing_authority_ref()
            .cloned()
            .ok_or(EvmWalletLiveError::InvalidContract)?;
        if executor_binding.contract().resource_domain_requirement() != Some(&wallet_domain_ref) {
            return Err(EvmWalletLiveError::InvalidContract);
        }

        let signer_descriptor = signer_binding
            .public_descriptor()
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let sender_raw = signer_descriptor
            .expected_public_identity()
            .account_id()
            .ok_or(EvmWalletLiveError::InvalidContract)?;
        let sender =
            Address::from_str(sender_raw).map_err(|_| EvmWalletLiveError::InvalidContract)?;
        if sender.is_zero()
            || sender_raw != format!("{sender:#x}")
            || signer_descriptor.algorithm().as_str()
                != SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID
            || signer_descriptor.profile().as_str() != SECP256K1_RFC6979_LOW_S_PROFILE_ID
            || signer_descriptor.durable_generation_ref()
                != executor_binding
                    .deployment()
                    .durable_ledger_generation_ref()
            || signer_descriptor.durable_generation_ref()
                != ownership.durable_ledger_generation_ref()
            || signer_descriptor.fence_attestation_ref() != &generation_fence_ref
        {
            return Err(EvmWalletLiveError::InvalidContract);
        }

        let initial_nonce_ref = initial_nonce_descriptor
            .reference()
            .and_then(|reference| reference.to_content_ref())
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        if initial_nonce_descriptor
            .wallet_domain_ref()
            .to_content_ref()
            .ok()
            .as_ref()
            != Some(&wallet_domain_ref)
            || initial_nonce_descriptor.chain_id() != chain_id
            || initial_nonce_descriptor.sender() != sender_raw
            || initial_nonce_descriptor
                .durable_generation_ref()
                .to_content_ref()
                .ok()
                .as_ref()
                != Some(
                    executor_binding
                        .deployment()
                        .durable_ledger_generation_ref(),
                )
        {
            return Err(EvmWalletLiveError::InvalidContract);
        }

        let nonce_policy_ref = evm_wallet_nonce_policy_ref()
            .and_then(|reference| reference.to_content_ref())
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let resource_policy_binding =
            ResourcePolicyBinding::new(nonce_policy_ref.clone(), initial_nonce_ref.clone());
        let already_known_classifier_ref =
            evm_already_known_classifier_ref().map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let finality_policy_ref =
            evm_wallet_finality_policy_ref().map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let assurance_policy_ref =
            evm_wallet_assurance_policy_ref().map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let signer_descriptor_ref = signer_descriptor.reference().clone();
        let route_chain_map_canonical = route_chain_map
            .iter()
            .map(|(reference, chain_id)| {
                json!({
                    "chain_id": chain_id,
                    "route_generation_ref": reference,
                })
            })
            .collect::<Vec<_>>();
        let bounds = executor_binding.contract().evidence_bounds();
        let canonical = canonical_json(&json!({
            "already_known_classifier_ref": &already_known_classifier_ref,
            "assurance_policy_ref": &assurance_policy_ref,
            "chain_id": chain_id,
            "evidence_bounds": {
                "completion_reserve_bytes": bounds.completion_reserve_bytes().to_string(),
                "completion_reserve_records": bounds.completion_reserve_records(),
                "max_attempts": bounds.max_attempts(),
                "max_completion_record_bytes": bounds.max_completion_record_bytes().to_string(),
                "max_records": bounds.max_records(),
                "max_retained_bytes": bounds.max_retained_bytes().to_string(),
            },
            "executor_binding_ref": executor_binding.binding_ref().as_content_ref(),
            "executor_generation_ref": executor_binding.deployment().durable_ledger_generation_ref(),
            "finality_policy_ref": &finality_policy_ref,
            "generation_fence_ref": &generation_fence_ref,
            "initial_nonce": initial_nonce_descriptor.initial_nonce()
                .map_err(|_| EvmWalletLiveError::InvalidContract)?
                .to_string(),
            "nonce_configuration_ref": &initial_nonce_ref,
            "nonce_policy_ref": &nonce_policy_ref,
            "object_evidence_contract_ref": &object_evidence_contract_ref,
            "resource_ownership_ref": ownership.reference()
                .map_err(|_| EvmWalletLiveError::InvalidContract)?
                .as_content_ref(),
            "route_chain_map": route_chain_map_canonical,
            "route_generation_ref": &selected_route_ref,
            "routing_catalog_ref": &routing_catalog_ref,
            "sender": sender_raw,
            "signer_descriptor_ref": &signer_descriptor_ref,
            "tenant_scope_id": executor_binding.deployment().tenant_scope_id(),
            "version": "mfm.evm-live.wallet-request-qualification.v1",
            "wallet_domain_ref": wallet_domain_ref,
        }))?;
        let reference = boundary_content_ref(
            descriptor_schema_id("mfm.evm-live.wallet-request-qualification")?,
            &canonical,
        )
        .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        Ok(Self {
            transport: transport.clone(),
            executor_binding,
            resource_policy_binding,
            object_evidence_contract_ref,
            routing_catalog_ref,
            route_chain_map,
            route_generation_ref,
            chain_id,
            signer_descriptor,
            sender,
            wallet_domain_ref,
            initial_nonce_descriptor,
            already_known_classifier_ref,
            finality_policy_ref,
            assurance_policy_ref,
            generation_fence_ref,
            canonical,
            reference,
        })
    }

    /// Verifies the complete request/deployment equality predicate.
    pub fn verify_request(
        &self,
        request: &EvmSubmitTransactionRequest,
    ) -> Result<(), EvmWalletLiveError> {
        let policy = request.policy();
        let route_ref = policy
            .route_generation_ref()
            .to_content_ref()
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let signer_ref = policy
            .signer_binding_ref()
            .to_content_ref()
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let wallet_domain_ref = policy
            .wallet_domain_ref()
            .to_content_ref()
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let nonce_descriptor_ref = policy
            .initial_nonce_descriptor_ref()
            .to_content_ref()
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let nonce_policy_ref = policy
            .nonce_policy_ref()
            .to_content_ref()
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        if policy
            .tenant_scope_id()
            .map_err(|_| EvmWalletLiveError::InvalidContract)?
            != *self.executor_binding.deployment().tenant_scope_id()
            || route_ref
                != self
                    .route_generation_ref
                    .to_content_ref()
                    .map_err(|_| EvmWalletLiveError::InvalidContract)?
            || policy.chain_id() != self.chain_id
            || signer_ref != *self.signer_descriptor.reference()
            || policy
                .sender_address()
                .map_err(|_| EvmWalletLiveError::InvalidContract)?
                != self.sender
            || wallet_domain_ref != self.wallet_domain_ref
            || policy
                .initial_nonce()
                .map_err(|_| EvmWalletLiveError::InvalidContract)?
                != self
                    .initial_nonce_descriptor
                    .initial_nonce()
                    .map_err(|_| EvmWalletLiveError::InvalidContract)?
            || nonce_descriptor_ref != *self.resource_policy_binding.policy_configuration_ref()
            || nonce_policy_ref != *self.resource_policy_binding.policy_ref()
            || policy.already_known_classifier_ref() != &self.already_known_classifier_ref
            || policy.finality_policy_ref() != &self.finality_policy_ref
            || policy.assurance_policy_ref() != &self.assurance_policy_ref
            || policy.evidence_bounds() != self.executor_binding.contract().evidence_bounds()
        {
            return Err(EvmWalletLiveError::InvalidContract);
        }
        validate_completion_capacity(&self.executor_binding, request)?;
        Ok(())
    }

    pub(crate) const fn transport(&self) -> &EvmJsonRpcTransport {
        &self.transport
    }

    /// Returns the exact verified executor binding.
    pub const fn executor_binding(&self) -> &VerifiedExecutorBinding {
        &self.executor_binding
    }

    /// Returns the exact derived nonce policy/configuration pair.
    pub const fn resource_policy_binding(&self) -> &ResourcePolicyBinding {
        &self.resource_policy_binding
    }

    /// Returns the product object-evidence contract shared by the full executor closure.
    pub const fn object_evidence_contract_ref(&self) -> &ContentRef {
        &self.object_evidence_contract_ref
    }

    /// Returns the exact complete routing catalog selected by qualification.
    pub const fn routing_catalog_ref(&self) -> &ContentRef {
        &self.routing_catalog_ref
    }

    /// Returns the complete ordered route-generation to chain-id map.
    pub fn route_chain_map(&self) -> &[(EvmRoutingGenerationRef, u64)] {
        &self.route_chain_map
    }

    /// Returns the selected routing generation.
    pub const fn route_generation_ref(&self) -> &EvmRoutingGenerationRef {
        &self.route_generation_ref
    }

    /// Returns the selected non-zero chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the complete public guarded-signer descriptor.
    pub const fn signer_descriptor(&self) -> &GenerationGuardedSignerDescriptor {
        &self.signer_descriptor
    }

    /// Returns the canonical sender.
    pub const fn sender(&self) -> Address {
        self.sender
    }

    /// Returns the exact external wallet/resource domain.
    pub const fn wallet_domain_ref(&self) -> &ContentRef {
        &self.wallet_domain_ref
    }

    /// Returns the complete initial nonce descriptor.
    pub const fn initial_nonce_descriptor(&self) -> &EvmWalletInitialNonceDescriptor {
        &self.initial_nonce_descriptor
    }

    /// Returns the exact already-known classifier.
    pub const fn already_known_classifier_ref(&self) -> &EvmWalletReference {
        &self.already_known_classifier_ref
    }

    /// Returns the exact finalized-tag finality policy.
    pub const fn finality_policy_ref(&self) -> &EvmWalletReference {
        &self.finality_policy_ref
    }

    /// Returns the exact terminal assurance policy.
    pub const fn assurance_policy_ref(&self) -> &EvmWalletReference {
        &self.assurance_policy_ref
    }

    /// Returns the exact destination generation fence.
    pub const fn generation_fence_ref(&self) -> &ContentRef {
        &self.generation_fence_ref
    }

    /// Returns exact canonical qualification-closure bytes.
    pub const fn canonical(&self) -> &PlainCanonicalJsonBytes {
        &self.canonical
    }

    /// Returns the exact qualification-closure content identity.
    pub const fn reference(&self) -> &ContentRef {
        &self.reference
    }
}

fn validate_completion_capacity(
    binding: &VerifiedExecutorBinding,
    request: &EvmSubmitTransactionRequest,
) -> Result<(), EvmWalletLiveError> {
    let (max_observation, max_tombstone) = calculated_completion_maxima(binding, request)?;
    let admitted = binding
        .contract()
        .evidence_bounds()
        .max_completion_record_bytes();
    if max_observation > admitted || max_tombstone > admitted {
        return Err(EvmWalletLiveError::InvalidContract);
    }
    Ok(())
}

fn calculated_completion_maxima(
    binding: &VerifiedExecutorBinding,
    request: &EvmSubmitTransactionRequest,
) -> Result<(usize, usize), EvmWalletLiveError> {
    let identity = EffectIdentity::reconstruct(
        binding,
        binding.deployment().tenant_scope_id().clone(),
        binding.binding_ref().clone(),
        EffectKey::from_digest(sha256_digest_bytes(b"evm-completion-effect")),
        RequestDigest::from_digest(sha256_digest_bytes(b"evm-completion-request")),
    )
    .map_err(|_| EvmWalletLiveError::InvalidContract)?;
    let predecessor = DeliveryAuditFrontierRef::from_content_ref(dummy_content_ref(
        "mfm.executor-delivery-frontier.v2",
        b"evm-completion-predecessor",
    )?)
    .map_err(|_| EvmWalletLiveError::InvalidContract)?;
    let attempt_id = AttemptId::from_digest(sha256_digest_bytes(b"evm-completion-attempt"));
    let proof_ref = binding.deployment().evidence_authority_ref().clone();
    let safe_result = maximum_attempt_result(request)?;
    let returned = DeliveryAttemptOutcome::returned(safe_result.clone())
        .map_err(|_| EvmWalletLiveError::InvalidContract)?;
    let fixed =
        PreparedWalletOutcomes::new(binding.contract().safe_failure_contract_ref().clone())?;
    let mut max_observation = observation_completion_closure_bytes(
        &identity,
        predecessor.clone(),
        attempt_id.clone(),
        returned,
        proof_ref.clone(),
    )
    .map_err(|_| EvmWalletLiveError::InvalidContract)?;
    for outcome in fixed.completion_outcomes() {
        max_observation = max_observation.max(
            observation_completion_closure_bytes(
                &identity,
                predecessor.clone(),
                attempt_id.clone(),
                outcome.clone(),
                proof_ref.clone(),
            )
            .map_err(|_| EvmWalletLiveError::InvalidContract)?,
        );
    }

    let returned =
        ReturnedOutcome::new(safe_result).map_err(|_| EvmWalletLiveError::InvalidContract)?;
    let observation_ref = dummy_content_ref(
        "mfm.executor-delivery-attempt-observed.v2",
        b"evm-completion-observation",
    )?;
    let terminal_proof = ReferenceTerminalProof::new(attempt_id, returned, observation_ref)
        .map_err(|_| EvmWalletLiveError::InvalidContract)?;
    let mut max_tombstone = 0_usize;
    for outcome in [
        EVM_WALLET_SUCCEEDED_TERMINAL_OUTCOME,
        EVM_WALLET_REVERTED_TERMINAL_OUTCOME,
    ] {
        let tombstone = TerminalTombstone::new(
            EVM_SUBMIT_TRANSACTION_OPERATION_ID,
            outcome,
            terminal_proof.clone(),
        )
        .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        max_tombstone = max_tombstone.max(
            tombstone_completion_closure_bytes(
                &identity,
                predecessor.clone(),
                tombstone,
                proof_ref.clone(),
            )
            .map_err(|_| EvmWalletLiveError::InvalidContract)?,
        );
    }
    Ok((max_observation, max_tombstone))
}

#[cfg(test)]
pub(crate) fn calculated_completion_maxima_for_test(
    binding: &VerifiedExecutorBinding,
    request: &EvmSubmitTransactionRequest,
) -> Result<(usize, usize), EvmWalletLiveError> {
    calculated_completion_maxima(binding, request)
}

fn maximum_attempt_result(
    request: &EvmSubmitTransactionRequest,
) -> Result<SchemaQualifiedCanonicalValue, EvmWalletLiveError> {
    let bytes = usize::try_from(request.policy().convergence().max_attempt_result_bytes())
        .map_err(|_| EvmWalletLiveError::InvalidContract)?;
    let canonical = match bytes {
        0 => return Err(EvmWalletLiveError::InvalidContract),
        1 => vec![b'0'],
        _ => {
            let mut canonical = Vec::with_capacity(bytes);
            canonical.push(b'"');
            canonical.resize(bytes - 1, b'x');
            canonical.push(b'"');
            canonical
        }
    };
    SchemaQualifiedCanonicalValue::new(
        EvmWalletAttemptResult::schema_id().map_err(|_| EvmWalletLiveError::InvalidContract)?,
        &canonical,
    )
    .map_err(|_| EvmWalletLiveError::InvalidContract)
}

fn dummy_content_ref(schema_contract: &str, seed: &[u8]) -> Result<ContentRef, EvmWalletLiveError> {
    let schema_id = RecoverabilityContract::embedded()
        .map_err(|_| EvmWalletLiveError::InvalidContract)?
        .schema_id(schema_contract)
        .map_err(|_| EvmWalletLiveError::InvalidContract)?
        .clone();
    ContentRef::new(
        schema_id,
        ContentDigest::from_digest(DigestAlgorithm::Sha256V1, sha256_digest_bytes(seed)),
    )
    .map_err(|_| EvmWalletLiveError::InvalidContract)
}

impl PartialEq for EvmWalletRequestQualification {
    fn eq(&self, other: &Self) -> bool {
        self.canonical == other.canonical
            && self.reference == other.reference
            && self.transport.is_same_instance(&other.transport)
    }
}

impl Eq for EvmWalletRequestQualification {}

impl fmt::Debug for EvmWalletRequestQualification {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EvmWalletRequestQualification")
            .field("reference", &self.reference)
            .field("transport", &"<private-live-transport>")
            .finish()
    }
}

fn descriptor_schema_id(name: &'static str) -> Result<SchemaId, EvmWalletLiveError> {
    SchemaId::new(
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("schema:{name}:1").as_bytes()),
    )
    .map_err(|_| EvmWalletLiveError::InvalidContract)
}

fn canonical_json(
    value: &serde_json::Value,
) -> Result<PlainCanonicalJsonBytes, EvmWalletLiveError> {
    PlainCanonicalJsonBytes::from_json_str(&value.to_string())
        .map_err(|_| EvmWalletLiveError::InvalidContract)
}

fn validate_executor_semantics(
    binding: &VerifiedExecutorBinding,
    object_evidence_contract_ref: &ContentRef,
) -> Result<(), EvmWalletLiveError> {
    let contract = binding.contract();
    let ownership = binding
        .resource_ownership()
        .ok_or(EvmWalletLiveError::InvalidContract)?;
    let value_contracts =
        mfm_evm::evm_submit_transaction_value_contracts(object_evidence_contract_ref.clone())
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
    let target_callback_ref = crate::evm_wallet_target_callback_surface_ref()
        .map_err(|_| EvmWalletLiveError::InvalidContract)?;
    let expected_expansion =
        mfm_evm::evm_submit_transaction_leaf_expansion(target_callback_ref.clone())
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
    let safe_failure_contract_ref = mfm_evm::evm_safe_failure_contract_ref()
        .map_err(|_| EvmWalletLiveError::InvalidContract)?;
    let retained = contract.retained_closure_contract();
    let evidence_contracts = [
        contract.semantic_request_contract(),
        contract.safe_failure_value_contract(),
        retained.ensure_result_contract(),
        retained.delivery_audit_contract(),
        retained.executor_frontier_contract(),
        retained.terminal_evidence_contract(),
        retained.terminal_tombstone_contract(),
        retained.terminal_proof_contract(),
        retained.domain_evidence_contract(),
    ];
    if contract.semantic_request_contract() != value_contracts.request()
        || retained.domain_evidence_contract() != value_contracts.attempt_result()
        || contract.safe_failure_contract_ref() != &safe_failure_contract_ref
        || contract.downstream_convergence_contract_ref() != &target_callback_ref
        || contract.required_plan_expansions() != [expected_expansion]
        || contract.resource_domain_requirement() != Some(ownership.external_resource_domain_ref())
        || ownership.destination_fencing_authority_ref().is_none()
        || evidence_contracts.into_iter().any(|value_contract| {
            value_contract.evidence_contract_ref() != object_evidence_contract_ref
        })
    {
        return Err(EvmWalletLiveError::InvalidContract);
    }
    Ok(())
}
