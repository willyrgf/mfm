//! Deterministic EVM read and transaction graph authoring.

use mfm_ids::{ContentRef, StableId};
use mfm_journal::v1::ValueRef;
use mfm_program::{AuthoredHandle, AuthoredProgramBuilder, Operation, StateBindings};
use mfm_spec::{
    AuthoredSourceSelector, CanonicalAuthoredProgram, CanonicalJsonValue, EntryPointContract,
    EntryPointId, PlanningProfile,
};
use mfm_values::{MfmValue as _, PublicOutputDescriptor};

use crate::state::{
    AggregateEvmBalancesState, BootstrapEvmSourceState, ConfirmEvmAnchorState, EvmBalanceAsset,
    EvmBalanceCollection, EvmBalanceCollectionConfig, EvmBalanceSource, ReadEvmInitialAnchorState,
    ReadEvmNativeBalanceState, ReadEvmTokenBalanceState, ReadEvmTokenDecimalsState,
};
use crate::{
    EvmNetworkBinding, EvmSubmitTransactionPublicOutputs, EvmSubmitTransactionRequest,
    EvmSubmitTransactionSelector, EvmTransactionOutcome, SubmitEvmTransactionState,
    EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID, EVM_SUBMIT_TRANSACTION_OPERATION_ID,
};

/// Stable internal operation identity for a reusable EVM collection graph.
pub const EVM_BALANCE_COLLECTION_OPERATION_ID: &str = "mfm.evm/balance-collection";

/// Validator-owned field selections consumed by one static collection graph.
pub struct EvmBalanceCollectionAuthoringInputs {
    unit_config_ref: ValueRef,
    binding: AuthoredHandle<EvmNetworkBinding>,
    native_decimals: AuthoredHandle<u8>,
    sources: Vec<AuthoredHandle<EvmBalanceSource>>,
    token_contracts: Vec<AuthoredHandle<String>>,
}

impl EvmBalanceCollectionAuthoringInputs {
    /// Binds one graph to the retained framework unit config and validator output fields.
    pub fn new(
        unit_config_ref: ValueRef,
        binding: AuthoredHandle<EvmNetworkBinding>,
        native_decimals: AuthoredHandle<u8>,
        sources: Vec<AuthoredHandle<EvmBalanceSource>>,
        token_contracts: Vec<AuthoredHandle<String>>,
    ) -> Self {
        Self {
            unit_config_ref,
            binding,
            native_decimals,
            sources,
            token_contracts,
        }
    }
}

/// Typed output handle for one EVM balance collection graph.
pub struct EvmBalanceCollectionOutputs {
    /// Pure aggregate output consumed directly by the parent operation.
    pub collection: AuthoredHandle<EvmBalanceCollection>,
}

/// Reusable certified collection operation.
///
/// `topology` is inspected only while the package author deterministically
/// enumerates a bounded graph. Every runtime value still comes from the
/// successful validator output selected by `inputs`.
pub struct EvmBalanceCollectionOperation {
    topology: EvmBalanceCollectionConfig,
    inputs: EvmBalanceCollectionAuthoringInputs,
}

impl EvmBalanceCollectionOperation {
    /// Creates one exact graph and rejects a topology/selector cardinality mismatch.
    pub fn new(
        topology: EvmBalanceCollectionConfig,
        inputs: EvmBalanceCollectionAuthoringInputs,
    ) -> mfm_program::Result<Self> {
        if topology.sources().len() != inputs.sources.len()
            || topology.token_contracts().len() != inputs.token_contracts.len()
        {
            return Err(mfm_program::ProgramError::Authoring(
                "EVM collection topology differs from validator field selections".to_owned(),
            ));
        }
        Ok(Self { topology, inputs })
    }

    /// Returns the immutable topology inspected during package authoring.
    pub const fn topology(&self) -> &EvmBalanceCollectionConfig {
        &self.topology
    }

    fn bindings(&self) -> StateBindings {
        StateBindings::new(self.inputs.unit_config_ref.clone(), None)
    }
}

