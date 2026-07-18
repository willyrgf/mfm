//! Runtime binding and evidence-only replay for one EVM transaction.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use alloy_primitives::U256;
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_capabilities::CapabilitySpec;
use mfm_events::v1::{self as events, side_effect};
use mfm_evm_capabilities::{
    EvmBlockSelector, EvmCapabilityError, EvmCapabilityFailureDisposition, EvmCapabilityPhase,
    EvmNetworkBinding, EvmTransactionCapability, EvmTransactionSession,
    EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
};
use mfm_evm_signing::TransientSignedEip1559Envelope;
use mfm_program::{SideEffectState, StateSpec, ValidatedConfig};
use mfm_replay::v1 as replay;
use mfm_runtime::{
    load_launch_config_for_node, load_materialized_struct_input, load_runner_config_for_node,
    load_side_effect_artifact, preclaim_side_effect_resource_lane, side_effect_idempotency_key,
    CapabilityImplementationId, ErasedNodeRunner, ErasedRunCtx, ErasedRunnerFuture,
    ErasedRunnerRegistry, MaterializedInputs, PreInvocationRunCtx, PreInvocationRunnerFuture,
    RunnerCapabilityBinding, RunnerExecutableIdentityTemplate, RunnerIngressContext,
    RunnerIngressFuture, RunnerOutputSettlement, RunnerRegistrationBuilder, SideEffectAdapter,
    SideEffectDriver, SideEffectDriverFuture, SideEffectObservedEvidence,
    SideEffectPreparedInvocation, SideEffectReplayEvidence, SideEffectSubmissionDecision,
    SideEffectUnknownSubmissionDecision, SideEffectVerifyDriver,
};
use mfm_spec::v1 as spec;
use mfm_states_evm::{
    evm_jsonrpc_adapter_kind, evm_jsonrpc_adapter_version, EvmPreparedTransaction, EvmSenderLane,
    EvmTransactionAction, EvmTransactionConfig, EvmTransactionConfirmation, EvmTransactionIntent,
    EvmTransactionOutcome, EvmTransactionReceipt, EvmTransactionRecoveryEvidence,
    EvmTransactionSubmission, EvmUnsignedTransaction, SubmitEvmTransactionState,
    EVM_SENDER_LANE_NAMESPACE,
};
use mfm_store::v1 as store;
use mfm_values::MfmValue;
use serde::de::DeserializeOwned;
use serde::Serialize;

use super::{adapter_identity_error, evm_capability_runtime_error, ADAPTER_FACTORY};

const SIDE_EFFECT_FACTORY: &str = "apply_side_effect";
const VERIFY_FACTORY: &str = "read_external";
const REPLAY_VERIFIER_ID: &str = "mfm.evm.transaction.replay.v1";
const SIGNED_ENVELOPE_CACHE_CAPACITY: usize = 32;

/// Future returned by the application-owned transaction-session binder.
pub type EvmTransactionSessionBindFuture = Pin<
    Box<
        dyn Future<Output = mfm_evm_capabilities::Result<Arc<dyn EvmTransactionSession>>>
            + Send
            + 'static,
    >,
>;

/// Future returned by application-owned mutation ingress validation.
pub type EvmMutationValidationFuture =
    Pin<Box<dyn Future<Output = mfm_runtime::Result<()>> + Send + 'static>>;

type ValidateMutation =
    dyn Fn(EvmNetworkBinding, mfm_signing::SignerRef) -> EvmMutationValidationFuture + Send + Sync;
type BindTransactionSession =
    dyn Fn(EvmNetworkBinding) -> EvmTransactionSessionBindFuture + Send + Sync;
/// Process assembly required by the generic EVM transaction runner.
#[derive(Clone)]
pub struct EvmTransactionRunnerCapabilities {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    signing_provider_binder: mfm_signing::DeterministicSigningProviderBinder,
    validate_mutation: Arc<ValidateMutation>,
    bind_transaction_session: Arc<BindTransactionSession>,
}

impl EvmTransactionRunnerCapabilities {
    /// Creates lazy, exact-bound transaction and signer capabilities.
    pub fn new<V, T>(
        artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
        signing_provider_binder: mfm_signing::DeterministicSigningProviderBinder,
        validate_mutation: V,
        bind_transaction_session: T,
    ) -> Self
    where
        V: Fn(EvmNetworkBinding, mfm_signing::SignerRef) -> EvmMutationValidationFuture
            + Send
            + Sync
            + 'static,
        T: Fn(EvmNetworkBinding) -> EvmTransactionSessionBindFuture + Send + Sync + 'static,
    {
        Self {
            artifacts,
            signing_provider_binder,
            validate_mutation: Arc::new(validate_mutation),
            bind_transaction_session: Arc::new(bind_transaction_session),
        }
    }

    async fn validate(
        &self,
        binding: EvmNetworkBinding,
        signer_ref: mfm_signing::SignerRef,
    ) -> mfm_runtime::Result<()> {
        (self.validate_mutation)(binding, signer_ref).await
    }

    async fn bind_session(
        &self,
        binding: EvmNetworkBinding,
        phase: EvmCapabilityPhase,
    ) -> mfm_runtime::Result<Arc<dyn EvmTransactionSession>> {
        let session = (self.bind_transaction_session)(binding.clone())
            .await
            .map_err(|error| evm_capability_runtime_error(error, phase))?;
        let evidence = session.evidence();
        if !evidence.matches_binding(&binding)
            || evidence.implementation_id() != EVM_JSONRPC_SESSION_IMPLEMENTATION_ID
        {
            return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
                "bound EVM transaction session violated semantic authority".to_owned(),
            ));
        }
        Ok(session)
    }

    async fn bind_signer(
        &self,
        signer_ref: mfm_signing::SignerRef,
    ) -> mfm_runtime::Result<Arc<dyn mfm_signing::DeterministicSigningProvider>> {
        let provider = self
            .signing_provider_binder
            .bind(signer_ref)
            .await
            .map_err(signing_error)?;
        if provider.implementation_id() != self.signing_provider_binder.implementation_id().as_str()
        {
            return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
                "bound deterministic signer violated implementation authority".to_owned(),
            ));
        }
        Ok(provider)
    }
}

/// Registers the one reusable EVM transaction submit/verify runner pair.
pub fn register_evm_transaction_runner(
    registry: &mut ErasedRunnerRegistry,
    capabilities: EvmTransactionRunnerCapabilities,
) -> mfm_runtime::Result<()> {
    let descriptor = mfm_program::state_descriptor::<SubmitEvmTransactionState>()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    registry.register_capability_spec::<EvmTransactionCapability>(
        CapabilityImplementationId::new(EVM_JSONRPC_SESSION_IMPLEMENTATION_ID)?,
    )?;
    registry.register_capability_spec::<mfm_signing::SigningCapability>(
        CapabilityImplementationId::new(
            capabilities
                .signing_provider_binder
                .implementation_id()
                .as_str(),
        )?,
    )?;
    let executable_identities = RunnerExecutableIdentityTemplate::new(
        "mfm-adapters-evm",
        "typed-evm-jsonrpc",
        env!("CARGO_PKG_VERSION"),
    )?;
    let side_effect_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(SIDE_EFFECT_FACTORY)?);
    let verify_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(VERIFY_FACTORY)?);
    let adapter_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(ADAPTER_FACTORY)?);
    let adapter = Arc::new(EvmTransactionAdapter::new(capabilities));
    let mut registrations = RunnerRegistrationBuilder::new(registry);
    registrations.register_adapter_executable_with_factory(
        evm_jsonrpc_adapter_kind().map_err(adapter_identity_error)?,
        evm_jsonrpc_adapter_version().map_err(adapter_identity_error)?,
        &adapter_factory,
    )?;
    registrations.register_side_effect_state_runner_with_factory::<
        SubmitEvmTransactionState,
        EvmTransactionAdapter,
    >(
        &side_effect_factory,
        adapter.as_ref(),
        Arc::new(EvmTransactionSubmitRunner {
            adapter: Arc::clone(&adapter),
        }),
    )?;
    registrations.register_side_effect_verify_runner_with_factory(
        descriptor.descriptor_id().clone(),
        &verify_factory,
        Arc::new(EvmTransactionVerifyRunner { adapter }),
    )?;
    Ok(())
}

