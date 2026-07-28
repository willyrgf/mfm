//! Package-owned callback-surface identities for production EVM graphs.

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_executor::RequiredPlanExpansion;
use mfm_ids::{ContentRef, DigestAlgorithm, FieldPath, SemanticTypeId, StableId};
use mfm_program::{
    boundary_content_ref, mfm_value_contract, state_input_value_contract,
    unit_config_value_contract, CanonicalCodec, ProgramError, QualifiedInputContract,
    QualifiedOutputProjector, QualifiedOutputSourceContract, QualifiedProgramRegistryBuilder,
    QualifiedSettlementCodecs, QualifiedSourceContract, QualifiedStateContract,
    QualifiedStateRegistration, Result, State, StateExecution,
};
use mfm_spec::{
    CertifiedFactSlot, CertifiedInputDestination, CertifiedOutputSlot, CertifiedSettlementContract,
    CertifiedStateExecution, ComponentImplementationDescriptor, ComponentKind,
};
use mfm_values::{MfmValue, RetainedValueContract, StateInput};
use serde::Serialize;

use crate::wallet_state::submit_transaction_execution;
use crate::{
    state::{
        aggregate_execution, balance_fact_descriptor_canonical, balance_fact_descriptor_ref,
        bootstrap_execution, confirmation_execution, fact_evidence_contract_canonical,
        fact_evidence_contract_ref, initial_anchor_execution, native_balance_execution,
        state_contract_canonical, support_contract, token_balance_execution,
        token_decimals_execution, EvmBalanceSnapshotSubject,
    },
    AggregateEvmBalancesState, BootstrapEvmSourceState, ConfirmEvmAnchorState,
    EvmAnchorConfirmationInput, EvmAnchorConfirmationRequest, EvmAnchoredSource,
    EvmBalanceAggregationInput, EvmBalanceCollection, EvmBalanceGraphResult, EvmBalanceReadInput,
    EvmBalanceSnapshotFact, EvmBalanceSource, EvmBlockResponse, EvmBootstrapInput,
    EvmChainIdentityRequest, EvmChainIdentityResponse, EvmCheckedSource, EvmInitialAnchorInput,
    EvmLatestAnchorRequest, EvmNativeBalanceRequest, EvmNetworkBinding, EvmQuantityResponse,
    EvmReadFailure, EvmSafeFailure, EvmSubmitTransactionFailure, EvmSubmitTransactionInput,
    EvmSubmitTransactionRequest, EvmSubmitTransactionSelector, EvmTokenBalanceRequest,
    EvmTokenDecimalsInput, EvmTokenDecimalsRequest, EvmTokenDecimalsResponse,
    EvmTransactionOutcome, EvmWalletAttemptResult, EvmWalletTerminalEvidence,
    ReadEvmInitialAnchorState, ReadEvmNativeBalanceState, ReadEvmTokenBalanceState,
    ReadEvmTokenDecimalsState, SubmitEvmTransactionState, EVM_BALANCE_COLLECTION_SOURCE_LIMIT,
    EVM_CHAIN_ID_OPERATION_ID, EVM_CONFIRM_ANCHOR_OPERATION_ID, EVM_LATEST_ANCHOR_OPERATION_ID,
    EVM_NATIVE_BALANCE_OPERATION_ID, EVM_SUBMIT_TRANSACTION_OPERATION_ID,
    EVM_TOKEN_BALANCE_OPERATION_ID, EVM_TOKEN_DECIMALS_OPERATION_ID,
};

/// Exact version of every package-owned EVM state callback-surface descriptor.
pub const EVM_STATE_CALLBACK_SURFACE_VERSION: &str = "mfm.evm.state-callback-surface.v1";

/// One exact package-owned state callback surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmStateCallbackSurface {
    state_contract_ref: ContentRef,
    state_contract_canonical: PlainCanonicalJsonBytes,
    canonical: PlainCanonicalJsonBytes,
    content_ref: ContentRef,
}

impl EvmStateCallbackSurface {
    fn read<S: State>(state_name: &'static str, operation_id: &'static str) -> Result<Self> {
        Self::new::<S>(state_name, &["apply", "request"], Some(operation_id))
    }

    fn pure<S: State>(state_name: &'static str) -> Result<Self> {
        Self::new::<S>(state_name, &["apply"], None)
    }

    fn effect<S: State>(state_name: &'static str, operation_id: &'static str) -> Result<Self> {
        Self::new::<S>(state_name, &["request", "settle"], Some(operation_id))
    }

    fn new<S: State>(
        state_name: &'static str,
        callbacks: &[&'static str],
        external_operation_id: Option<&'static str>,
    ) -> Result<Self> {
        #[derive(Serialize)]
        struct Descriptor<'a> {
            callbacks: &'a [&'static str],
            external_operation_id: Option<&'static str>,
            state_contract_ref: &'a ContentRef,
            version: &'static str,
        }

        let state_contract_ref = S::state_contract_ref()?;
        let state_contract_canonical = state_contract_canonical(state_name)?;
        let described_state_contract_ref = boundary_content_ref(
            super::state::contract_schema_id("mfm.evm.state-contract")?,
            &state_contract_canonical,
        )?;
        if described_state_contract_ref != state_contract_ref {
            return Err(ProgramError::Registry(
                "EVM state semantic contract differs from its package state identity".to_owned(),
            ));
        }
        let descriptor = Descriptor {
            callbacks,
            external_operation_id,
            state_contract_ref: &state_contract_ref,
            version: EVM_STATE_CALLBACK_SURFACE_VERSION,
        };
        let encoded = serde_json::to_string(&descriptor)
            .map_err(|error| ProgramError::Codec(error.to_string()))?;
        let canonical = PlainCanonicalJsonBytes::from_json_str(&encoded)
            .map_err(|error| ProgramError::Codec(error.to_string()))?;
        let content_ref = boundary_content_ref(
            super::state::contract_schema_id("mfm.evm.state-callback-surface")?,
            &canonical,
        )?;
        Ok(Self {
            state_contract_ref,
            state_contract_canonical,
            canonical,
            content_ref,
        })
    }

    /// Returns the exact semantic state contract implemented by this surface.
    pub const fn state_contract_ref(&self) -> &ContentRef {
        &self.state_contract_ref
    }

    /// Returns exact canonical bytes for the semantic state contract.
    pub const fn state_contract_canonical(&self) -> &PlainCanonicalJsonBytes {
        &self.state_contract_canonical
    }

    /// Builds retained metadata for this exact semantic state-contract object.
    pub fn state_contract_support_contract(
        &self,
        role: StableId,
        evidence_contract_ref: ContentRef,
    ) -> Result<RetainedValueContract> {
        support_contract(
            "mfm.evm.state-contract",
            "state-contract",
            role,
            evidence_contract_ref,
        )
    }

    /// Returns exact canonical descriptor bytes.
    pub const fn canonical(&self) -> &PlainCanonicalJsonBytes {
        &self.canonical
    }

    /// Returns the exact callback-surface identity.
    pub const fn content_ref(&self) -> &ContentRef {
        &self.content_ref
    }

    /// Builds retained metadata for this exact callback-surface object.
    pub fn support_contract(
        &self,
        role: StableId,
        evidence_contract_ref: ContentRef,
    ) -> Result<RetainedValueContract> {
        support_contract(
            "mfm.evm.state-callback-surface",
            "state-callback-surface",
            role,
            evidence_contract_ref,
        )
    }
}

