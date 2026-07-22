#![warn(missing_docs)]
//! Deterministic one-state aggregate Bitcoin balance collection topology.

use mfm_ids::{DigestAlgorithm, OperationKind, OperationVersion};
use mfm_program::{NoContext, Operation, OperationExpansion, StateKey};
use mfm_program_derive::OperationOutput;
pub use mfm_states_btc::{
    BitcoinBalanceCollectionConfig, BitcoinBalanceCollectionReceipt, BitcoinBalanceSnapshotFact,
    CollectBitcoinBalancesState,
};

const OP_VERSION: &str = "mfm.bitcoin.operation.balance_collection.v1";

/// Output of one aggregate Bitcoin balance collection.
#[derive(OperationOutput)]
#[mfm(schema = "mfm.bitcoin.operation_outputs.balance_collection")]
pub struct BitcoinBalanceCollectionOutputs<'program, 'scope> {
    /// Minimal receipt returned by the fact-producing read state.
    pub receipt: mfm_program::Handle<'program, 'scope, BitcoinBalanceCollectionReceipt>,
}

/// Reusable aggregate Bitcoin balance collection operation.
pub struct BitcoinBalanceCollectionOperation;

impl Operation for BitcoinBalanceCollectionOperation {
    type Config = BitcoinBalanceCollectionConfig;
    type Input<'program, 'scope> = ();
    type Output<'program, 'scope> = BitcoinBalanceCollectionOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        OperationKind::new(
            "mfm.bitcoin",
            "balance_collection",
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.bitcoin.operation:balance_collection"),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        OperationVersion::new(OP_VERSION)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.bitcoin.balance_collection"
    }

    fn expand<'program, 'scope>(
        &self,
        config: mfm_program::ValidatedConfig<Self::Config>,
        _input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let receipt = builder.state::<CollectBitcoinBalancesState, _>(
            StateKey::new("collect_balances")?,
            NoContext,
            config.into_inner(),
            (),
        )?;
        Ok(BitcoinBalanceCollectionOutputs { receipt })
    }
}

mfm_certify::define_program_descriptor_registry! {
    state_registry: pub bitcoin_collectors_state_registry,
    operation_registry: pub bitcoin_collectors_operation_registry,
    certification: pub register_bitcoin_collectors_certification_descriptors,
    includes: [],
    states: [
        CollectBitcoinBalancesState,
    ],
    operations: [
        BitcoinBalanceCollectionOperation,
    ],
}

#[cfg(test)]
mod tests;