struct EvmTransactionSubmitRunner {
    adapter: Arc<EvmTransactionAdapter>,
}

impl ErasedNodeRunner for EvmTransactionSubmitRunner {
    fn validate_ingress<'a>(&'a self, ctx: RunnerIngressContext<'a>) -> RunnerIngressFuture<'a> {
        Box::pin(async move {
            let config = load_launch_config_for_node::<EvmTransactionConfig>(&ctx, ctx.node())?;
            let binding = config.as_ref().network_binding().map_err(state_error)?;
            let signer_ref = config.as_ref().signer_reference().map_err(state_error)?;
            self.adapter
                .capabilities
                .validate(binding, signer_ref)
                .await
        })
    }

    fn preclaim_resource_lane<'a>(
        &'a self,
        ctx: &'a PreInvocationRunCtx<'a>,
    ) -> PreInvocationRunnerFuture<'a> {
        Box::pin(async move {
            let (state, input, context) =
                preclaim_state_material(ctx, self.adapter.capabilities.artifacts.as_ref()).await?;
            let authored = state.intent(&input, &context).map_err(state_error)?;
            let lane = authored.intent().sender_lane();
            let resource_key = sender_lane_resource_key(ctx.node(), &lane)?;
            preclaim_side_effect_resource_lane(
                ctx,
                authored.intent(),
                authored.idempotency(),
                transaction_capability_binding()?,
                resource_key,
            )
        })
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { SideEffectDriver::drive(ctx, self.adapter.as_ref()).await })
    }
}

struct EvmTransactionVerifyRunner {
    adapter: Arc<EvmTransactionAdapter>,
}

impl ErasedNodeRunner for EvmTransactionVerifyRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { SideEffectVerifyDriver::drive(ctx, self.adapter.as_ref()).await })
    }
}

struct EvmTransactionAdapter {
    capabilities: EvmTransactionRunnerCapabilities,
    signed_envelopes: Arc<SignedEnvelopeCache>,
}

struct PreparedTransactionAuthority {
    prepared: EvmPreparedTransaction,
    reservation: SignedEnvelopeReservation,
    signed: TransientSignedEip1559Envelope,
}

impl EvmTransactionAdapter {
    fn new(capabilities: EvmTransactionRunnerCapabilities) -> Self {
        Self {
            capabilities,
            signed_envelopes: Arc::new(SignedEnvelopeCache::default()),
        }
    }

    async fn sign_prepared_transaction(
        &self,
        prepared: &EvmPreparedTransaction,
    ) -> mfm_runtime::Result<SignedEnvelopeLease> {
        prepared.validate().map_err(state_error)?;
        let reservation = self.signed_envelopes.reserve()?;
        let signer_ref = prepared.intent().signer_reference().map_err(state_error)?;
        let signer = self.capabilities.bind_signer(signer_ref.clone()).await?;
        let envelope = prepared.signing_envelope().map_err(state_error)?;
        let signed = mfm_evm_signing::sign_eip1559(
            &envelope,
            signer_ref,
            prepared
                .intent()
                .expected_sender_address()
                .map_err(state_error)?,
            signer.as_ref(),
        )
        .await
        .map_err(signing_error)?;
        ensure_signed_hash(prepared, &signed)?;
        Ok(reservation.into_lease(prepared.expected_transaction_hash().to_owned(), signed))
    }

    async fn session_for(
        &self,
        prepared: &EvmPreparedTransaction,
        phase: EvmCapabilityPhase,
    ) -> mfm_runtime::Result<Arc<dyn EvmTransactionSession>> {
        self.capabilities
            .bind_session(
                prepared.intent().network_binding().map_err(state_error)?,
                phase,
            )
            .await
    }

    async fn author_prepared_transaction(
        &self,
        intent: &EvmTransactionIntent,
    ) -> mfm_runtime::Result<PreparedTransactionAuthority> {
        intent.validate().map_err(state_error)?;
        let reservation = self.signed_envelopes.reserve()?;
        let session = self
            .capabilities
            .bind_session(
                intent.network_binding().map_err(state_error)?,
                EvmCapabilityPhase::BeforeSubmission,
            )
            .await?;
        let sender = intent.expected_sender_address().map_err(state_error)?;
        let pending_nonce = session
            .pending_nonce(sender)
            .await
            .map_err(pre_submission_capability_error)?;
        let fees = session
            .fee_inputs()
            .await
            .map_err(pre_submission_capability_error)?;
        let estimate = intent
            .transaction_estimate(pending_nonce, &fees)
            .map_err(state_error)?;
        let gas_estimate = session
            .estimate_gas(&estimate)
            .await
            .map_err(pre_submission_capability_error)?;
        let unsigned =
            EvmUnsignedTransaction::from_estimate(&estimate, gas_estimate).map_err(state_error)?;
        let signer_ref = intent.signer_reference().map_err(state_error)?;
        let signer = self.capabilities.bind_signer(signer_ref.clone()).await?;
        let signing_envelope = unsigned.to_signing_envelope().map_err(state_error)?;
        let signed =
            mfm_evm_signing::sign_eip1559(&signing_envelope, signer_ref, sender, signer.as_ref())
                .await
                .map_err(signing_error)?;
        let transaction_hash = signed.transaction_hash();
        let prepared = EvmPreparedTransaction::new(
            intent.clone(),
            pending_nonce,
            fees,
            gas_estimate,
            unsigned,
            transaction_hash,
            session.evidence(),
        )
        .map_err(state_error)?;
        Ok(PreparedTransactionAuthority {
            prepared,
            reservation,
            signed,
        })
    }

    async fn prepare_transaction(
        &self,
        intent: &EvmTransactionIntent,
    ) -> mfm_runtime::Result<SideEffectPreparedInvocation<EvmPreparedTransaction>> {
        let PreparedTransactionAuthority {
            prepared,
            reservation,
            signed,
        } = self.author_prepared_transaction(intent).await?;
        let transaction_hash = prepared.expected_transaction_hash().to_owned();
        let settlement = RunnerOutputSettlement::on_appended(move || {
            reservation.commit(transaction_hash, signed, SignedEnvelopeState::Fresh);
        });
        Ok(SideEffectPreparedInvocation::with_settlement(
            prepared, settlement,
        ))
    }

    #[cfg(test)]
    async fn prepare_committed_transaction_for_test(
        &self,
        intent: &EvmTransactionIntent,
    ) -> mfm_runtime::Result<EvmPreparedTransaction> {
        let PreparedTransactionAuthority {
            prepared,
            reservation,
            signed,
        } = self.author_prepared_transaction(intent).await?;
        reservation.commit(
            prepared.expected_transaction_hash().to_owned(),
            signed,
            SignedEnvelopeState::Fresh,
        );
        Ok(prepared)
    }