/// Closed named callback-surface inventory for the seven-state EVM read graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmBalanceCollectionCallbackSurfaces {
    bootstrap_source: EvmStateCallbackSurface,
    read_initial_anchor: EvmStateCallbackSurface,
    read_token_decimals: EvmStateCallbackSurface,
    read_native_balance: EvmStateCallbackSurface,
    read_token_balance: EvmStateCallbackSurface,
    confirm_anchor: EvmStateCallbackSurface,
    aggregate_balances: EvmStateCallbackSurface,
}

impl EvmBalanceCollectionCallbackSurfaces {
    /// Returns the source-bootstrap callback surface.
    pub const fn bootstrap_source(&self) -> &EvmStateCallbackSurface {
        &self.bootstrap_source
    }

    /// Returns the initial-anchor callback surface.
    pub const fn read_initial_anchor(&self) -> &EvmStateCallbackSurface {
        &self.read_initial_anchor
    }

    /// Returns the token-decimals callback surface.
    pub const fn read_token_decimals(&self) -> &EvmStateCallbackSurface {
        &self.read_token_decimals
    }

    /// Returns the native-balance callback surface.
    pub const fn read_native_balance(&self) -> &EvmStateCallbackSurface {
        &self.read_native_balance
    }

    /// Returns the token-balance callback surface.
    pub const fn read_token_balance(&self) -> &EvmStateCallbackSurface {
        &self.read_token_balance
    }

    /// Returns the final-anchor callback surface.
    pub const fn confirm_anchor(&self) -> &EvmStateCallbackSurface {
        &self.confirm_anchor
    }

    /// Returns the pure aggregation callback surface.
    pub const fn aggregate_balances(&self) -> &EvmStateCallbackSurface {
        &self.aggregate_balances
    }

    /// Returns the exact seven entries in graph lifecycle order.
    pub fn ordered(&self) -> [&EvmStateCallbackSurface; 7] {
        [
            &self.bootstrap_source,
            &self.read_initial_anchor,
            &self.read_token_decimals,
            &self.read_native_balance,
            &self.read_token_balance,
            &self.confirm_anchor,
            &self.aggregate_balances,
        ]
    }
}

/// Builds the exact named callback-surface inventory for EVM collection.
pub fn evm_balance_collection_callback_surfaces() -> Result<EvmBalanceCollectionCallbackSurfaces> {
    Ok(EvmBalanceCollectionCallbackSurfaces {
        bootstrap_source: EvmStateCallbackSurface::read::<BootstrapEvmSourceState>(
            "bootstrap_source",
            EVM_CHAIN_ID_OPERATION_ID,
        )?,
        read_initial_anchor: EvmStateCallbackSurface::read::<ReadEvmInitialAnchorState>(
            "read_initial_anchor",
            EVM_LATEST_ANCHOR_OPERATION_ID,
        )?,
        read_token_decimals: EvmStateCallbackSurface::read::<ReadEvmTokenDecimalsState>(
            "read_token_decimals",
            EVM_TOKEN_DECIMALS_OPERATION_ID,
        )?,
        read_native_balance: EvmStateCallbackSurface::read::<ReadEvmNativeBalanceState>(
            "read_native_balance",
            EVM_NATIVE_BALANCE_OPERATION_ID,
        )?,
        read_token_balance: EvmStateCallbackSurface::read::<ReadEvmTokenBalanceState>(
            "read_token_balance",
            EVM_TOKEN_BALANCE_OPERATION_ID,
        )?,
        confirm_anchor: EvmStateCallbackSurface::read::<ConfirmEvmAnchorState>(
            "confirm_anchor",
            EVM_CONFIRM_ANCHOR_OPERATION_ID,
        )?,
        aggregate_balances: EvmStateCallbackSurface::pure::<AggregateEvmBalancesState>(
            "aggregate_balances",
        )?,
    })
}

/// Shared retained-value contracts selected across portfolio and EVM graphs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmBalanceCollectionValueContracts {
    network_binding: RetainedValueContract,
    native_decimals: RetainedValueContract,
    balance_source: RetainedValueContract,
    token_contract: RetainedValueContract,
    balance_graph_result: RetainedValueContract,
    balance_collection: RetainedValueContract,
}

impl EvmBalanceCollectionValueContracts {
    /// Returns the exact network-binding contract.
    pub const fn network_binding(&self) -> &RetainedValueContract {
        &self.network_binding
    }

    /// Returns the exact native-decimals scalar contract.
    pub const fn native_decimals(&self) -> &RetainedValueContract {
        &self.native_decimals
    }

    /// Returns the exact balance-source contract.
    pub const fn balance_source(&self) -> &RetainedValueContract {
        &self.balance_source
    }

    /// Returns the exact canonical token-contract string contract.
    pub const fn token_contract(&self) -> &RetainedValueContract {
        &self.token_contract
    }

    /// Returns the exact fan-out result contract.
    pub const fn balance_graph_result(&self) -> &RetainedValueContract {
        &self.balance_graph_result
    }

    /// Returns the exact pure collection output contract.
    pub const fn balance_collection(&self) -> &RetainedValueContract {
        &self.balance_collection
    }
}

/// Builds shared EVM contracts used by validator projections and state inputs.
pub fn evm_balance_collection_value_contracts(
    object_evidence_contract_ref: ContentRef,
) -> Result<EvmBalanceCollectionValueContracts> {
    Ok(EvmBalanceCollectionValueContracts {
        network_binding: value_contract::<EvmNetworkBinding>(
            "mfm.evm.value.network-binding",
            object_evidence_contract_ref.clone(),
        )?,
        native_decimals: scalar_contract(
            "native-decimals",
            "mfm.evm.value.native-decimals",
            object_evidence_contract_ref.clone(),
        )?,
        balance_source: value_contract::<EvmBalanceSource>(
            "mfm.evm.value.balance-source",
            object_evidence_contract_ref.clone(),
        )?,
        token_contract: scalar_contract(
            "token-contract",
            "mfm.evm.value.token-contract",
            object_evidence_contract_ref.clone(),
        )?,
        balance_graph_result: value_contract::<EvmBalanceGraphResult>(
            "mfm.evm.value.balance-graph-result",
            object_evidence_contract_ref.clone(),
        )?,
        balance_collection: value_contract::<EvmBalanceCollection>(
            "mfm.evm.value.balance-collection",
            object_evidence_contract_ref,
        )?,
    })
}

/// Closed retained-value contract inventory for the six EVM protocol reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmReadValueContracts {
    chain_identity_request: RetainedValueContract,
    chain_identity_response: RetainedValueContract,
    latest_anchor_request: RetainedValueContract,
    block_response: RetainedValueContract,
    token_decimals_request: RetainedValueContract,
    token_decimals_response: RetainedValueContract,
    native_balance_request: RetainedValueContract,
    quantity_response: RetainedValueContract,
    token_balance_request: RetainedValueContract,
    anchor_confirmation_request: RetainedValueContract,
    safe_failure: RetainedValueContract,
}

