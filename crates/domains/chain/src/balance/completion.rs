use std::marker::PhantomData;

use mfm_ids::StableId;
use mfm_program::{ProposedStateOutcome, PureState, State};
use mfm_program_derive::MfmValue;
use mfm_values::{InvocationDiagnostic, MfmValue as Value};
use serde::{Deserialize, Serialize};

use super::{BalanceCollectionFailure, BalanceContext};

/// Complete confirmed collection and its checked aggregate, retaining the typed caller once.
#[derive(Debug, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct BalanceCollectionCompletion<K: Value> {
    context: BalanceContext<K>,
    total_scaled: String,
}
impl<K: Value> BalanceCollectionCompletion<K> {
    /// Complete request, caller, metadata and confirmed sources.
    pub fn context(&self) -> &BalanceContext<K> {
        &self.context
    }
    /// Canonical aggregate at the requested collection scale.
    pub fn total_scaled(&self) -> &str {
        &self.total_scaled
    }
    /// Moves the confirmed context and aggregate into the caller's handoff.
    pub fn into_parts(self) -> (BalanceContext<K>, String) {
        (self.context, self.total_scaled)
    }
}
fn total<K: Value>(context: &BalanceContext<K>) -> Result<String, BalanceCollectionFailure> {
    context.request().total_scaled(
        context
            .completed()
            .iter()
            .map(|entry| (entry.raw_units(), entry.source_decimals())),
    )
}
impl<'de, K: Value> Deserialize<'de> for BalanceCollectionCompletion<K> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(
            deny_unknown_fields,
            bound(deserialize = "K: serde::de::DeserializeOwned")
        )]
        struct Wire<K: Value> {
            context: BalanceContext<K>,
            total_scaled: String,
        }
        let wire = Wire::<K>::deserialize(deserializer)?;
        wire.context
            .require_complete()
            .map_err(serde::de::Error::custom)?;
        let expected = total(&wire.context).map_err(serde::de::Error::custom)?;
        if expected != wire.total_scaled {
            return Err(serde::de::Error::custom(
                "balance collection total differs from confirmed amounts",
            ));
        }
        Ok(Self {
            context: wire.context,
            total_scaled: wire.total_scaled,
        })
    }
}

/// Consolidates only after every source has passed native confirmation and individual admission.
pub struct ConsolidateBalanceCollection<K>(PhantomData<fn() -> K>);
impl<K: Value> State for ConsolidateBalanceCollection<K> {
    type Input = BalanceContext<K>;
    type Output = BalanceCollectionCompletion<K>;
    type Failure = BalanceCollectionFailure;
    fn description() -> &'static str {
        "Checks complete collection coverage and consolidates confirmed balances."
    }
    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("mfm.chain.consolidate-balances@1")?)
    }
}
impl<K: Value> PureState for ConsolidateBalanceCollection<K> {
    fn evaluate(
        input: BalanceContext<K>,
    ) -> Result<ProposedStateOutcome<Self::Output, Self::Failure>, InvocationDiagnostic> {
        input.require_complete().map_err(|source| {
            InvocationDiagnostic::from_fields(
                "state_internal",
                "consolidate_balances",
                &source,
                None,
            )
        })?;
        Ok(match total(&input) {
            Ok(total_scaled) => ProposedStateOutcome::Success {
                output: BalanceCollectionCompletion {
                    context: input,
                    total_scaled,
                },
            },
            Err(failure) => ProposedStateOutcome::Failure { failure },
        })
    }
}