    async fn submit_transaction(
        &self,
        prepared: EvmPreparedTransaction,
    ) -> mfm_runtime::Result<
        SideEffectSubmissionDecision<EvmTransactionSubmission, EvmTransactionRecoveryEvidence>,
    > {
        prepared.validate().map_err(state_error)?;
        let session = self
            .session_for(&prepared, EvmCapabilityPhase::BeforeSubmission)
            .await?;
        let mut signed = match self
            .signed_envelopes
            .take(prepared.expected_transaction_hash())?
        {
            Some(signed) => signed,
            None => {
                match lookup_submission(session.as_ref(), &prepared)
                    .await
                    .map_err(pre_submission_capability_error)?
                {
                    LookupSubmission::Observed(submission) => {
                        return Ok(SideEffectSubmissionDecision::Observed(*submission));
                    }
                    LookupSubmission::Mismatched => {
                        return Ok(SideEffectSubmissionDecision::Ambiguous {
                            ambiguity_code: transaction_mismatch_code()?,
                            evidence: recovery_evidence(&prepared, session.as_ref())?,
                        });
                    }
                    LookupSubmission::Missing => {}
                }
                self.sign_prepared_transaction(&prepared).await?
            }
        };
        if signed.state()? == SignedEnvelopeState::Uncertain {
            match lookup_submission(session.as_ref(), &prepared)
                .await
                .map_err(post_submission_capability_error)?
            {
                LookupSubmission::Observed(submission) => {
                    signed.discard()?;
                    return Ok(SideEffectSubmissionDecision::Observed(*submission));
                }
                LookupSubmission::Mismatched => {
                    let evidence = recovery_evidence(&prepared, session.as_ref())?;
                    signed.discard()?;
                    return Ok(SideEffectSubmissionDecision::Ambiguous {
                        ambiguity_code: transaction_mismatch_code()?,
                        evidence,
                    });
                }
                LookupSubmission::Missing => {}
            }
        }
        signed.mark_uncertain()?;
        let signed_envelope = signed.envelope()?;
        ensure_signed_hash(&prepared, signed_envelope)?;
        match broadcast_and_lookup(session.as_ref(), &prepared, signed_envelope).await? {
            LookupSubmission::Observed(submission) => {
                signed.discard()?;
                Ok(SideEffectSubmissionDecision::Observed(*submission))
            }
            LookupSubmission::Mismatched => {
                let evidence = recovery_evidence(&prepared, session.as_ref())?;
                signed.discard()?;
                Ok(SideEffectSubmissionDecision::Ambiguous {
                    ambiguity_code: transaction_mismatch_code()?,
                    evidence,
                })
            }
            LookupSubmission::Missing => Ok(SideEffectSubmissionDecision::Unknown(
                recovery_evidence(&prepared, session.as_ref())?,
            )),
        }
    }

    async fn recover_transaction(
        &self,
        prepared: EvmPreparedTransaction,
    ) -> mfm_runtime::Result<
        SideEffectUnknownSubmissionDecision<
            EvmTransactionSubmission,
            EvmTransactionRecoveryEvidence,
        >,
    > {
        prepared.validate().map_err(state_error)?;
        let session = self
            .session_for(&prepared, EvmCapabilityPhase::AfterSubmission)
            .await?;
        match lookup_submission(session.as_ref(), &prepared)
            .await
            .map_err(post_submission_capability_error)?
        {
            LookupSubmission::Observed(submission) => {
                self.signed_envelopes
                    .discard_one(prepared.expected_transaction_hash())?;
                return Ok(SideEffectUnknownSubmissionDecision::Observed(*submission));
            }
            LookupSubmission::Mismatched => {
                self.signed_envelopes
                    .discard_one(prepared.expected_transaction_hash())?;
                return Ok(SideEffectUnknownSubmissionDecision::Ambiguous {
                    ambiguity_code: transaction_mismatch_code()?,
                    evidence: recovery_evidence(&prepared, session.as_ref())?,
                });
            }
            LookupSubmission::Missing => {}
        }
        let mut signed = match self
            .signed_envelopes
            .take(prepared.expected_transaction_hash())?
        {
            Some(signed) => signed,
            None => self.sign_prepared_transaction(&prepared).await?,
        };
        signed.mark_uncertain()?;
        let signed_envelope = signed.envelope()?;
        ensure_signed_hash(&prepared, signed_envelope)?;
        match broadcast_and_lookup(session.as_ref(), &prepared, signed_envelope).await? {
            LookupSubmission::Observed(submission) => {
                signed.discard()?;
                Ok(SideEffectUnknownSubmissionDecision::Observed(*submission))
            }
            LookupSubmission::Mismatched => {
                let evidence = recovery_evidence(&prepared, session.as_ref())?;
                signed.discard()?;
                Ok(SideEffectUnknownSubmissionDecision::Ambiguous {
                    ambiguity_code: transaction_mismatch_code()?,
                    evidence,
                })
            }
            LookupSubmission::Missing => Ok(SideEffectUnknownSubmissionDecision::StillUnknown),
        }
    }
}

struct SignedEnvelopeCache {
    state: Mutex<SignedEnvelopeCacheState>,
}

#[derive(Default)]
struct SignedEnvelopeCacheState {
    entries: VecDeque<SignedEnvelopeEntry>,
    reservations: usize,
}

struct SignedEnvelopeEntry {
    transaction_hash: String,
    envelope: TransientSignedEip1559Envelope,
    state: SignedEnvelopeState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SignedEnvelopeState {
    Fresh,
    Uncertain,
}

impl Default for SignedEnvelopeCache {
    fn default() -> Self {
        Self {
            state: Mutex::new(SignedEnvelopeCacheState::default()),
        }
    }
}

impl SignedEnvelopeCache {
    fn reserve(self: &Arc<Self>) -> mfm_runtime::Result<SignedEnvelopeReservation> {
        let mut state = self.state.lock().map_err(|_| cache_error())?;
        if state.entries.len() + state.reservations >= SIGNED_ENVELOPE_CACHE_CAPACITY {
            return Err(mfm_runtime::RuntimeError::Blocked(
                "transient signed-envelope capacity is saturated".to_owned(),
            ));
        }
        state.reservations += 1;
        Ok(SignedEnvelopeReservation {
            cache: Arc::clone(self),
            active: true,
        })
    }

    fn take(
        self: &Arc<Self>,
        transaction_hash: &str,
    ) -> mfm_runtime::Result<Option<SignedEnvelopeLease>> {
        let mut state = self.state.lock().map_err(|_| cache_error())?;
        let Some(index) = state
            .entries
            .iter()
            .position(|entry| entry.transaction_hash == transaction_hash)
        else {
            return Ok(None);
        };
        let entry = state.entries.remove(index).ok_or_else(cache_error)?;
        state.reservations += 1;
        Ok(Some(SignedEnvelopeLease {
            cache: Arc::clone(self),
            entry: Some(entry),
        }))
    }

    fn discard_one(self: &Arc<Self>, transaction_hash: &str) -> mfm_runtime::Result<()> {
        if let Some(envelope) = self.take(transaction_hash)? {
            envelope.discard()?;
        }
        Ok(())
    }

    fn release_reservation(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.reservations > 0 {
            state.reservations -= 1;
        }
    }

    fn restore(&self, entry: SignedEnvelopeEntry) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.reservations > 0 {
            state.reservations -= 1;
            state.entries.push_back(entry);
        }
    }
}

struct SignedEnvelopeReservation {
    cache: Arc<SignedEnvelopeCache>,
    active: bool,
}

impl SignedEnvelopeReservation {
    fn commit(
        mut self,
        transaction_hash: String,
        envelope: TransientSignedEip1559Envelope,
        envelope_state: SignedEnvelopeState,
    ) {
        let mut state = self
            .cache
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.reservations == 0 {
            self.active = false;
            return;
        }
        state.reservations -= 1;
        state.entries.push_back(SignedEnvelopeEntry {
            transaction_hash,
            envelope,
            state: envelope_state,
        });
        self.active = false;
    }

