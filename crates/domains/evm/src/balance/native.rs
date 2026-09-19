use mfm_capabilities::{codec, CallbackFailure, ReadCapabilityContract, ReadImplementation};
use mfm_chain::balance::{
    BalanceEvidence, BalanceOutcome, BalanceRead, BalanceSourceDefinition, ObserveBalance,
    ReadBalanceAt,
};
use mfm_ids::{ContentRef, StableId};
use mfm_program::{InjectRead, ResolvedRead};
use mfm_values::{InvocationDiagnostic, MfmValue, Object, Unsigned256};

use super::*;
use crate::{
    EvmAnchorRead, EvmBalanceRead, EvmBlockAnchor, EvmBlockPoint, EvmChainIdentityRead,
    EvmReadEvidence, EvmReadIntent, EvmReadSubject, EvmReadValue,
};

fn encode(
    binding: &EvmBalanceBinding,
    intent: &ReadBalanceAt,
    token: bool,
) -> Result<EvmReadIntent, CallbackFailure> {
    let ledger = codec::decode(|| {
        intent
            .target()
            .ledger()
            .native()
            .decode::<EvmBalanceLedger>()
    })?;
    let target = codec::decode(|| intent.target().native().decode::<EvmBalanceTarget>())?;
    if target.token().is_some() != token || target != binding.target {
        return Err(mismatch(BindingMismatch::Target).into());
    }
    if ledger.chain_id() != binding.route.chain_id {
        return Err(mismatch(BindingMismatch::Ledger).into());
    }
    let point = codec::decode(|| intent.observed_at().native().decode::<EvmBlockPoint>())?;
    let anchor = EvmBlockAnchor {
        number: point.number().clone(),
        hash: point.hash().clone(),
    };
    let route = codec::encode(|| {
        Object::from_value(&binding.route)
            .map_err(|source| source.into_diagnostic("balance_route_identity"))
    })?;
    if route.value_ref() != intent.route_ref() {
        return Err(mismatch(BindingMismatch::Route).into());
    }
    Ok(EvmReadIntent::new(
        binding.source_ordinal,
        binding.collection_scale,
        ledger.chain_id(),
        target,
        route.value_ref().clone(),
        if token {
            EvmReadSubject::TokenBalance { anchor }
        } else {
            EvmReadSubject::NativeBalance { anchor }
        },
    )
    .map_err(|source| {
        InvocationDiagnostic::from_fields("state_internal", "encode_balance", &source, None)
    })?)
}

impl EvmBalanceBinding {
    fn from_source<K>(source: &BalanceSourceDefinition<K>) -> Result<Self, CallbackFailure> {
        let route = EvmBalanceRoute::from_execution(source.execution())?;
        let target = codec::decode(|| {
            source
                .source()
                .target()
                .native()
                .decode::<EvmBalanceTarget>()
        })?;
        let ledger = codec::decode(|| {
            source
                .source()
                .target()
                .ledger()
                .native()
                .decode::<EvmBalanceLedger>()
        })?;
        if ledger.chain_id() != route.chain_id() {
            return Err(mismatch(BindingMismatch::Ledger).into());
        }
        let physical = codec::encode(|| {
            route
                .physical_target()
                .map_err(|cause| cause.into_diagnostic("balance_route_identity"))
        })?;
        Ok(Self::new(
            physical,
            source.ordinal(),
            target,
            source.scale(),
        ))
    }
}

