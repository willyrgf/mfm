//! Native balance qualification; shared collection progression belongs to Chain.

use std::num::NonZeroU64;

use mfm_chain::balance::{BalanceContext, DecimalScale};
use mfm_program_derive::MfmValue;
use mfm_values::{InvocationDiagnostic, MfmValue as Value, Object};
use serde::{Deserialize, Serialize};

use crate::{EvmAddress, EvmPhysicalTarget};
mod route;
pub use route::EvmBalanceRoute;
mod native;
mod stages;
pub use stages::{
    project_balance_context, CheckEvmBalanceChain, ConfirmEvmBalanceAnchor, EvmBalanceFailure,
    EvmChainChecked, EvmNativeBalance, EvmObservationRejection, EvmTokenAnchored, EvmTokenBalance,
    NativeBalanceStage, ReadEvmTokenDecimals, ReadInitialEvmBalanceAnchor,
};

/// Chain-ID-qualified balance ledger. The existing balance protocol does not observe genesis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "balance-ledger",
    version = "1",
    schema = "mfm.evm-balance-ledger"
)]
pub struct EvmBalanceLedger {
    chain_id: NonZeroU64,
}
impl EvmBalanceLedger {
    /// Retains the checked chain ID without implying a genesis-hash guarantee.
    pub fn new(chain_id: NonZeroU64) -> Self {
        Self { chain_id }
    }
    /// Nonzero chain ID checked by the native chain observation.
    pub fn chain_id(&self) -> NonZeroU64 {
        self.chain_id
    }
}

/// Native account and asset, independent of shared source identity and public endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "balance-target",
    version = "1",
    schema = "mfm.evm-balance-target"
)]
pub struct EvmBalanceTarget {
    account: EvmAddress,
    token: Option<EvmAddress>,
}
impl EvmBalanceTarget {
    /// Selects native currency with None or the exact token contract with Some.
    pub fn new(account: EvmAddress, token: Option<EvmAddress>) -> Self {
        Self { account, token }
    }
    /// Exact observed account.
    pub fn account(&self) -> &EvmAddress {
        &self.account
    }
    /// Token contract, absent for native currency.
    pub fn token(&self) -> Option<&EvmAddress> {
        self.token.as_ref()
    }
}

/// Per-source native planning facts retained in Program for configuration-free admission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "balance-binding",
    version = "1",
    schema = "mfm.evm-balance-binding"
)]
pub struct EvmBalanceBinding {
    route: EvmPhysicalTarget,
    source_ordinal: u32,
    target: EvmBalanceTarget,
    collection_scale: DecimalScale,
}
impl EvmBalanceBinding {
    /// Retains selected facts; native use must check them against its actual execution context.
    pub fn new(
        route: EvmPhysicalTarget,
        source_ordinal: u32,
        target: EvmBalanceTarget,
        collection_scale: DecimalScale,
    ) -> Self {
        Self {
            route,
            source_ordinal,
            target,
            collection_scale,
        }
    }
    /// Full public route, independent of provider handles or configuration repositories.
    pub fn route(&self) -> &EvmPhysicalTarget {
        &self.route
    }
    /// Planned position within the collection's declaration order.
    pub fn source_ordinal(&self) -> u32 {
        self.source_ordinal
    }
    /// Exact planned account and asset.
    pub fn target(&self) -> &EvmBalanceTarget {
        &self.target
    }
    /// Planned collection scale used by native-currency preparation.
    pub fn collection_scale(&self) -> DecimalScale {
        self.collection_scale
    }