    fn into_lease(
        mut self,
        transaction_hash: String,
        envelope: TransientSignedEip1559Envelope,
    ) -> SignedEnvelopeLease {
        self.active = false;
        SignedEnvelopeLease {
            cache: Arc::clone(&self.cache),
            entry: Some(SignedEnvelopeEntry {
                transaction_hash,
                envelope,
                state: SignedEnvelopeState::Fresh,
            }),
        }
    }
}

impl Drop for SignedEnvelopeReservation {
    fn drop(&mut self) {
        if self.active {
            self.cache.release_reservation();
        }
    }
}

struct SignedEnvelopeLease {
    cache: Arc<SignedEnvelopeCache>,
    entry: Option<SignedEnvelopeEntry>,
}

impl SignedEnvelopeLease {
    fn envelope(&self) -> mfm_runtime::Result<&TransientSignedEip1559Envelope> {
        self.entry
            .as_ref()
            .map(|entry| &entry.envelope)
            .ok_or_else(cache_error)
    }

    fn state(&self) -> mfm_runtime::Result<SignedEnvelopeState> {
        self.entry
            .as_ref()
            .map(|entry| entry.state)
            .ok_or_else(cache_error)
    }

    fn mark_uncertain(&mut self) -> mfm_runtime::Result<()> {
        self.entry.as_mut().ok_or_else(cache_error)?.state = SignedEnvelopeState::Uncertain;
        Ok(())
    }

    fn discard(mut self) -> mfm_runtime::Result<()> {
        let mut state = self.cache.state.lock().map_err(|_| cache_error())?;
        if state.reservations == 0 {
            return Err(cache_error());
        }
        state.reservations -= 1;
        self.entry.take();
        Ok(())
    }
}

impl Drop for SignedEnvelopeLease {
    fn drop(&mut self) {
        if let Some(entry) = self.entry.take() {
            self.cache.restore(entry);
        }
    }
}

impl SideEffectAdapter for EvmTransactionAdapter {
    type Intent = EvmTransactionIntent;
    type Idempotency = EvmTransactionIntent;
    type PreparedInvocation = EvmPreparedTransaction;
    type Submission = EvmTransactionSubmission;
    type RecoveryEvidence = EvmTransactionRecoveryEvidence;
    type Receipt = EvmTransactionReceipt;
    type Confirmation = EvmTransactionConfirmation;
    type Output = EvmTransactionOutcome;

    fn capability_binding(&self) -> mfm_runtime::Result<RunnerCapabilityBinding> {
        transaction_capability_binding()
    }

    fn authored_intent<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
    ) -> SideEffectDriverFuture<'a, mfm_program::SideEffectIntent<Self::Intent, Self::Idempotency>>
    {
        let artifacts = Arc::clone(&self.capabilities.artifacts);
        Box::pin(async move {
            let (state, input, context) =
                transaction_state_material(ctx, submit_node, submit_inputs, artifacts.as_ref())
                    .await?;
            state.intent(&input, &context).map_err(state_error)
        })
    }

    fn prepare<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        intent: &'a Self::Intent,
        _idempotency: &'a Self::Idempotency,
    ) -> SideEffectDriverFuture<'a, SideEffectPreparedInvocation<Self::PreparedInvocation>> {
        Box::pin(async move { self.prepare_transaction(intent).await })
    }

    fn load_prepared<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        prepared: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::PreparedInvocation> {
        let artifacts = Arc::clone(&self.capabilities.artifacts);
        Box::pin(async move {
            let prepared = load_transaction_artifact::<EvmPreparedTransaction>(
                prepared,
                events::ArtifactRole::PreparedInvocation,
                submit_node,
                artifacts.as_ref(),
            )
            .await?;
            prepared.validate().map_err(state_error)?;
            Ok(prepared)
        })
    }

    fn submit_prepared<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        prepared: Self::PreparedInvocation,
    ) -> SideEffectDriverFuture<
        'a,
        SideEffectSubmissionDecision<Self::Submission, Self::RecoveryEvidence>,
    > {
        Box::pin(async move { self.submit_transaction(prepared).await })
    }

    fn recover_unknown<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        _submit_node: &'a spec::NodeSpec,
        _submit_inputs: &'a MaterializedInputs,
        prepared: Self::PreparedInvocation,
    ) -> SideEffectDriverFuture<
        'a,
        SideEffectUnknownSubmissionDecision<Self::Submission, Self::RecoveryEvidence>,
    > {
        Box::pin(async move { self.recover_transaction(prepared).await })
    }

    fn observe_receipt<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        _submit_inputs: &'a MaterializedInputs,
        prepared: &'a store::SideEffectArtifactProjection,
        submission: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Receipt>> {
        let artifacts = Arc::clone(&self.capabilities.artifacts);
        Box::pin(async move {
            let prepared = load_transaction_artifact::<EvmPreparedTransaction>(
                prepared,
                events::ArtifactRole::PreparedInvocation,
                submit_node,
                artifacts.as_ref(),
            )
            .await?;
            let submission = load_transaction_artifact::<EvmTransactionSubmission>(
                submission,
                events::ArtifactRole::Submission,
                submit_node,
                artifacts.as_ref(),
            )
            .await?;
            submission
                .validate_against(&prepared)
                .map_err(state_error)?;
            let session = self
                .session_for(&prepared, EvmCapabilityPhase::AfterSubmission)
                .await?;
            let Some(receipt) = session
                .receipt_by_hash(prepared.expected_hash().map_err(state_error)?)
                .await
                .map_err(post_submission_capability_error)?
            else {
                return Err(transaction_pending("receipt is not yet available"));
            };
            let receipt =
                EvmTransactionReceipt::from_observation(&prepared, &receipt, session.evidence())
                    .map_err(state_error)?;
            receipt
                .validate_with_submission(&prepared, &submission)
                .map_err(state_error)?;
            Ok(SideEffectObservedEvidence::new(
                receipt,
                transaction_replay_evidence()?,
            ))
        })
    }

    fn observe_confirmation<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        _submit_inputs: &'a MaterializedInputs,
        prepared: &'a store::SideEffectArtifactProjection,
        submission: &'a store::SideEffectArtifactProjection,
        receipt: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Confirmation>> {
        let artifacts = Arc::clone(&self.capabilities.artifacts);
        Box::pin(async move {
            let prepared = load_transaction_artifact::<EvmPreparedTransaction>(
                prepared,
                events::ArtifactRole::PreparedInvocation,
                submit_node,
                artifacts.as_ref(),
            )
            .await?;
            let submission = load_transaction_artifact::<EvmTransactionSubmission>(
                submission,
                events::ArtifactRole::Submission,
                submit_node,
                artifacts.as_ref(),
            )
            .await?;
            submission
                .validate_against(&prepared)
                .map_err(state_error)?;
            let retained_receipt = load_transaction_artifact::<EvmTransactionReceipt>(
                receipt,
                events::ArtifactRole::Receipt,
                ctx.node(),
                artifacts.as_ref(),
            )
            .await?;
            retained_receipt
                .validate_with_submission(&prepared, &submission)
                .map_err(state_error)?;
            let required_depth = finalized_depth(submit_node)?;
            let session = self
                .session_for(&prepared, EvmCapabilityPhase::AfterSubmission)
                .await?;
            let Some(fresh_receipt) = session
                .receipt_by_hash(prepared.expected_hash().map_err(state_error)?)
                .await
                .map_err(post_submission_capability_error)?
            else {
                return Err(transaction_pending(
                    "receipt disappeared before confirmation",
                ));
            };
            let fresh_receipt = EvmTransactionReceipt::from_observation(
                &prepared,
                &fresh_receipt,
                session.evidence(),
            )
            .map_err(state_error)?;
            if !retained_receipt.matches_chain_receipt(&fresh_receipt) {
                return Err(transaction_pending("receipt moved before confirmation"));
            }
            let receipt_number = retained_receipt
                .block_number_quantity()
                .map_err(state_error)?;
            let canonical_block = session
                .read_block(&EvmBlockSelector::Number(receipt_number))
                .await
                .map_err(post_submission_capability_error)?;
            let canonical_number = canonical_block.number_quantity().map_err(|_| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(
                    "EVM capability returned an invalid canonical block number".to_owned(),
                )
            })?;
            let canonical_hash = canonical_block.hash_value().map_err(|_| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(
                    "EVM capability returned an invalid canonical block hash".to_owned(),
                )
            })?;
            if canonical_number != receipt_number
                || canonical_hash != retained_receipt.block_hash_value().map_err(state_error)?
            {
                return Err(transaction_pending("receipt block is no longer canonical"));
            }
            let head = session
                .read_block(&EvmBlockSelector::Latest)
                .await
                .map_err(post_submission_capability_error)?;
            let head_number = head.number_quantity().map_err(|_| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(
                    "EVM capability returned an invalid head block number".to_owned(),
                )
            })?;
            let confirmations = head_number
                .checked_sub(receipt_number)
                .and_then(|distance| distance.checked_add(U256::from(1)));
            if confirmations.is_none_or(|count| count < U256::from(required_depth)) {
                return Err(transaction_pending(
                    "certified confirmation depth is not yet available",
                ));
            }
            let confirmation = EvmTransactionConfirmation::new(
                &prepared,
                &retained_receipt,
                fresh_receipt,
                &canonical_block,
                &head,
                required_depth,
                session.evidence(),
            )
            .map_err(state_error)?;
            Ok(SideEffectObservedEvidence::new(
                confirmation,
                transaction_replay_evidence()?,
            ))
        })
    }

    fn output_from_receipt<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_inputs: &'a MaterializedInputs,
        prepared: &'a store::SideEffectArtifactProjection,
        submission: &'a store::SideEffectArtifactProjection,
        receipt: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::Output> {
        let artifacts = Arc::clone(&self.capabilities.artifacts);
        Box::pin(async move {
            let submit_node = transaction_submit_node(ctx)?;
            let (state, input, context) =
                transaction_state_material(ctx, submit_node, submit_inputs, artifacts.as_ref())
                    .await?;
            let prepared = load_transaction_artifact::<EvmPreparedTransaction>(
                prepared,
                events::ArtifactRole::PreparedInvocation,
                submit_node,
                artifacts.as_ref(),
            )
            .await?;
            let submission = load_transaction_artifact::<EvmTransactionSubmission>(
                submission,
                events::ArtifactRole::Submission,
                submit_node,
                artifacts.as_ref(),
            )
            .await?;
            let receipt = load_transaction_artifact::<EvmTransactionReceipt>(
                receipt,
                events::ArtifactRole::Receipt,
                ctx.node(),
                artifacts.as_ref(),
            )
            .await?;
            state
                .output_from_receipt(&input, &prepared, &submission, &receipt, &context)
                .map_err(state_error)
        })
    }

    fn output_from_confirmation<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_inputs: &'a MaterializedInputs,
        prepared: &'a store::SideEffectArtifactProjection,
        submission: &'a store::SideEffectArtifactProjection,
        receipt: &'a store::SideEffectArtifactProjection,
        confirmation: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::Output> {
        let artifacts = Arc::clone(&self.capabilities.artifacts);
        Box::pin(async move {
            let submit_node = transaction_submit_node(ctx)?;
            let (state, input, context) =
                transaction_state_material(ctx, submit_node, submit_inputs, artifacts.as_ref())
                    .await?;
            let prepared = load_transaction_artifact::<EvmPreparedTransaction>(
                prepared,
                events::ArtifactRole::PreparedInvocation,
                submit_node,
                artifacts.as_ref(),
            )
            .await?;
            let submission = load_transaction_artifact::<EvmTransactionSubmission>(
                submission,
                events::ArtifactRole::Submission,
                submit_node,
                artifacts.as_ref(),
            )
            .await?;
            let receipt = load_transaction_artifact::<EvmTransactionReceipt>(
                receipt,
                events::ArtifactRole::Receipt,
                ctx.node(),
                artifacts.as_ref(),
            )
            .await?;
            let confirmation = load_transaction_artifact::<EvmTransactionConfirmation>(
                confirmation,
                events::ArtifactRole::Confirmation,
                ctx.node(),
                artifacts.as_ref(),
            )
            .await?;
            state
                .output_from_confirmation(
                    &input,
                    &prepared,
                    &submission,
                    &receipt,
                    &confirmation,
                    &context,
                )
                .map_err(state_error)
        })
    }
}