impl Operation for EvmBalanceCollectionOperation {
    type Output = EvmBalanceCollectionOutputs;

    fn author(&self, builder: &mut AuthoredProgramBuilder) -> mfm_program::Result<Self::Output> {
        let bootstrap = builder
            .state::<BootstrapEvmSourceState>(stable_id("bootstrap_source")?, self.bindings())?;
        builder.connect_to(&self.inputs.binding, &bootstrap, 0)?;

        let initial = builder
            .state::<ReadEvmInitialAnchorState>(stable_id("initial_anchor")?, self.bindings())?;
        builder.connect_to(bootstrap.output(), &initial, 0)?;

        let mut fanout = Vec::with_capacity(
            self.topology.sources().len() + self.topology.token_contracts().len(),
        );
        for (index, contract) in self.inputs.token_contracts.iter().enumerate() {
            let state = builder.state::<ReadEvmTokenDecimalsState>(
                indexed_key("token_decimals", index)?,
                self.bindings(),
            )?;
            builder.connect_to(initial.output(), &state, 0)?;
            builder.connect_to(contract, &state, 1)?;
            fanout.push(state.into_output());
        }
        for (index, (source, source_handle)) in self
            .topology
            .sources()
            .iter()
            .zip(&self.inputs.sources)
            .enumerate()
        {
            match source.asset() {
                EvmBalanceAsset::Native => {
                    let state = builder.state::<ReadEvmNativeBalanceState>(
                        indexed_key("native_balance", index)?,
                        self.bindings(),
                    )?;
                    builder.connect_to(initial.output(), &state, 0)?;
                    builder.connect_to(source_handle, &state, 1)?;
                    fanout.push(state.into_output());
                }
                EvmBalanceAsset::Erc20 { .. } => {
                    let state = builder.state::<ReadEvmTokenBalanceState>(
                        indexed_key("token_balance", index)?,
                        self.bindings(),
                    )?;
                    builder.connect_to(initial.output(), &state, 0)?;
                    builder.connect_to(source_handle, &state, 1)?;
                    fanout.push(state.into_output());
                }
            }
        }

        let confirmation = builder
            .state::<ConfirmEvmAnchorState>(stable_id("confirm_anchor")?, self.bindings())?;
        for result in &fanout {
            builder.connect_to(result, &confirmation, 0)?;
        }

        let aggregate = builder.state::<AggregateEvmBalancesState>(
            stable_id("aggregate_balances")?,
            self.bindings(),
        )?;
        builder.connect_to(&self.inputs.binding, &aggregate, 0)?;
        builder.connect_to(&self.inputs.native_decimals, &aggregate, 1)?;
        for result in &fanout {
            builder.connect_to(result, &aggregate, 2)?;
        }
        builder.connect_to(confirmation.output(), &aggregate, 2)?;
        for source in &self.inputs.sources {
            builder.connect_to(source, &aggregate, 3)?;
        }
        for contract in &self.inputs.token_contracts {
            builder.connect_to(contract, &aggregate, 4)?;
        }
        builder.required_success(aggregate.output())?;

        Ok(EvmBalanceCollectionOutputs {
            collection: aggregate.into_output(),
        })
    }
}

/// Validator-selected inputs consumed by the one-state wallet effect graph.
pub struct EvmSubmitTransactionAuthoringInputs {
    unit_config_ref: ValueRef,
    request: AuthoredHandle<EvmSubmitTransactionRequest>,
    selector: AuthoredHandle<EvmSubmitTransactionSelector>,
}

impl EvmSubmitTransactionAuthoringInputs {
    /// Binds an immutable configured request to the public value-only selector.
    pub fn new(
        unit_config_ref: ValueRef,
        request: AuthoredHandle<EvmSubmitTransactionRequest>,
        selector: AuthoredHandle<EvmSubmitTransactionSelector>,
    ) -> Self {
        Self {
            unit_config_ref,
            request,
            selector,
        }
    }
}

/// Typed output handle for one submitted transaction.
pub struct EvmSubmitTransactionOutputs {
    /// Finalized success or revert.
    pub outcome: AuthoredHandle<EvmTransactionOutcome>,
}