impl EvmReadValueContracts {
    /// Returns the exact chain-identity request contract.
    pub const fn chain_identity_request(&self) -> &RetainedValueContract {
        &self.chain_identity_request
    }

    /// Returns the exact chain-identity returned-value contract.
    pub const fn chain_identity_response(&self) -> &RetainedValueContract {
        &self.chain_identity_response
    }

    /// Returns the exact latest-anchor request contract.
    pub const fn latest_anchor_request(&self) -> &RetainedValueContract {
        &self.latest_anchor_request
    }

    /// Returns the shared exact block returned-value contract.
    pub const fn block_response(&self) -> &RetainedValueContract {
        &self.block_response
    }

    /// Returns the exact token-decimals request contract.
    pub const fn token_decimals_request(&self) -> &RetainedValueContract {
        &self.token_decimals_request
    }

    /// Returns the exact token-decimals returned-value contract.
    pub const fn token_decimals_response(&self) -> &RetainedValueContract {
        &self.token_decimals_response
    }

    /// Returns the exact native-balance request contract.
    pub const fn native_balance_request(&self) -> &RetainedValueContract {
        &self.native_balance_request
    }

    /// Returns the shared exact balance-quantity returned-value contract.
    pub const fn quantity_response(&self) -> &RetainedValueContract {
        &self.quantity_response
    }

    /// Returns the exact token-balance request contract.
    pub const fn token_balance_request(&self) -> &RetainedValueContract {
        &self.token_balance_request
    }

    /// Returns the exact anchor-confirmation request contract.
    pub const fn anchor_confirmation_request(&self) -> &RetainedValueContract {
        &self.anchor_confirmation_request
    }

    /// Returns the exact redaction-safe read-failure contract.
    pub const fn safe_failure(&self) -> &RetainedValueContract {
        &self.safe_failure
    }
}

/// Builds the one domain-owned retained contract inventory used by states and live reads.
pub fn evm_read_value_contracts(
    object_evidence_contract_ref: ContentRef,
) -> Result<EvmReadValueContracts> {
    Ok(EvmReadValueContracts {
        chain_identity_request: value_contract::<EvmChainIdentityRequest>(
            "mfm.evm.read.request.chain-identity",
            object_evidence_contract_ref.clone(),
        )?,
        chain_identity_response: value_contract::<EvmChainIdentityResponse>(
            "mfm.evm.read.returned.chain-identity",
            object_evidence_contract_ref.clone(),
        )?,
        latest_anchor_request: value_contract::<EvmLatestAnchorRequest>(
            "mfm.evm.read.request.latest-anchor",
            object_evidence_contract_ref.clone(),
        )?,
        block_response: value_contract::<EvmBlockResponse>(
            "mfm.evm.read.returned.block",
            object_evidence_contract_ref.clone(),
        )?,
        token_decimals_request: value_contract::<EvmTokenDecimalsRequest>(
            "mfm.evm.read.request.token-decimals",
            object_evidence_contract_ref.clone(),
        )?,
        token_decimals_response: value_contract::<EvmTokenDecimalsResponse>(
            "mfm.evm.read.returned.token-decimals",
            object_evidence_contract_ref.clone(),
        )?,
        native_balance_request: value_contract::<EvmNativeBalanceRequest>(
            "mfm.evm.read.request.native-balance",
            object_evidence_contract_ref.clone(),
        )?,
        quantity_response: value_contract::<EvmQuantityResponse>(
            "mfm.evm.read.returned.quantity",
            object_evidence_contract_ref.clone(),
        )?,
        token_balance_request: value_contract::<EvmTokenBalanceRequest>(
            "mfm.evm.read.request.token-balance",
            object_evidence_contract_ref.clone(),
        )?,
        anchor_confirmation_request: value_contract::<EvmAnchorConfirmationRequest>(
            "mfm.evm.read.request.anchor-confirmation",
            object_evidence_contract_ref.clone(),
        )?,
        safe_failure: value_contract::<EvmSafeFailure>(
            "mfm.evm.failure.safe-read",
            object_evidence_contract_ref,
        )?,
    })
}

/// Exact package-owned support objects referenced by balance fact slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmBalanceFactSupportObjects {
    descriptor_canonical: PlainCanonicalJsonBytes,
    descriptor_ref: ContentRef,
    subject_evidence_canonical: PlainCanonicalJsonBytes,
    subject_evidence_ref: ContentRef,
    response_evidence_canonical: PlainCanonicalJsonBytes,
    response_evidence_ref: ContentRef,
}

impl EvmBalanceFactSupportObjects {
    /// Returns exact balance-fact descriptor bytes.
    pub const fn descriptor_canonical(&self) -> &PlainCanonicalJsonBytes {
        &self.descriptor_canonical
    }

    /// Returns the exact balance-fact descriptor identity.
    pub const fn descriptor_ref(&self) -> &ContentRef {
        &self.descriptor_ref
    }

    /// Returns exact subject object-evidence contract bytes.
    pub const fn subject_evidence_canonical(&self) -> &PlainCanonicalJsonBytes {
        &self.subject_evidence_canonical
    }

    /// Returns the exact subject object-evidence contract identity.
    pub const fn subject_evidence_ref(&self) -> &ContentRef {
        &self.subject_evidence_ref
    }

    /// Returns exact response object-evidence contract bytes.
    pub const fn response_evidence_canonical(&self) -> &PlainCanonicalJsonBytes {
        &self.response_evidence_canonical
    }

    /// Returns the exact response object-evidence contract identity.
    pub const fn response_evidence_ref(&self) -> &ContentRef {
        &self.response_evidence_ref
    }

    /// Builds retained metadata for the fact descriptor support object.
    pub fn descriptor_support_contract(
        &self,
        role: StableId,
        evidence_contract_ref: ContentRef,
    ) -> Result<RetainedValueContract> {
        support_contract(
            "mfm.fact-descriptor",
            "balance-fact-descriptor",
            role,
            evidence_contract_ref,
        )
    }

    /// Builds retained metadata for a package-owned fact evidence-contract object.
    pub fn evidence_support_contract(
        &self,
        role: StableId,
        evidence_contract_ref: ContentRef,
    ) -> Result<RetainedValueContract> {
        support_contract(
            "mfm.object-evidence-contract",
            "fact-object-evidence-contract",
            role,
            evidence_contract_ref,
        )
    }
}

/// Builds exact package support objects referenced by the repeated balance fact slot.
pub fn evm_balance_fact_support_objects() -> Result<EvmBalanceFactSupportObjects> {
    Ok(EvmBalanceFactSupportObjects {
        descriptor_canonical: balance_fact_descriptor_canonical()?,
        descriptor_ref: balance_fact_descriptor_ref()?,
        subject_evidence_canonical: fact_evidence_contract_canonical("subject")?,
        subject_evidence_ref: fact_evidence_contract_ref("subject")?,
        response_evidence_canonical: fact_evidence_contract_canonical("response")?,
        response_evidence_ref: fact_evidence_contract_ref("response")?,
    })
}