enum LookupSubmission {
    Observed(Box<EvmTransactionSubmission>),
    Mismatched,
    Missing,
}

async fn broadcast_and_lookup(
    session: &dyn EvmTransactionSession,
    prepared: &EvmPreparedTransaction,
    signed: &TransientSignedEip1559Envelope,
) -> mfm_runtime::Result<LookupSubmission> {
    let Ok(expected_hash) = prepared.expected_hash() else {
        return Ok(LookupSubmission::Mismatched);
    };
    let submission_error = match session
        .submit_raw_transaction(signed.bytes(), expected_hash)
        .await
    {
        Ok(returned_hash) if returned_hash != expected_hash => {
            return Ok(LookupSubmission::Mismatched);
        }
        Ok(_) => None,
        Err(error) => Some(error),
    };
    match lookup_submission(session, prepared).await {
        Ok(LookupSubmission::Observed(submission)) => Ok(LookupSubmission::Observed(submission)),
        Ok(LookupSubmission::Mismatched) => Ok(LookupSubmission::Mismatched),
        Ok(LookupSubmission::Missing) | Err(_) => {
            if let Some(error) = submission_error.filter(|error| {
                error.failure_disposition(EvmCapabilityPhase::BeforeSubmission)
                    == EvmCapabilityFailureDisposition::TerminalValidation
            }) {
                return Err(pre_submission_capability_error(error));
            }
            Ok(LookupSubmission::Missing)
        }
    }
}

async fn lookup_submission(
    session: &dyn EvmTransactionSession,
    prepared: &EvmPreparedTransaction,
) -> mfm_evm_capabilities::Result<LookupSubmission> {
    let Ok(expected_hash) = prepared.expected_hash() else {
        return Ok(LookupSubmission::Mismatched);
    };
    match session.transaction_by_hash(expected_hash).await {
        Ok(Some(observation)) => match EvmTransactionSubmission::from_observation(
            prepared,
            &observation,
            session.evidence(),
        ) {
            Ok(submission) => Ok(LookupSubmission::Observed(Box::new(submission))),
            Err(_) => Ok(LookupSubmission::Mismatched),
        },
        Ok(None) => Ok(LookupSubmission::Missing),
        Err(error) => Err(error),
    }
}

