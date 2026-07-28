//! Deterministic reusable EVM balance collection topology.
//!
//! [`EvmBalanceCollectionOperation`] expands to one fact-producing external read and exports its
//! resulting receipt. Parent operations compose it directly.
//!
//! # Examples
//!
//! ```rust
//! use mfm_evm::EvmBalanceCollectionOperation;
//! use mfm_program::Operation as _;
//!
//! assert_eq!(EvmBalanceCollectionOperation::name(), "mfm.evm.balance_collection");
//! ```

use crate::state::{
    CollectEvmBalancesState, EvmBalanceCollectionConfig, EvmBalanceCollectionReceipt,
};
use mfm_ids::{DigestAlgorithm, OperationKind, OperationVersion};
use mfm_program::{NoContext, Operation, OperationExpansion, StateKey};
use mfm_program_derive::OperationOutput;

const OP_NAMESPACE: &str = "mfm.evm";
const OP_KIND_NAME: &str = "balance_collection";
const OP_VERSION: &str = "mfm.evm.operation.balance_collection.v1";

/// Output of one reusable EVM balance collection operation.
#[derive(OperationOutput)]
#[mfm(schema = "mfm.evm.operation_outputs.balance_collection")]
pub struct EvmBalanceCollectionOutputs<'program, 'scope> {
    /// Checked receipt returned by the fact-producing read state.
    pub receipt: mfm_program::Handle<'program, 'scope, EvmBalanceCollectionReceipt>,
}

/// Reusable one-state EVM balance collection operation.
pub struct EvmBalanceCollectionOperation;

impl Operation for EvmBalanceCollectionOperation {
    type Config = EvmBalanceCollectionConfig;
    type Input<'program, 'scope> = ();
    type Output<'program, 'scope> = EvmBalanceCollectionOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        OperationKind::new(
            OP_NAMESPACE,
            OP_KIND_NAME,
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.evm.operation:balance_collection"),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        OperationVersion::new(OP_VERSION)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.evm.balance_collection"
    }

    fn expand<'program, 'scope>(
        &self,
        config: mfm_program::ValidatedConfig<Self::Config>,
        _input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let receipt = builder.state::<CollectEvmBalancesState, _>(
            StateKey::new("collect_balances")?,
            NoContext,
            config.into_inner(),
            (),
        )?;
        Ok(EvmBalanceCollectionOutputs { receipt })
    }
}

mfm_certify::define_program_descriptor_registry! {
    state_registry: pub evm_collectors_state_registry,
    operation_registry: pub evm_collectors_operation_registry,
    certification: pub register_evm_collectors_certification_descriptors,
    authoring_catalog: pub evm_collectors_authoring_catalog,
    includes: [],
    states: [
        CollectEvmBalancesState,
    ],
    operations: [
        EvmBalanceCollectionOperation,
    ],
}

#[cfg(test)]
#[path = "operation_tests.rs"]
mod tests;