/// Exact named implementation descriptors for all seven EVM read-graph states.
pub struct EvmBalanceCollectionStateImplementations {
    /// Source-bootstrap state implementation.
    pub bootstrap_source: ComponentImplementationDescriptor,
    /// Initial-anchor state implementation.
    pub read_initial_anchor: ComponentImplementationDescriptor,
    /// Token-decimals state implementation.
    pub read_token_decimals: ComponentImplementationDescriptor,
    /// Native-balance state implementation.
    pub read_native_balance: ComponentImplementationDescriptor,
    /// Token-balance state implementation.
    pub read_token_balance: ComponentImplementationDescriptor,
    /// Final-anchor state implementation.
    pub confirm_anchor: ComponentImplementationDescriptor,
    /// Pure aggregate state implementation.
    pub aggregate_balances: ComponentImplementationDescriptor,
}

impl EvmBalanceCollectionStateImplementations {
    fn validate(&self, surfaces: &EvmBalanceCollectionCallbackSurfaces) -> Result<ContentRef> {
        let pairs = [
            (&self.bootstrap_source, surfaces.bootstrap_source()),
            (&self.read_initial_anchor, surfaces.read_initial_anchor()),
            (&self.read_token_decimals, surfaces.read_token_decimals()),
            (&self.read_native_balance, surfaces.read_native_balance()),
            (&self.read_token_balance, surfaces.read_token_balance()),
            (&self.confirm_anchor, surfaces.confirm_anchor()),
            (&self.aggregate_balances, surfaces.aggregate_balances()),
        ];
        let qualification_ref = pairs[0].0.qualification_ref().clone();
        if pairs.iter().any(|(implementation, surface)| {
            implementation.component_kind() != ComponentKind::State
                || implementation.semantic_contract_ref() != surface.state_contract_ref()
                || implementation.callback_surface_ref() != surface.content_ref()
                || implementation.qualification_ref() != &qualification_ref
        }) {
            return Err(ProgramError::Registry(
                "EVM state implementation inventory differs from its exact package callbacks"
                    .to_owned(),
            ));
        }
        Ok(qualification_ref)
    }
}

/// Product-owned identities required to qualify the seven EVM states.
pub struct EvmBalanceCollectionStateArtifacts {
    object_evidence_contract_ref: ContentRef,
    unit_config_contract: RetainedValueContract,
    read_binding_ref: ContentRef,
    implementations: EvmBalanceCollectionStateImplementations,
}

impl EvmBalanceCollectionStateArtifacts {
    /// Constructs and validates the complete named EVM state artifact set.
    pub fn new(
        object_evidence_contract_ref: ContentRef,
        unit_config_contract: RetainedValueContract,
        read_binding_ref: ContentRef,
        implementations: EvmBalanceCollectionStateImplementations,
    ) -> Result<Self> {
        let expected_unit_config = unit_config_value_contract(
            unit_config_contract.role().clone(),
            object_evidence_contract_ref.clone(),
        )?;
        if unit_config_contract != expected_unit_config {
            return Err(ProgramError::Registry(
                "EVM state artifacts require the exact retained framework UnitConfig contract"
                    .to_owned(),
            ));
        }
        let surfaces = evm_balance_collection_callback_surfaces()?;
        implementations.validate(&surfaces)?;
        Ok(Self {
            object_evidence_contract_ref,
            unit_config_contract,
            read_binding_ref,
            implementations,
        })
    }

    /// Returns the shared reviewed object-evidence contract.
    pub const fn object_evidence_contract_ref(&self) -> &ContentRef {
        &self.object_evidence_contract_ref
    }

    /// Returns the exact retained framework UnitConfig contract.
    pub const fn unit_config_contract(&self) -> &RetainedValueContract {
        &self.unit_config_contract
    }

    /// Returns the aggregate six-operation EVM read binding.
    pub const fn read_binding_ref(&self) -> &ContentRef {
        &self.read_binding_ref
    }

    /// Returns the exact named state implementation inventory.
    pub const fn implementations(&self) -> &EvmBalanceCollectionStateImplementations {
        &self.implementations
    }

    /// Returns the one shared app-owned qualification identity.
    pub fn qualification_ref(&self) -> Result<&ContentRef> {
        let surfaces = evm_balance_collection_callback_surfaces()?;
        self.implementations.validate(&surfaces)?;
        Ok(self.implementations.bootstrap_source.qualification_ref())
    }
}

/// Closed typed registrations for every state in one EVM collection graph.
pub struct QualifiedEvmBalanceCollectionStates {
    bootstrap_source: QualifiedStateRegistration<BootstrapEvmSourceState>,
    read_initial_anchor: QualifiedStateRegistration<ReadEvmInitialAnchorState>,
    read_token_decimals: QualifiedStateRegistration<ReadEvmTokenDecimalsState>,
    read_native_balance: QualifiedStateRegistration<ReadEvmNativeBalanceState>,
    read_token_balance: QualifiedStateRegistration<ReadEvmTokenBalanceState>,
    confirm_anchor: QualifiedStateRegistration<ConfirmEvmAnchorState>,
    aggregate_balances: QualifiedStateRegistration<AggregateEvmBalancesState>,
}

impl QualifiedEvmBalanceCollectionStates {
    /// Registers the exact seven-state inventory into the product registry.
    pub fn register_into(
        self,
        registry: &mut QualifiedProgramRegistryBuilder,
    ) -> Result<&mut QualifiedProgramRegistryBuilder> {
        registry
            .register_state(self.bootstrap_source)?
            .register_state(self.read_initial_anchor)?
            .register_state(self.read_token_decimals)?
            .register_state(self.read_native_balance)?
            .register_state(self.read_token_balance)?
            .register_state(self.confirm_anchor)?
            .register_state(self.aggregate_balances)
    }
}