async fn transaction_state_material(
    ctx: &ErasedRunCtx<'_>,
    submit_node: &spec::NodeSpec,
    submit_inputs: &MaterializedInputs,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> mfm_runtime::Result<(
    SubmitEvmTransactionState,
    EvmTransactionAction,
    mfm_program::CertifiedContext<mfm_program::NoContext>,
)> {
    let config =
        load_runner_config_for_node::<EvmTransactionConfig>(submit_node, artifacts).await?;
    let state = SubmitEvmTransactionState::new(config)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let input =
        load_materialized_struct_input::<EvmTransactionAction>(submit_inputs, artifacts).await?;
    let context = ctx
        .invocation_context_for_node(submit_node)?
        .certified_context::<mfm_program::NoContext>()?;
    Ok((state, input, context))
}

async fn preclaim_state_material(
    ctx: &PreInvocationRunCtx<'_>,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> mfm_runtime::Result<(
    SubmitEvmTransactionState,
    EvmTransactionAction,
    mfm_program::CertifiedContext<mfm_program::NoContext>,
)> {
    let config = load_runner_config_for_node::<EvmTransactionConfig>(ctx.node(), artifacts).await?;
    let state = SubmitEvmTransactionState::new(config)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let input =
        load_materialized_struct_input::<EvmTransactionAction>(ctx.inputs(), artifacts).await?;
    let context = ctx
        .context()
        .clone()
        .certified_context::<mfm_program::NoContext>()?;
    Ok((state, input, context))
}

fn transaction_submit_node<'a>(
    ctx: &'a ErasedRunCtx<'a>,
) -> mfm_runtime::Result<&'a spec::NodeSpec> {
    let Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) = &ctx.node().framework else {
        return Ok(ctx.node());
    };
    ctx.certified_node(&verify.submit_node_id).ok_or_else(|| {
        mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "EVM transaction verify node referenced a missing submit node".to_owned(),
        )
    })
}

async fn load_transaction_artifact<T>(
    artifact: &store::SideEffectArtifactProjection,
    role: events::ArtifactRole,
    _producer_node: &spec::NodeSpec,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> mfm_runtime::Result<T>
where
    T: MfmValue + DeserializeOwned,
{
    load_side_effect_artifact::<T>(artifact, role, artifacts)
        .await
        .map(|(value, _)| value)
}

fn sender_lane_resource_key(
    node: &spec::NodeSpec,
    lane: &EvmSenderLane,
) -> mfm_runtime::Result<events::ResourceKeyEvidence> {
    let Some(spec::SideEffectContractSpec {
        resource_claim:
            spec::ResourceClaimSpec::Exclusive {
                namespace,
                key_schema,
            },
        ..
    }) = &node.side_effect
    else {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "EVM transaction node requires an exclusive sender lane".to_owned(),
        ));
    };
    let expected_schema = EvmSenderLane::schema_id()
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    if namespace.as_str() != EVM_SENDER_LANE_NAMESPACE || key_schema != &expected_schema {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "EVM transaction node declared the wrong sender lane schema".to_owned(),
        ));
    }
    Ok(events::ResourceKeyEvidence {
        namespace: namespace.clone(),
        key_schema_id: key_schema.clone(),
        key: events::ResourceKey::new(lane.resource_key())?,
    })
}

fn transaction_capability_binding() -> mfm_runtime::Result<RunnerCapabilityBinding> {
    RunnerCapabilityBinding::for_capability::<EvmTransactionCapability>(
        evm_jsonrpc_adapter_kind().map_err(adapter_identity_error)?,
        evm_jsonrpc_adapter_version().map_err(adapter_identity_error)?,
    )
}

fn finalized_depth(node: &spec::NodeSpec) -> mfm_runtime::Result<u64> {
    match node
        .side_effect
        .as_ref()
        .map(|contract| &contract.verification)
    {
        Some(spec::SideEffectVerificationSpec::Finalized { depth }) if *depth > 0 => Ok(*depth),
        _ => Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "EVM transaction confirmation requires positive finalized depth".to_owned(),
        )),
    }
}

fn ensure_signed_hash(
    prepared: &EvmPreparedTransaction,
    signed: &TransientSignedEip1559Envelope,
) -> mfm_runtime::Result<()> {
    if signed.transaction_hash() != prepared.expected_hash().map_err(state_error)? {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "regenerated signed envelope did not match prepared transaction hash".to_owned(),
        ));
    }
    Ok(())
}

fn recovery_evidence(
    prepared: &EvmPreparedTransaction,
    session: &dyn EvmTransactionSession,
) -> mfm_runtime::Result<EvmTransactionRecoveryEvidence> {
    EvmTransactionRecoveryEvidence::new(prepared, session.evidence()).map_err(state_error)
}

fn transaction_mismatch_code() -> mfm_runtime::Result<events::AmbiguityCode> {
    events::AmbiguityCode::new("mfm.evm.transaction_mismatch").map_err(Into::into)
}

fn transaction_replay_evidence() -> mfm_runtime::Result<SideEffectReplayEvidence> {
    Ok(SideEffectReplayEvidence::new(replay_verifier_id()?, None))
}

fn replay_verifier_id() -> mfm_runtime::Result<events::ReplayVerifierId> {
    events::ReplayVerifierId::new(REPLAY_VERIFIER_ID).map_err(Into::into)
}

fn state_error(error: impl std::fmt::Display) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn pre_submission_capability_error(error: EvmCapabilityError) -> mfm_runtime::RuntimeError {
    evm_capability_runtime_error(error, EvmCapabilityPhase::BeforeSubmission)
}

fn post_submission_capability_error(error: EvmCapabilityError) -> mfm_runtime::RuntimeError {
    evm_capability_runtime_error(error, EvmCapabilityPhase::AfterSubmission)
}

fn signing_error(error: impl std::fmt::Display) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn cache_error() -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(
        "transient signed-envelope cache was unavailable".to_owned(),
    )
}

fn transaction_pending(message: &'static str) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::Blocked(message.to_owned())
}

struct EvmTransactionReplayVerifier {
    verifier_id: events::ReplayVerifierId,
}

impl EvmTransactionReplayVerifier {
    fn new() -> replay::Result<Self> {
        Ok(Self {
            verifier_id: events::ReplayVerifierId::new(REPLAY_VERIFIER_ID)
                .map_err(replay_identity_error)?,
        })
    }
}

impl replay::SideEffectReplayVerifier for EvmTransactionReplayVerifier {
    fn verifier_id(&self) -> &events::ReplayVerifierId {
        &self.verifier_id
    }

    fn verify_submission(
        &self,
        input: &replay::SideEffectSubmissionReplayInput,
    ) -> replay::Result<()> {
        let (_, prepared) =
            replay_intent_and_prepared(&input.intent, input.prepared_invocation.as_ref())?;
        ensure_replay_schema::<EvmTransactionSubmission>(
            &input.submission.submission.submission_schema_id,
        )?;
        let submission = decode_replay::<EvmTransactionSubmission>(
            "submission",
            &input.submission.artifact_bytes,
        )?;
        submission
            .validate_against(&prepared)
            .map_err(replay_state_error)
    }

    fn verify_receipt(&self, input: &replay::SideEffectReceiptReplayInput) -> replay::Result<()> {
        let (_, prepared) =
            replay_intent_and_prepared(&input.intent, input.prepared_invocation.as_ref())?;
        let submission = input
            .submission
            .as_ref()
            .ok_or_else(|| replay_missing("submission"))?;
        ensure_replay_schema::<EvmTransactionSubmission>(
            &submission.submission.submission_schema_id,
        )?;
        let submission =
            decode_replay::<EvmTransactionSubmission>("submission", &submission.artifact_bytes)?;
        ensure_replay_schema::<EvmTransactionReceipt>(&input.receipt.receipt.receipt_schema_id)?;
        let receipt =
            decode_replay::<EvmTransactionReceipt>("receipt", &input.receipt.artifact_bytes)?;
        receipt
            .validate_with_submission(&prepared, &submission)
            .map_err(replay_state_error)
    }