    fn check_intent(
        &self,
        intent: &crate::EvmReadIntent,
    ) -> Result<(), mfm_capabilities::CallbackFailure> {
        if self.source_ordinal != intent.source_ordinal() {
            return Err(mismatch(BindingMismatch::Ordinal).into());
        }
        if intent.chain_id() != self.route.chain_id {
            return Err(mismatch(BindingMismatch::Ledger).into());
        }
        if intent.target() != &self.target {
            return Err(mismatch(BindingMismatch::Target).into());
        }
        let route = mfm_capabilities::codec::encode(|| {
            Object::from_value(&self.route)
                .map_err(|source| source.into_diagnostic("balance_route_identity"))
        })?;
        if route.value_ref() != intent.route_ref() {
            return Err(mismatch(BindingMismatch::Route).into());
        }
        if intent.collection_scale() != self.collection_scale {
            return Err(mismatch(BindingMismatch::Scale).into());
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
enum BindingMismatch {
    #[error("balance binding source ordinal mismatch")]
    Ordinal,
    #[error("balance binding ledger mismatch")]
    Ledger,
    #[error("balance binding target mismatch")]
    Target,
    #[error("balance binding route mismatch")]
    Route,
    #[error("balance binding scale mismatch")]
    Scale,
}
fn mismatch(source: BindingMismatch) -> InvocationDiagnostic {
    InvocationDiagnostic::from_fields("state_internal", "qualify_balance_binding", &source, None)
}

impl crate::EvmReadIntent {
    /// Derives native qualification from retained execution input, independently of selected binding.
    pub fn from_context<K: Value>(
        context: &BalanceContext<K>,
        subject: crate::EvmReadSubject,
    ) -> Result<Self, InvocationDiagnostic> {
        let source = context.active_source().map_err(|source| {
            InvocationDiagnostic::from_fields(
                "state_internal",
                "prepare_native_balance",
                &source,
                None,
            )
        })?;
        let ledger = source
            .target()
            .ledger()
            .native()
            .decode::<EvmBalanceLedger>()?;
        let target = source.target().native().decode::<EvmBalanceTarget>()?;
        Self::new(
            context.completed().len() as u32,
            context.request().decimals(),
            ledger.chain_id(),
            target,
            context.metadata().route_ref().clone(),
            subject,
        )
        .map_err(|source| {
            InvocationDiagnostic::from_fields(
                "state_internal",
                "prepare_native_balance",
                &source,
                None,
            )
        })
    }
}

/// Identity native translation for the three supporting EVM observation protocols.
pub struct EvmBalanceObservation;
macro_rules! supporting_implementation {
    ($capability:ty, $family:ident, $id:literal) => {
        impl mfm_capabilities::ReadImplementation<$capability> for EvmBalanceObservation {
            type Binding = EvmBalanceBinding;
            type NativeIntent = crate::EvmReadIntent;
            type NativeEvidence = crate::EvmReadEvidence;
            type OperationalError = crate::EvmOperationalError;
            fn implementation_id() -> mfm_capabilities::Result<mfm_ids::StableId> {
                Ok(mfm_ids::StableId::new($id)?)
            }
            fn encode_intent(
                _: &mfm_ids::ContentRef,
                _: &mfm_ids::ContentRef,
                binding: &Self::Binding,
                intent: &crate::EvmReadIntent,
            ) -> Result<Self::NativeIntent, mfm_capabilities::CallbackFailure> {
                binding.check_intent(intent)?;
                if !crate::ReadCapabilityFamily::$family.accepts(intent.subject()) {
                    return Err(InvocationDiagnostic::from_fields(
                        "state_internal",
                        "balance_observation_family",
                        &mfm_capabilities::CapabilityError::EvidenceBinding,
                        None,
                    )
                    .into());
                }
                Ok(intent.clone())
            }
            fn project_evidence(
                _: &mfm_ids::ContentRef,
                _: &mfm_ids::ContentRef,
                binding: &Self::Binding,
                intent_ref: &mfm_ids::ContentRef,
                intent: &crate::EvmReadIntent,
                native_intent_ref: &mfm_ids::ContentRef,
                native_intent: &Self::NativeIntent,
                evidence: &Self::NativeEvidence,
                original: &Object,
            ) -> Result<crate::EvmReadEvidence, mfm_capabilities::CallbackFailure> {
                binding.check_intent(intent)?;
                if intent != native_intent || intent_ref != native_intent_ref {
                    return Err(InvocationDiagnostic::from_fields(
                        "state_internal",
                        "balance_observation_identity",
                        &mfm_capabilities::CapabilityError::EvidenceBinding,
                        None,
                    )
                    .into());
                }
                <$capability as mfm_capabilities::ReadCapabilityContract>::bind_evidence(
                    intent_ref,
                    intent,
                    original.value_ref(),
                    evidence,
                )?;
                Ok(evidence.clone())
            }
        }
        impl<S> mfm_program::InjectRead<S, $capability> for EvmBalanceObservation
        where
            S: mfm_program::ReadSelection<
                $capability,
                ExpandedInput = <S as mfm_program::State>::Input,
                ExpandedOutput = <S as mfm_program::State>::Output,
            >,
        {
            type Prefix = mfm_program::Identity<S::Input>;
            type Suffix = mfm_program::Identity<S::Output>;
            fn surround(
                _: &EvmBalanceBinding,
            ) -> mfm_program::Result<(Self::Prefix, Self::Suffix)> {
                Ok((Self::Prefix::default(), Self::Suffix::default()))
            }
        }
    };
}
supporting_implementation!(
    crate::EvmChainIdentityRead,
    ChainIdentity,
    "mfm.evm.balance-chain-observation@1"
);
supporting_implementation!(
    crate::EvmAnchorRead,
    Anchor,
    "mfm.evm.balance-anchor-observation@1"
);
supporting_implementation!(
    crate::EvmBalanceRead,
    Balance,
    "mfm.evm.balance-asset-observation@1"
);