/// Qualifies the exact seven EVM state callbacks and their retained contracts.
pub fn qualify_evm_balance_collection_states(
    artifacts: EvmBalanceCollectionStateArtifacts,
) -> Result<QualifiedEvmBalanceCollectionStates> {
    let surfaces = evm_balance_collection_callback_surfaces()?;
    artifacts.implementations.validate(&surfaces)?;
    let contracts = EvmStateValueContracts::new(artifacts.object_evidence_contract_ref.clone())?;
    let binding = artifacts.read_binding_ref;
    let unit = artifacts.unit_config_contract;
    let implementations = artifacts.implementations;

    let bootstrap_source = qualify_read_state(
        implementations.bootstrap_source,
        unit.clone(),
        contracts.bootstrap_input.clone(),
        vec![ordinary_input(
            "binding",
            contracts.shared.network_binding.clone(),
        )?],
        contracts.checked_source.clone(),
        EVM_CHAIN_ID_OPERATION_ID,
        binding.clone(),
        contracts.chain_identity_request.clone(),
        contracts.chain_identity_response.clone(),
        contracts.safe_failure.clone(),
        contracts.read_failure.clone(),
        bootstrap_execution(
            binding.clone(),
            contracts.chain_identity_request.clone(),
            contracts.chain_identity_response.clone(),
            contracts.safe_failure.clone(),
        )?,
    )?;
    let read_initial_anchor = qualify_read_state(
        implementations.read_initial_anchor,
        unit.clone(),
        contracts.initial_anchor_input.clone(),
        vec![ordinary_input(
            "checked_source",
            contracts.checked_source.clone(),
        )?],
        contracts.anchored_source.clone(),
        EVM_LATEST_ANCHOR_OPERATION_ID,
        binding.clone(),
        contracts.latest_anchor_request.clone(),
        contracts.block_response.clone(),
        contracts.safe_failure.clone(),
        contracts.read_failure.clone(),
        initial_anchor_execution(
            binding.clone(),
            contracts.latest_anchor_request.clone(),
            contracts.block_response.clone(),
            contracts.safe_failure.clone(),
        )?,
    )?;
    let read_token_decimals = qualify_read_state(
        implementations.read_token_decimals,
        unit.clone(),
        contracts.token_decimals_input.clone(),
        vec![
            ordinary_input("anchored_source", contracts.anchored_source.clone())?,
            ordinary_input("contract_address", contracts.shared.token_contract.clone())?,
        ],
        contracts.shared.balance_graph_result.clone(),
        EVM_TOKEN_DECIMALS_OPERATION_ID,
        binding.clone(),
        contracts.token_decimals_request.clone(),
        contracts.token_decimals_response.clone(),
        contracts.safe_failure.clone(),
        contracts.read_failure.clone(),
        token_decimals_execution(
            binding.clone(),
            contracts.token_decimals_request.clone(),
            contracts.token_decimals_response.clone(),
            contracts.safe_failure.clone(),
        )?,
    )?;
    let read_native_balance = qualify_read_state(
        implementations.read_native_balance,
        unit.clone(),
        contracts.balance_read_input.clone(),
        vec![
            ordinary_input("anchored_source", contracts.anchored_source.clone())?,
            ordinary_input("balance_source", contracts.shared.balance_source.clone())?,
        ],
        contracts.shared.balance_graph_result.clone(),
        EVM_NATIVE_BALANCE_OPERATION_ID,
        binding.clone(),
        contracts.native_balance_request.clone(),
        contracts.quantity_response.clone(),
        contracts.safe_failure.clone(),
        contracts.read_failure.clone(),
        native_balance_execution(
            binding.clone(),
            contracts.native_balance_request.clone(),
            contracts.quantity_response.clone(),
            contracts.safe_failure.clone(),
        )?,
    )?;
    let read_token_balance = qualify_read_state(
        implementations.read_token_balance,
        unit.clone(),
        contracts.balance_read_input.clone(),
        vec![
            ordinary_input("anchored_source", contracts.anchored_source.clone())?,
            ordinary_input("balance_source", contracts.shared.balance_source.clone())?,
        ],
        contracts.shared.balance_graph_result.clone(),
        EVM_TOKEN_BALANCE_OPERATION_ID,
        binding.clone(),
        contracts.token_balance_request.clone(),
        contracts.quantity_response.clone(),
        contracts.safe_failure.clone(),
        contracts.read_failure.clone(),
        token_balance_execution(
            binding.clone(),
            contracts.token_balance_request.clone(),
            contracts.quantity_response.clone(),
            contracts.safe_failure.clone(),
        )?,
    )?;
    let confirm_anchor = qualify_read_state(
        implementations.confirm_anchor,
        unit.clone(),
        contracts.anchor_confirmation_input.clone(),
        vec![ordinary_input(
            "fanout",
            contracts.shared.balance_graph_result.clone(),
        )?],
        contracts.shared.balance_graph_result.clone(),
        EVM_CONFIRM_ANCHOR_OPERATION_ID,
        binding.clone(),
        contracts.anchor_confirmation_request.clone(),
        contracts.block_response.clone(),
        contracts.safe_failure.clone(),
        contracts.read_failure.clone(),
        confirmation_execution(
            binding,
            contracts.anchor_confirmation_request.clone(),
            contracts.block_response.clone(),
            contracts.safe_failure.clone(),
        )?,
    )?;
    let aggregate_balances =
        qualify_aggregate_state(implementations.aggregate_balances, unit, &contracts)?;

    Ok(QualifiedEvmBalanceCollectionStates {
        bootstrap_source,
        read_initial_anchor,
        read_token_decimals,
        read_native_balance,
        read_token_balance,
        confirm_anchor,
        aggregate_balances,
    })
}

struct EvmStateValueContracts {
    shared: EvmBalanceCollectionValueContracts,
    checked_source: RetainedValueContract,
    anchored_source: RetainedValueContract,
    read_failure: RetainedValueContract,
    safe_failure: RetainedValueContract,
    chain_identity_request: RetainedValueContract,
    chain_identity_response: RetainedValueContract,
    latest_anchor_request: RetainedValueContract,
    block_response: RetainedValueContract,
    token_decimals_request: RetainedValueContract,
    token_decimals_response: RetainedValueContract,
    native_balance_request: RetainedValueContract,
    quantity_response: RetainedValueContract,
    token_balance_request: RetainedValueContract,
    anchor_confirmation_request: RetainedValueContract,
    bootstrap_input: RetainedValueContract,
    initial_anchor_input: RetainedValueContract,
    token_decimals_input: RetainedValueContract,
    balance_read_input: RetainedValueContract,
    anchor_confirmation_input: RetainedValueContract,
    aggregation_input: RetainedValueContract,
}