    fn verify_confirmation(
        &self,
        input: &replay::SideEffectConfirmationReplayInput,
    ) -> replay::Result<()> {
        let (_, prepared) =
            replay_intent_and_prepared(&input.intent, input.prepared_invocation.as_ref())?;
        let submission = input
            .submission
            .as_ref()
            .ok_or_else(|| replay_missing("submission"))?;
        let receipt = input
            .receipt
            .as_ref()
            .ok_or_else(|| replay_missing("receipt"))?;
        ensure_replay_schema::<EvmTransactionSubmission>(
            &submission.submission.submission_schema_id,
        )?;
        ensure_replay_schema::<EvmTransactionReceipt>(&receipt.receipt.receipt_schema_id)?;
        ensure_replay_schema::<EvmTransactionConfirmation>(
            &input.confirmation.confirmation.confirmation_schema_id,
        )?;
        let submission =
            decode_replay::<EvmTransactionSubmission>("submission", &submission.artifact_bytes)?;
        let receipt = decode_replay::<EvmTransactionReceipt>("receipt", &receipt.artifact_bytes)?;
        let confirmation = decode_replay::<EvmTransactionConfirmation>(
            "confirmation",
            &input.confirmation.artifact_bytes,
        )?;
        receipt
            .validate_with_submission(&prepared, &submission)
            .map_err(replay_state_error)?;
        confirmation
            .validate_against(&prepared, &receipt)
            .map_err(replay_state_error)?;
        match input.verification {
            spec::SideEffectVerificationSpec::Finalized { depth }
                if depth == confirmation.required_depth() =>
            {
                Ok(())
            }
            _ => Err(replay_mismatch(
                "confirmation depth differed from certified verification policy",
            )),
        }
    }
}

fn replay_intent_and_prepared(
    intent: &replay::SideEffectIntentReplayEvidence,
    prepared: Option<&replay::PreparedInvocationReplayEvidence>,
) -> replay::Result<(EvmTransactionIntent, EvmPreparedTransaction)> {
    let intent_value = replay_intent_value(intent)?;
    let prepared = prepared.ok_or_else(|| replay_missing("prepared invocation"))?;
    ensure_replay_schema::<EvmPreparedTransaction>(&prepared.prepared.prepared_schema_id)?;
    let prepared =
        decode_replay::<EvmPreparedTransaction>("prepared invocation", &prepared.artifact_bytes)?;
    prepared.validate().map_err(replay_state_error)?;
    verify_prepared_intent(&intent_value, &prepared)?;
    Ok((intent_value, prepared))
}

fn verify_prepared_intent(
    retained_intent: &EvmTransactionIntent,
    prepared: &EvmPreparedTransaction,
) -> replay::Result<()> {
    if prepared.intent() == retained_intent {
        Ok(())
    } else {
        Err(replay_mismatch(
            "prepared intent differed from retained intent",
        ))
    }
}

fn ensure_replay_schema<T: MfmValue>(actual: &mfm_ids::SchemaId) -> replay::Result<()> {
    let expected = T::schema_id().map_err(replay_value_error)?;
    if actual == &expected {
        Ok(())
    } else {
        Err(replay_mismatch(
            "side-effect evidence schema was unexpected",
        ))
    }
}

fn decode_replay<T: DeserializeOwned>(label: &str, bytes: &[u8]) -> replay::Result<T> {
    serde_json::from_slice(bytes).map_err(|_| {
        replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            format!("EVM transaction {label} failed strict typed decoding"),
        )
    })
}

fn canonical_digest<T: Serialize>(value: &T) -> replay::Result<mfm_ids::ContentDigest> {
    let json = serde_json::to_string(value).map_err(replay_value_error)?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map(|bytes| bytes.content_digest())
        .map_err(replay_value_error)
}

/// Identifies side-effect intents owned by the EVM transaction replay verifier.
pub fn is_evm_transaction_replay_intent(
    intent: &side_effect::IntentPersisted,
) -> replay::Result<bool> {
    let capability = EvmTransactionCapability::kind().map_err(replay_value_error)?;
    let capability_version = EvmTransactionCapability::version().map_err(replay_value_error)?;
    let adapter = evm_jsonrpc_adapter_kind().map_err(replay_value_error)?;
    let adapter_version = evm_jsonrpc_adapter_version().map_err(replay_value_error)?;
    Ok(intent.capability_kind == capability
        && intent.capability_version == capability_version
        && intent.adapter_kind == adapter
        && intent.adapter_version == adapter_version)
}

/// Verifies all retained EVM transaction relations without live or signer resources.
pub fn verify_evm_transaction_replay(broker: &replay::ReplayBroker) -> replay::Result<()> {
    let frames = broker.side_effect_replay_frames_matching(is_evm_transaction_replay_intent)?;
    let verifier = EvmTransactionReplayVerifier::new()?;
    for frame in frames {
        if frame.not_submitted.is_some() {
            return Err(replay_mismatch(
                "EVM transaction replay contained a forbidden non-submission proof",
            ));
        }
        let intent_request = replay_frame_intent_request(&frame);
        let intent_evidence = broker.side_effect_intent_evidence(&intent_request)?;
        let intent = replay_intent_value(&intent_evidence)?;
        let pair = broker
            .certified_spec()
            .spec
            .side_effect_verify_pair_for_pair_id(&frame.intent.pair_id)
            .map_err(replay_value_error)?;
        if pair.submit_node.node_id != frame.intent.node_id {
            return Err(replay_mismatch(
                "EVM transaction intent was not owned by its certified submit node",
            ));
        }
        let config = replay::load_node_config::<EvmTransactionConfig>(broker, pair.submit_node)?;
        let state = SubmitEvmTransactionState::new(
            ValidatedConfig::new(config).map_err(replay_state_error)?,
        )
        .map_err(replay_state_error)?;
        let input = replay::load_node_input::<EvmTransactionAction>(broker, pair.submit_node)?;
        let context =
            replay::load_node_context::<mfm_program::NoContext>(broker, pair.submit_node)?;
        let authored = state.intent(&input, &context).map_err(replay_state_error)?;
        verify_authored_transaction_replay(
            authored.intent(),
            authored.idempotency(),
            &intent,
            &frame.intent.idempotency_key,
        )?;
        let prepared = match frame.prepared_request() {
            Some(request) => {
                let prepared_evidence = broker.side_effect_prepared_invocation(&request)?;
                let (_, prepared) =
                    replay_intent_and_prepared(&intent_evidence, Some(&prepared_evidence))?;
                let expected_resource_key =
                    sender_lane_resource_key(pair.submit_node, &authored.intent().sender_lane())
                        .map_err(replay_state_error)?;
                verify_prepared_sender_lane(
                    prepared_evidence.prepared.resource_key.as_ref(),
                    &expected_resource_key,
                )?;
                Some(prepared)
            }
            None => None,
        };
        if let Some(request) = frame.submission_unknown_request() {
            let prepared = prepared
                .as_ref()
                .ok_or_else(|| replay_missing("prepared invocation"))?;
            let unknown = broker.side_effect_submission_unknown(&request)?;
            ensure_replay_schema::<EvmTransactionRecoveryEvidence>(
                &unknown.unknown.evidence_schema_id,
            )?;
            decode_replay::<EvmTransactionRecoveryEvidence>(
                "submission-unknown evidence",
                &unknown.artifact_bytes,
            )?
            .validate_against(prepared)
            .map_err(replay_state_error)?;
        }
        if let Some(request) = frame.ambiguity_request() {
            let prepared = prepared
                .as_ref()
                .ok_or_else(|| replay_missing("prepared invocation"))?;
            let ambiguity = broker.side_effect_ambiguity(&request)?;
            if ambiguity.ambiguity.ambiguity_code != replay_transaction_mismatch_code()? {
                return Err(replay_mismatch(
                    "EVM transaction ambiguity code was unexpected",
                ));
            }
            ensure_replay_schema::<EvmTransactionRecoveryEvidence>(
                &ambiguity.ambiguity.evidence_schema_id,
            )?;
            decode_replay::<EvmTransactionRecoveryEvidence>(
                "ambiguity evidence",
                &ambiguity.artifact_bytes,
            )?
            .validate_against(prepared)
            .map_err(replay_state_error)?;
        }
        if let Some(request) = frame.submission_request() {
            broker.verify_side_effect_submission(&request, &verifier)?;
        }
        if let Some(request) = frame.receipt_request() {
            broker.verify_side_effect_receipt(&request, &verifier)?;
        }
        if let Some(request) = frame.confirmation_request() {
            broker.verify_side_effect_confirmation(&request, &verifier)?;
        }
        verify_evm_transaction_output(broker, &frame, prepared.as_ref(), &state, &input, &context)?;
    }
    Ok(())
}