macro_rules! implementation {
    ($implementation:ty, $token:literal, $id:literal) => {
        impl<K> mfm_program::ResolveReadBinding<BalanceSourceDefinition<K>, BalanceRead>
            for $implementation
        {
            fn binding(
                config: &BalanceSourceDefinition<K>,
            ) -> mfm_program::Result<EvmBalanceBinding> {
                let binding = EvmBalanceBinding::from_source(config).map_err(|cause| {
                    mfm_program::ProgramError::Diagnostic(cause.into_diagnostic())
                })?;
                if binding.target().token().is_some() != $token {
                    return Err(mfm_program::ProgramError::Diagnostic(mismatch(
                        BindingMismatch::Target,
                    )));
                }
                Ok(binding)
            }
        }
        impl ReadImplementation<BalanceRead> for $implementation {
            type Binding = EvmBalanceBinding;
            type NativeIntent = EvmReadIntent;
            type NativeEvidence = EvmReadEvidence;
            type OperationalError = crate::EvmOperationalError;
            fn implementation_id() -> mfm_capabilities::Result<StableId> {
                Ok(StableId::new($id)?)
            }
            fn encode_intent(
                _: &ContentRef,
                _: &ContentRef,
                binding: &EvmBalanceBinding,
                intent: &ReadBalanceAt,
            ) -> Result<EvmReadIntent, mfm_capabilities::CallbackFailure> {
                encode(binding, intent, $token)
            }
            fn project_evidence(
                implementation_ref: &ContentRef,
                _: &ContentRef,
                binding: &EvmBalanceBinding,
                intent_ref: &ContentRef,
                intent: &ReadBalanceAt,
                native_intent_ref: &ContentRef,
                native_intent: &EvmReadIntent,
                evidence: &EvmReadEvidence,
                original: &Object,
            ) -> Result<BalanceEvidence, mfm_capabilities::CallbackFailure> {
                if &encode(binding, intent, $token)? != native_intent {
                    return Err(InvocationDiagnostic::from_fields(
                        "state_internal",
                        "project_balance_intent",
                        &mfm_capabilities::CapabilityError::EvidenceBinding,
                        None,
                    )
                    .into());
                }
                EvmBalanceRead::bind_evidence(
                    native_intent_ref,
                    native_intent,
                    original.value_ref(),
                    evidence,
                )?;
                let outcome = match evidence {
                    EvmReadEvidence::Returned {
                        value: EvmReadValue::RawUnits(raw),
                        ..
                    } => BalanceOutcome::Observed {
                        observed_at: intent.observed_at().clone(),
                        raw_units: Unsigned256::new(raw.as_str()).map_err(|source| {
                            InvocationDiagnostic::from_fields(
                                "state_internal",
                                "project_balance_amount",
                                &source,
                                None,
                            )
                        })?,
                    },
                    EvmReadEvidence::Returned { .. } => {
                        return Err(InvocationDiagnostic::from_fields(
                            "state_internal",
                            "project_balance_amount",
                            &mfm_capabilities::CapabilityError::EvidenceBinding,
                            None,
                        )
                        .into());
                    }
                    EvmReadEvidence::Rejected { .. } => BalanceOutcome::Rejected,
                    EvmReadEvidence::SafeFailure { .. } => BalanceOutcome::SafeFailure,
                    EvmReadEvidence::IntegrityBlocked { .. } => BalanceOutcome::IntegrityBlocked,
                };
                Ok(BalanceEvidence::new(
                    intent_ref.clone(),
                    implementation_ref.clone(),
                    original.clone(),
                    outcome,
                ))
            }
        }
    };
}
implementation!(EvmNativeBalance, false, "mfm.evm.native-balance@1");
implementation!(EvmTokenBalance, true, "mfm.evm.token-balance@1");

impl<K: MfmValue> InjectRead<ObserveBalance<K>, BalanceRead> for EvmNativeBalance {
    type Prefix = (
        ResolvedRead<CheckEvmBalanceChain<K>, EvmChainIdentityRead, EvmBalanceObservation>,
        ResolvedRead<ReadInitialEvmBalanceAnchor<K, Self>, EvmAnchorRead, EvmBalanceObservation>,
    );
    type Suffix = ResolvedRead<ConfirmEvmBalanceAnchor<K>, EvmAnchorRead, EvmBalanceObservation>;
    fn surround(binding: &EvmBalanceBinding) -> mfm_program::Result<(Self::Prefix, Self::Suffix)> {
        Ok((
            (
                ResolvedRead::new(binding.clone()),
                ResolvedRead::new(binding.clone()),
            ),
            ResolvedRead::new(binding.clone()),
        ))
    }
}

impl<K: MfmValue> InjectRead<ObserveBalance<K>, BalanceRead> for EvmTokenBalance {
    type Prefix = (
        ResolvedRead<CheckEvmBalanceChain<K>, EvmChainIdentityRead, EvmBalanceObservation>,
        ResolvedRead<ReadInitialEvmBalanceAnchor<K, Self>, EvmAnchorRead, EvmBalanceObservation>,
        ResolvedRead<ReadEvmTokenDecimals<K>, EvmBalanceRead, EvmBalanceObservation>,
    );
    type Suffix = ResolvedRead<ConfirmEvmBalanceAnchor<K>, EvmAnchorRead, EvmBalanceObservation>;
    fn surround(binding: &EvmBalanceBinding) -> mfm_program::Result<(Self::Prefix, Self::Suffix)> {
        Ok((
            (
                ResolvedRead::new(binding.clone()),
                ResolvedRead::new(binding.clone()),
                ResolvedRead::new(binding.clone()),
            ),
            ResolvedRead::new(binding.clone()),
        ))
    }
}