impl EvmStateValueContracts {
    fn new(evidence: ContentRef) -> Result<Self> {
        let read = evm_read_value_contracts(evidence.clone())?;
        Ok(Self {
            shared: evm_balance_collection_value_contracts(evidence.clone())?,
            checked_source: value_contract::<EvmCheckedSource>(
                "mfm.evm.value.checked-source",
                evidence.clone(),
            )?,
            anchored_source: value_contract::<EvmAnchoredSource>(
                "mfm.evm.value.anchored-source",
                evidence.clone(),
            )?,
            read_failure: value_contract::<EvmReadFailure>(
                "mfm.evm.failure.read",
                evidence.clone(),
            )?,
            safe_failure: read.safe_failure,
            chain_identity_request: read.chain_identity_request,
            chain_identity_response: read.chain_identity_response,
            latest_anchor_request: read.latest_anchor_request,
            block_response: read.block_response,
            token_decimals_request: read.token_decimals_request,
            token_decimals_response: read.token_decimals_response,
            native_balance_request: read.native_balance_request,
            quantity_response: read.quantity_response,
            token_balance_request: read.token_balance_request,
            anchor_confirmation_request: read.anchor_confirmation_request,
            bootstrap_input: input_contract::<EvmBootstrapInput>("bootstrap", evidence.clone())?,
            initial_anchor_input: input_contract::<EvmInitialAnchorInput>(
                "initial-anchor",
                evidence.clone(),
            )?,
            token_decimals_input: input_contract::<EvmTokenDecimalsInput>(
                "token-decimals",
                evidence.clone(),
            )?,
            balance_read_input: input_contract::<EvmBalanceReadInput>(
                "balance-read",
                evidence.clone(),
            )?,
            anchor_confirmation_input: input_contract::<EvmAnchorConfirmationInput>(
                "anchor-confirmation",
                evidence.clone(),
            )?,
            aggregation_input: input_contract::<EvmBalanceAggregationInput>(
                "balance-aggregation",
                evidence,
            )?,
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn qualify_read_state<S>(
    implementation: ComponentImplementationDescriptor,
    unit_config_contract: RetainedValueContract,
    input_contract: RetainedValueContract,
    input_destinations: Vec<QualifiedInputContract>,
    output_contract: RetainedValueContract,
    operation_id: &'static str,
    binding_ref: ContentRef,
    request_contract: RetainedValueContract,
    returned_contract: RetainedValueContract,
    safe_failure_contract: RetainedValueContract,
    state_failure_contract: RetainedValueContract,
    execution: StateExecution<S>,
) -> Result<QualifiedStateRegistration<S>>
where
    S: State<Failure = EvmReadFailure>,
    S::Output: MfmValue,
{
    let certified_execution = CertifiedStateExecution::Read {
        capability_operation_id: stable_id(operation_id)?,
        capability_binding_ref: binding_ref,
        request_contract,
        returned_contract,
        safe_failure_contract,
    };
    let settlement = settlement_contract(
        output_contract.clone(),
        state_failure_contract.clone(),
        Vec::new(),
    )?;
    let contract = QualifiedStateContract::new(
        S::state_contract_ref()?,
        unit_config_contract,
        None,
        input_contract,
        input_destinations,
        vec![QualifiedOutputSourceContract::new(
            0,
            QualifiedSourceContract::new(output_contract.clone(), Vec::new())?,
        )],
        certified_execution,
        settlement,
    )?;
    QualifiedStateRegistration::new(
        contract,
        implementation,
        execution,
        settlement_codecs(output_contract, state_failure_contract)?,
    )
}

fn qualify_aggregate_state(
    implementation: ComponentImplementationDescriptor,
    unit_config_contract: RetainedValueContract,
    contracts: &EvmStateValueContracts,
) -> Result<QualifiedStateRegistration<AggregateEvmBalancesState>> {
    let output_contract = contracts.shared.balance_collection.clone();
    let subject_contract = mfm_value_contract::<EvmBalanceSnapshotSubject>(
        stable_id("mfm.evm.fact.balance-snapshot.subject")?,
        fact_evidence_contract_ref("subject")?,
    )?;
    let response_contract = mfm_value_contract::<EvmBalanceSnapshotFact>(
        stable_id("mfm.evm.fact.balance-snapshot.response")?,
        fact_evidence_contract_ref("response")?,
    )?;
    let fact_slot = CertifiedFactSlot::new(
        0,
        1,
        u32::try_from(EVM_BALANCE_COLLECTION_SOURCE_LIMIT)
            .map_err(|_| ProgramError::Registry("EVM source limit exceeds u32".to_owned()))?,
        balance_fact_descriptor_ref()?,
        subject_contract,
        response_contract,
    )?;
    let settlement = settlement_contract(
        output_contract.clone(),
        contracts.read_failure.clone(),
        vec![fact_slot],
    )?;
    let contract = QualifiedStateContract::new(
        AggregateEvmBalancesState::state_contract_ref()?,
        unit_config_contract,
        None,
        contracts.aggregation_input.clone(),
        vec![
            ordinary_input("binding", contracts.shared.network_binding.clone())?,
            ordinary_input("native_decimals", contracts.shared.native_decimals.clone())?,
            ordinary_input("results", contracts.shared.balance_graph_result.clone())?,
            ordinary_input("sources", contracts.shared.balance_source.clone())?,
            ordinary_input("token_contracts", contracts.shared.token_contract.clone())?,
        ],
        vec![QualifiedOutputSourceContract::new(
            0,
            QualifiedSourceContract::new(output_contract.clone(), Vec::new())?,
        )],
        CertifiedStateExecution::Pure,
        settlement,
    )?;
    QualifiedStateRegistration::new(
        contract,
        implementation,
        aggregate_execution(),
        settlement_codecs(output_contract, contracts.read_failure.clone())?,
    )
}

fn settlement_contract(
    output_contract: RetainedValueContract,
    failure_contract: RetainedValueContract,
    fact_slots: Vec<CertifiedFactSlot>,
) -> Result<CertifiedSettlementContract> {
    CertifiedSettlementContract::new(
        Some(failure_contract),
        vec![CertifiedOutputSlot::new(
            0,
            field_path("value")?,
            output_contract,
        )],
        fact_slots,
    )
    .map_err(Into::into)
}

fn settlement_codecs<S>(
    output_contract: RetainedValueContract,
    failure_contract: RetainedValueContract,
) -> Result<QualifiedSettlementCodecs<S>>
where
    S: State<Failure = EvmReadFailure>,
    S::Output: MfmValue,
{
    QualifiedSettlementCodecs::new(
        vec![QualifiedOutputProjector::new(
            0,
            field_path("value")?,
            CanonicalCodec::mfm_value(output_contract)?,
            identity::<S::Output>,
        )],
        Some(CanonicalCodec::mfm_value(failure_contract)?),
    )
}

fn ordinary_input(
    path: &'static str,
    value_contract: RetainedValueContract,
) -> Result<QualifiedInputContract> {
    Ok(QualifiedInputContract::new(
        field_path(path)?,
        CertifiedInputDestination::OrdinaryValue,
        QualifiedSourceContract::new(value_contract, Vec::new())?,
    ))
}

fn value_contract<T: MfmValue>(
    role: &'static str,
    evidence_contract_ref: ContentRef,
) -> Result<RetainedValueContract> {
    mfm_value_contract::<T>(stable_id(role)?, evidence_contract_ref)
}

fn input_contract<T: StateInput>(
    name: &'static str,
    evidence_contract_ref: ContentRef,
) -> Result<RetainedValueContract> {
    state_input_value_contract::<T>(
        semantic_type_id(&format!("state-input-{name}"))?,
        stable_id(format!("mfm.evm.input.{name}"))?,
        evidence_contract_ref,
    )
}

fn scalar_contract(
    name: &'static str,
    role: &'static str,
    evidence_contract_ref: ContentRef,
) -> Result<RetainedValueContract> {
    RetainedValueContract::new(
        crate::state::contract_schema_id(match name {
            "native-decimals" => "mfm.evm.native-decimals",
            "token-contract" => "mfm.evm.token-contract",
            _ => {
                return Err(ProgramError::Registry(
                    "unknown EVM scalar contract".to_owned(),
                ))
            }
        })?,
        semantic_type_id(name)?,
        stable_id(role)?,
        "application/json",
        evidence_contract_ref,
    )
    .map_err(|error| ProgramError::Codec(error.to_string()))
}

fn semantic_type_id(name: &str) -> Result<SemanticTypeId> {
    SemanticTypeId::new(
        "mfm.evm",
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        mfm_canonical::sha256_digest_bytes(format!("semantic:mfm.evm:{name}:1").as_bytes()),
    )
    .map_err(|error| ProgramError::Codec(error.to_string()))
}

fn stable_id(value: impl AsRef<str>) -> Result<StableId> {
    StableId::new(value).map_err(|error| ProgramError::Codec(error.to_string()))
}

fn field_path(value: impl AsRef<str>) -> Result<FieldPath> {
    FieldPath::new(value).map_err(|error| ProgramError::Codec(error.to_string()))
}

const fn identity<T>(value: &T) -> &T {
    value
}

/// Returns the exact callback surface of the one EVM wallet effect state.
pub fn evm_submit_transaction_callback_surface() -> Result<EvmStateCallbackSurface> {
    EvmStateCallbackSurface::effect::<SubmitEvmTransactionState>(
        "submit_transaction",
        EVM_SUBMIT_TRANSACTION_OPERATION_ID,
    )
}

/// Builds the executor contract's leaf expansion for EVM wallet submission.
pub fn evm_submit_transaction_leaf_expansion(
    expansion_contract_ref: ContentRef,
) -> Result<RequiredPlanExpansion> {
    RequiredPlanExpansion::new(
        EVM_SUBMIT_TRANSACTION_OPERATION_ID,
        evm_submit_transaction_callback_surface()?
            .content_ref()
            .clone(),
        expansion_contract_ref,
    )
    .map_err(|error| ProgramError::Registry(error.to_string()))
}

/// Closed retained-value inventory for the EVM transaction effect boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmSubmitTransactionValueContracts {
    request: RetainedValueContract,
    selector: RetainedValueContract,
    input: RetainedValueContract,
    outcome: RetainedValueContract,
    failure: RetainedValueContract,
    attempt_result: RetainedValueContract,
    terminal_evidence: RetainedValueContract,
}

impl EvmSubmitTransactionValueContracts {
    /// Returns the immutable semantic-request contract.
    pub const fn request(&self) -> &RetainedValueContract {
        &self.request
    }

    /// Returns the public selector contract.
    pub const fn selector(&self) -> &RetainedValueContract {
        &self.selector
    }

    /// Returns the state input contract.
    pub const fn input(&self) -> &RetainedValueContract {
        &self.input
    }

    /// Returns the finalized outcome contract.
    pub const fn outcome(&self) -> &RetainedValueContract {
        &self.outcome
    }

    /// Returns the uninhabited semantic-failure contract.
    pub const fn failure(&self) -> &RetainedValueContract {
        &self.failure
    }

    /// Returns the closed five-variant attempt-result contract.
    pub const fn attempt_result(&self) -> &RetainedValueContract {
        &self.attempt_result
    }

    /// Returns the state-owned terminal domain-evidence contract.
    pub const fn terminal_evidence(&self) -> &RetainedValueContract {
        &self.terminal_evidence
    }
}

/// Builds all domain-owned wallet retained-value contracts.
pub fn evm_submit_transaction_value_contracts(
    object_evidence_contract_ref: ContentRef,
) -> Result<EvmSubmitTransactionValueContracts> {
    Ok(EvmSubmitTransactionValueContracts {
        request: value_contract::<EvmSubmitTransactionRequest>(
            "mfm.evm.wallet.request",
            object_evidence_contract_ref.clone(),
        )?,
        selector: value_contract::<EvmSubmitTransactionSelector>(
            "mfm.evm.wallet.selector",
            object_evidence_contract_ref.clone(),
        )?,
        input: input_contract::<EvmSubmitTransactionInput>(
            "submit-transaction",
            object_evidence_contract_ref.clone(),
        )?,
        outcome: value_contract::<EvmTransactionOutcome>(
            "mfm.evm.wallet.outcome",
            object_evidence_contract_ref.clone(),
        )?,
        failure: value_contract::<EvmSubmitTransactionFailure>(
            "mfm.evm.wallet.failure",
            object_evidence_contract_ref.clone(),
        )?,
        attempt_result: value_contract::<EvmWalletAttemptResult>(
            "mfm.evm.wallet.attempt-result",
            object_evidence_contract_ref.clone(),
        )?,
        terminal_evidence: value_contract::<EvmWalletTerminalEvidence>(
            "mfm.evm.wallet.terminal-evidence",
            object_evidence_contract_ref,
        )?,
    })
}

/// Product-owned identities required to qualify the wallet effect state.
pub struct EvmSubmitTransactionStateArtifacts {
    object_evidence_contract_ref: ContentRef,
    unit_config_contract: RetainedValueContract,
    executor_contract_ref: ContentRef,
    executor_binding_ref: ContentRef,
    ensure_result_contract: RetainedValueContract,
    terminal_effect_evidence_contract: RetainedValueContract,
    implementation: ComponentImplementationDescriptor,
}

impl EvmSubmitTransactionStateArtifacts {
    /// Constructs and validates the exact wallet-state artifact set.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        object_evidence_contract_ref: ContentRef,
        unit_config_contract: RetainedValueContract,
        executor_contract_ref: ContentRef,
        executor_binding_ref: ContentRef,
        ensure_result_contract: RetainedValueContract,
        terminal_effect_evidence_contract: RetainedValueContract,
        implementation: ComponentImplementationDescriptor,
    ) -> Result<Self> {
        let expected_unit_config = unit_config_value_contract(
            unit_config_contract.role().clone(),
            object_evidence_contract_ref.clone(),
        )?;
        let surface = evm_submit_transaction_callback_surface()?;
        if unit_config_contract != expected_unit_config
            || implementation.component_kind() != ComponentKind::State
            || implementation.semantic_contract_ref() != surface.state_contract_ref()
            || implementation.callback_surface_ref() != surface.content_ref()
        {
            return Err(ProgramError::Registry(
                "EVM wallet state artifacts differ from the exact package contract".to_owned(),
            ));
        }
        Ok(Self {
            object_evidence_contract_ref,
            unit_config_contract,
            executor_contract_ref,
            executor_binding_ref,
            ensure_result_contract,
            terminal_effect_evidence_contract,
            implementation,
        })
    }
}

/// Qualified registration for the one EVM wallet effect state.
pub struct QualifiedEvmSubmitTransactionState {
    registration: QualifiedStateRegistration<SubmitEvmTransactionState>,
}

impl QualifiedEvmSubmitTransactionState {
    /// Registers the wallet state into one product registry.
    pub fn register_into(
        self,
        registry: &mut QualifiedProgramRegistryBuilder,
    ) -> Result<&mut QualifiedProgramRegistryBuilder> {
        registry.register_state(self.registration)
    }

    /// Returns the exact qualified registration.
    pub const fn registration(&self) -> &QualifiedStateRegistration<SubmitEvmTransactionState> {
        &self.registration
    }
}

/// Qualifies the one EVM wallet effect state and all typed boundary contracts.
pub fn qualify_evm_submit_transaction_state(
    artifacts: EvmSubmitTransactionStateArtifacts,
) -> Result<QualifiedEvmSubmitTransactionState> {
    let contracts = evm_submit_transaction_value_contracts(artifacts.object_evidence_contract_ref)?;
    let settlement = settlement_contract(
        contracts.outcome.clone(),
        contracts.failure.clone(),
        Vec::new(),
    )?;
    let certified_execution = CertifiedStateExecution::Effect {
        executor_operation_id: stable_id(EVM_SUBMIT_TRANSACTION_OPERATION_ID)?,
        executor_binding_ref: artifacts.executor_binding_ref.clone(),
        request_contract: contracts.request.clone(),
        ensure_result_contract: artifacts.ensure_result_contract.clone(),
        terminal_evidence_contract: artifacts.terminal_effect_evidence_contract.clone(),
        domain_result_contract: contracts.attempt_result.clone(),
    };
    let contract = QualifiedStateContract::new(
        SubmitEvmTransactionState::state_contract_ref()?,
        artifacts.unit_config_contract,
        None,
        contracts.input,
        vec![
            ordinary_input("request", contracts.request.clone())?,
            ordinary_input("selector", contracts.selector)?,
        ],
        vec![QualifiedOutputSourceContract::new(
            0,
            QualifiedSourceContract::new(contracts.outcome.clone(), Vec::new())?,
        )],
        certified_execution,
        settlement,
    )?;
    let execution = submit_transaction_execution(
        artifacts.executor_contract_ref,
        artifacts.executor_binding_ref,
        contracts.request,
        artifacts.ensure_result_contract,
        artifacts.terminal_effect_evidence_contract,
        contracts.attempt_result,
    )?;
    let codecs = QualifiedSettlementCodecs::new(
        vec![QualifiedOutputProjector::new(
            0,
            field_path("value")?,
            CanonicalCodec::mfm_value(contracts.outcome)?,
            identity::<EvmTransactionOutcome>,
        )],
        Some(CanonicalCodec::mfm_value(contracts.failure)?),
    )?;
    Ok(QualifiedEvmSubmitTransactionState {
        registration: QualifiedStateRegistration::new(
            contract,
            artifacts.implementation,
            execution,
            codecs,
        )?,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use mfm_ids::{ContentDigest, SchemaId};

    use super::*;

    fn content_ref(label: &'static [u8]) -> ContentRef {
        ContentRef::new(
            SchemaId::new(
                "mfm.test.evm-qualification",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                mfm_canonical::sha256_digest_bytes(b"EVM qualification schema"),
            )
            .expect("schema"),
            ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                mfm_canonical::sha256_digest_bytes(label),
            ),
        )
        .expect("content ref")
    }

    fn qualified_states() -> (QualifiedEvmBalanceCollectionStates, ContentRef) {
        let surfaces = evm_balance_collection_callback_surfaces().expect("callback surfaces");
        let qualification = content_ref(b"qualification");
        let implementation = |surface: &EvmStateCallbackSurface| {
            ComponentImplementationDescriptor::new(
                ComponentKind::State,
                surface.state_contract_ref().clone(),
                surface.content_ref().clone(),
                qualification.clone(),
            )
            .expect("implementation")
        };
        let evidence = content_ref(b"object evidence");
        let unit = unit_config_value_contract(
            StableId::new("mfm.test.evm.unit-config").expect("unit role"),
            evidence.clone(),
        )
        .expect("unit contract");
        let binding = content_ref(b"read binding");
        let states = qualify_evm_balance_collection_states(
            EvmBalanceCollectionStateArtifacts::new(
                evidence,
                unit,
                binding.clone(),
                EvmBalanceCollectionStateImplementations {
                    bootstrap_source: implementation(surfaces.bootstrap_source()),
                    read_initial_anchor: implementation(surfaces.read_initial_anchor()),
                    read_token_decimals: implementation(surfaces.read_token_decimals()),
                    read_native_balance: implementation(surfaces.read_native_balance()),
                    read_token_balance: implementation(surfaces.read_token_balance()),
                    confirm_anchor: implementation(surfaces.confirm_anchor()),
                    aggregate_balances: implementation(surfaces.aggregate_balances()),
                },
            )
            .expect("artifacts"),
        )
        .expect("qualified states");
        (states, binding)
    }

    #[test]
    fn callback_surface_inventory_is_exact_closed_and_unique() {
        let surfaces = evm_balance_collection_callback_surfaces().expect("callback surfaces");
        let ordered = surfaces.ordered();
        let evidence_ref = content_ref(b"state contract evidence");
        assert_eq!(ordered.len(), 7);
        assert_eq!(
            ordered
                .iter()
                .map(|surface| surface.state_contract_ref())
                .collect::<BTreeSet<_>>()
                .len(),
            7
        );
        assert_eq!(
            ordered
                .iter()
                .map(|surface| surface.content_ref())
                .collect::<BTreeSet<_>>()
                .len(),
            7
        );
        assert_eq!(
            ordered
                .iter()
                .filter(|surface| {
                    surface
                        .canonical()
                        .as_str()
                        .contains("\"external_operation_id\":null")
                })
                .count(),
            1
        );
        for surface in ordered {
            let contract = surface
                .state_contract_support_contract(
                    StableId::new("mfm.test.evm.state-contract").expect("state role"),
                    evidence_ref.clone(),
                )
                .expect("state contract support metadata");
            assert_eq!(
                boundary_content_ref(
                    contract.schema_id().clone(),
                    surface.state_contract_canonical()
                )
                .expect("state contract identity"),
                *surface.state_contract_ref()
            );
            let rendered = surface.canonical().as_str();
            assert!(rendered.contains(EVM_STATE_CALLBACK_SURFACE_VERSION));
            for forbidden in [
                "endpoint",
                "authorization",
                "private_key",
                "retry",
                "fallback",
            ] {
                assert!(!rendered.contains(forbidden));
            }
        }
    }

    #[test]
    fn qualified_graph_has_six_exact_reads_and_one_bounded_fact_group() {
        let (states, binding) = qualified_states();
        let reads = [
            (
                states.bootstrap_source.contract(),
                EVM_CHAIN_ID_OPERATION_ID,
            ),
            (
                states.read_initial_anchor.contract(),
                EVM_LATEST_ANCHOR_OPERATION_ID,
            ),
            (
                states.read_token_decimals.contract(),
                EVM_TOKEN_DECIMALS_OPERATION_ID,
            ),
            (
                states.read_native_balance.contract(),
                EVM_NATIVE_BALANCE_OPERATION_ID,
            ),
            (
                states.read_token_balance.contract(),
                EVM_TOKEN_BALANCE_OPERATION_ID,
            ),
            (
                states.confirm_anchor.contract(),
                EVM_CONFIRM_ANCHOR_OPERATION_ID,
            ),
        ];
        for (contract, operation_id) in reads {
            let (actual_operation, actual_binding) = contract
                .execution()
                .operation_binding()
                .expect("read operation");
            assert_eq!(actual_operation.as_str(), operation_id);
            assert_eq!(actual_binding, &binding);
            assert!(contract.settlement_contract().fact_slots().is_empty());
        }

        let aggregate = states.aggregate_balances.contract();
        assert_eq!(aggregate.execution(), &CertifiedStateExecution::Pure);
        assert_eq!(
            aggregate
                .input_destinations()
                .iter()
                .map(|input| input.destination_field_path().as_str())
                .collect::<Vec<_>>(),
            [
                "binding",
                "native_decimals",
                "results",
                "sources",
                "token_contracts",
            ]
        );
        let facts = aggregate.settlement_contract().fact_slots();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].fact_slot_ordinal(), 0);
        assert_eq!(facts[0].minimum_emissions(), 1);
        assert_eq!(
            facts[0].maximum_emissions(),
            u32::try_from(EVM_BALANCE_COLLECTION_SOURCE_LIMIT).expect("bounded source limit")
        );
        assert_eq!(
            facts[0].fact_descriptor_ref(),
            &balance_fact_descriptor_ref().expect("fact descriptor")
        );
    }
}