fn verify_authored_transaction_replay(
    authored_intent: &EvmTransactionIntent,
    authored_idempotency: &EvmTransactionIntent,
    retained_intent: &EvmTransactionIntent,
    retained_idempotency_key: &events::IdempotencyKeyRef,
) -> replay::Result<()> {
    if authored_intent != retained_intent || authored_idempotency != retained_intent {
        return Err(replay_mismatch(
            "certified EVM transaction state authored a different intent",
        ));
    }
    let expected = side_effect_idempotency_key(authored_idempotency).map_err(replay_state_error)?;
    if retained_idempotency_key != &expected {
        return Err(replay_mismatch(
            "EVM transaction idempotency key differed from the certified state",
        ));
    }
    Ok(())
}

fn verify_prepared_sender_lane(
    retained: Option<&events::ResourceKeyEvidence>,
    expected: &events::ResourceKeyEvidence,
) -> replay::Result<()> {
    if retained == Some(expected) {
        Ok(())
    } else {
        Err(replay_mismatch(
            "prepared EVM transaction used a different certified sender lane",
        ))
    }
}

fn replay_frame_intent_request(
    frame: &replay::SideEffectReplayFrame<'_>,
) -> replay::SideEffectEvidenceReplayRequest {
    replay::SideEffectEvidenceReplayRequest {
        pair_id: frame.intent.pair_id.clone(),
        invocation_epoch: frame.intent.invocation_epoch,
        certified_context: frame.certified_context.clone(),
        evidence_schema_id: frame.intent.intent_schema_id.clone(),
        evidence_hash: frame.intent.intent_hash.clone(),
        replay_verifier_id: None,
    }
}

fn replay_intent_value(
    intent: &replay::SideEffectIntentReplayEvidence,
) -> replay::Result<EvmTransactionIntent> {
    ensure_replay_schema::<EvmTransactionIntent>(&intent.intent.intent_schema_id)?;
    ensure_replay_schema::<EvmTransactionIntent>(&intent.intent.idempotency_input_schema_id)?;
    let value = decode_replay::<EvmTransactionIntent>("intent", &intent.artifact_bytes)?;
    value.validate().map_err(replay_state_error)?;
    let digest = canonical_digest(&value)?;
    if intent.intent.intent_hash != digest || intent.intent.idempotency_input_hash != digest {
        return Err(replay_mismatch(
            "authored intent or idempotency hash differed from retained value",
        ));
    }
    Ok(value)
}

fn verify_evm_transaction_output(
    broker: &replay::ReplayBroker,
    frame: &replay::SideEffectReplayFrame<'_>,
    prepared: Option<&EvmPreparedTransaction>,
    state: &SubmitEvmTransactionState,
    input: &EvmTransactionAction,
    context: &mfm_program::CertifiedContext<mfm_program::NoContext>,
) -> replay::Result<()> {
    let pair = broker
        .certified_spec()
        .spec
        .side_effect_verify_pair_for_pair_id(&frame.intent.pair_id)
        .map_err(replay_value_error)?;
    let verify_node_id = pair.verify_node.node_id.clone();
    let verification = pair.submit_contract.verification.clone();
    let output_frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.node_id == verify_node_id)
    })?;
    let terminal = match verification {
        spec::SideEffectVerificationSpec::Receipt => {
            if frame.confirmation.is_some() {
                return Err(replay_mismatch(
                    "receipt-terminal EVM transaction recorded confirmation evidence",
                ));
            }
            frame.receipt.is_some()
        }
        spec::SideEffectVerificationSpec::Finalized { .. } => frame.confirmation.is_some(),
    };
    if !terminal {
        if output_frames.is_empty() {
            return Ok(());
        }
        return Err(replay_mismatch(
            "incomplete EVM transaction produced a terminal output",
        ));
    }
    if output_frames.len() != 1 {
        return Err(replay_mismatch(
            "terminal EVM transaction did not produce exactly one output",
        ));
    }
    let prepared = prepared.ok_or_else(|| replay_missing("prepared invocation"))?;
    let submission_request = frame
        .submission_request()
        .ok_or_else(|| replay_missing("submission"))?;
    let receipt_request = frame
        .receipt_request()
        .ok_or_else(|| replay_missing("receipt"))?;
    let submission_evidence = broker.side_effect_submission(&submission_request)?;
    let receipt_evidence = broker.side_effect_receipt(&receipt_request)?;
    let submission = decode_replay::<EvmTransactionSubmission>(
        "submission",
        &submission_evidence.artifact_bytes,
    )?;
    let receipt =
        decode_replay::<EvmTransactionReceipt>("receipt", &receipt_evidence.artifact_bytes)?;
    let confirmation = match frame.confirmation_request() {
        Some(request) => Some(decode_replay::<EvmTransactionConfirmation>(
            "confirmation",
            &broker.side_effect_confirmation(&request)?.artifact_bytes,
        )?),
        None => None,
    };
    let expected = match confirmation.as_ref() {
        Some(confirmation) => state.output_from_confirmation(
            input,
            prepared,
            &submission,
            &receipt,
            confirmation,
            context,
        ),
        None => state.output_from_receipt(input, prepared, &submission, &receipt, context),
    }
    .map_err(replay_state_error)?;
    let output_frame = &output_frames[0];
    ensure_replay_schema::<EvmTransactionOutcome>(&output_frame.cell.schema_id)?;
    let expected_bytes = replay::canonical_value_bytes(&expected)?;
    if output_frame.artifact_bytes != expected_bytes.as_bytes() {
        return Err(replay_mismatch(
            "terminal EVM transaction output differed from the state reducer",
        ));
    }
    Ok(())
}

fn replay_transaction_mismatch_code() -> replay::Result<events::AmbiguityCode> {
    events::AmbiguityCode::new("mfm.evm.transaction_mismatch").map_err(replay_identity_error)
}

fn replay_state_error(error: impl std::fmt::Display) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::SideEffectMismatch,
        error.to_string(),
    )
}

fn replay_identity_error(error: impl std::fmt::Display) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::SideEffectMismatch,
        error.to_string(),
    )
}

fn replay_value_error(error: impl std::fmt::Display) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::SideEffectMismatch,
        error.to_string(),
    )
}

fn replay_mismatch(message: &'static str) -> replay::ReplayError {
    replay::ReplayError::new(replay::ReplayErrorKind::SideEffectMismatch, message)
}

fn replay_missing(label: &'static str) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::SideEffectMissing,
        format!("missing EVM transaction {label} evidence"),
    )
}

#[cfg(test)]
#[path = "transaction_tests.rs"]
mod tests;