/// Reusable one-state EVM transaction submission operation.
pub struct EvmSubmitTransactionOperation {
    inputs: EvmSubmitTransactionAuthoringInputs,
}

impl EvmSubmitTransactionOperation {
    /// Creates one exact transaction submission graph.
    pub const fn new(inputs: EvmSubmitTransactionAuthoringInputs) -> Self {
        Self { inputs }
    }
}

impl Operation for EvmSubmitTransactionOperation {
    type Output = EvmSubmitTransactionOutputs;

    fn author(&self, builder: &mut AuthoredProgramBuilder) -> mfm_program::Result<Self::Output> {
        let state = builder.state::<SubmitEvmTransactionState>(
            stable_id(EVM_SUBMIT_TRANSACTION_OPERATION_ID)?,
            StateBindings::new(self.inputs.unit_config_ref.clone(), None),
        )?;
        builder.connect_to(&self.inputs.request, &state, 0)?;
        builder.connect_to(&self.inputs.selector, &state, 1)?;
        builder.required_success(state.output())?;
        Ok(EvmSubmitTransactionOutputs {
            outcome: state.into_output(),
        })
    }
}

pub(crate) fn evm_submit_transaction_authored_program(
    unit_config_ref: ValueRef,
) -> mfm_program::Result<CanonicalAuthoredProgram> {
    let mut builder = AuthoredProgramBuilder::new(stable_id(EVM_SUBMIT_TRANSACTION_OPERATION_ID)?);
    let state = builder.state::<SubmitEvmTransactionState>(
        stable_id(EVM_SUBMIT_TRANSACTION_OPERATION_ID)?,
        StateBindings::new(unit_config_ref, None),
    )?;
    builder.bind_input_source(
        &state,
        0,
        AuthoredSourceSelector::Config {
            source_field_path: None,
        },
    )?;
    builder.bind_input_source(
        &state,
        1,
        AuthoredSourceSelector::RunAdmission {
            source_field_path: None,
        },
    )?;
    builder.required_success(state.output())?;
    builder.public_output(stable_id("outcome")?, state.output())?;
    builder.finish()
}

/// Builds the exact empty-framework-policy profile for wallet submission.
pub fn evm_submit_transaction_planning_profile(
    planner_contract_ref: ContentRef,
    planner_implementation_ref: ContentRef,
) -> mfm_spec::Result<PlanningProfile> {
    PlanningProfile::new(
        planner_contract_ref,
        planner_implementation_ref,
        Vec::new(),
        CanonicalJsonValue::new(serde_json::json!({}))?,
    )
}

/// Builds the qualified EVM transaction entry-point contract.
pub fn evm_submit_transaction_entry_point_contract(
    planner_contract_ref: ContentRef,
    planner_implementation_ref: ContentRef,
) -> mfm_spec::Result<EntryPointContract> {
    EntryPointContract::new(
        EntryPointId::new(EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID)
            .map_err(|error| mfm_spec::SpecError::Contract(error.to_string()))?,
        StableId::new(EVM_SUBMIT_TRANSACTION_OPERATION_ID)
            .map_err(|error| mfm_spec::SpecError::Contract(error.to_string()))?,
        evm_submit_transaction_planning_profile(planner_contract_ref, planner_implementation_ref)?,
        EvmSubmitTransactionSelector::schema_id()
            .map_err(|error| mfm_spec::SpecError::Contract(error.to_string()))?,
        EvmSubmitTransactionPublicOutputs::public_schema_id()
            .map_err(|error| mfm_spec::SpecError::Contract(error.to_string()))?,
    )
}

fn indexed_key(prefix: &str, index: usize) -> mfm_program::Result<StableId> {
    stable_id(format!("{prefix}/{index:04}"))
}

fn stable_id(value: impl AsRef<str>) -> mfm_program::Result<StableId> {
    StableId::new(value).map_err(|error| mfm_program::ProgramError::Authoring(error.to_string()))
}
