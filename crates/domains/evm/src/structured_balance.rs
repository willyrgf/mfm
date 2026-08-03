//! Structured EVM balance states, inner fan-out authoring, and process registration.

use std::collections::BTreeSet;
use std::sync::Arc;

use alloy_primitives::{Address, B256, U256};
use mfm_capabilities::{
    CapabilityContractFault, ReadCapabilityContract, ReadCapabilityImplementation,
};
use mfm_certify::structured::ProgramRegistryBuilder;
use mfm_ids::{ContentRef, StableId};
use mfm_program::structured::{
    state_contract, AllowsExecution, AllowsFanOut, AuthoringPolicy, BlockBuilder,
    DefaultFailureMapper, Direct, FailureValue, FanOutResults, Never, OperationBuilder, Pure, Read,
    RuntimeReadCapability, SafeFailureMayFail, SafeFailureNotApplicable, State, StateFrame,
    StateSettlement, StructuredStateCallbacks, Value,
};
use mfm_program_derive::MfmValue;
use mfm_spec::structured::{
    LaneOutcome, ProposedStateOutcome, SecretFreeImplementationDescriptor, StructuredComponentKind,
    StructuredLiveComponentContract,
};
use serde::{Deserialize, Serialize};

use crate::{
    ChainInstanceDeclaration, ChainInstanceRegistryAttestation, EvmAnchorConfirmationRequest,
    EvmAnchoredSource, EvmBalanceAsset, EvmBalanceCollection, EvmBalanceCollectionError,
    EvmBalanceSource, EvmBlockAnchor, EvmBlockResponse, EvmChainIdentityRequest,
    EvmChainIdentityResponse, EvmCheckedSource, EvmCollectedBalance, EvmLatestAnchorRequest,
    EvmNativeBalanceRequest, EvmNetworkBinding, EvmQuantityResponse, EvmReadFailure,
    EvmSubmissionProcessQualification, EvmTokenBalanceRequest, EvmTokenDecimalsRequest,
    EvmTokenDecimalsResponse, EvmWalletReference,
};

/// Stable operation identity for the reusable structured EVM balance fragment.
pub const STRUCTURED_EVM_BALANCE_COLLECTION_OPERATION_ID: &str = "mfm.evm/balance-collection";
/// Maximum total source lanes retained by one nominal balance-program input.
pub const EVM_BALANCE_PROGRAM_LANE_LIMIT: usize = 4_096;

/// One independently authored source lane and its opaque caller correlation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "structured-balance-lane-input",
    version = "1",
    schema = "mfm.evm.structured_balance_lane_input"
)]
pub struct EvmBalanceLaneInput {
    collection_ordinal: u32,
    caller_context: String,
    binding: EvmNetworkBinding,
    native_decimals: u8,
    source: EvmBalanceSource,
}

/// One nominal admission input containing the complete declaration-ordered balance demand.
///
/// A structured program has exactly one input root of this type regardless of the number of
/// configured lanes. Pure selector states derive the individual lane cursors used by the nested
/// fan-out, so configuration-specific topology does not change the registered entry signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "structured-balance-operation-input",
    version = "1",
    schema = "mfm.evm.structured_balance_operation_input"
)]
pub struct EvmBalanceOperationInput {
    lanes: Vec<EvmBalanceLaneInput>,
}

impl EvmBalanceOperationInput {
    /// Constructs one bounded, non-empty declaration-ordered lane aggregate.
    pub fn new(lanes: Vec<EvmBalanceLaneInput>) -> Result<Self, EvmBalanceCollectionError> {
        if lanes.is_empty() || lanes.len() > EVM_BALANCE_PROGRAM_LANE_LIMIT {
            return Err(EvmBalanceCollectionError::Invalid(
                "balance program input lane count is out of bounds",
            ));
        }
        if lanes.iter().any(|lane| !lane.validate()) {
            return Err(EvmBalanceCollectionError::Invalid(
                "balance program input contains an invalid lane",
            ));
        }
        Ok(Self { lanes })
    }

    /// Returns the complete declaration-ordered lane demand.
    pub fn lanes(&self) -> &[EvmBalanceLaneInput] {
        &self.lanes
    }
}

/// Certified intermediate selecting one lane while retaining the unconsumed suffix.
///
/// Fields remain private because only registered EVM selector states may derive this value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "structured-balance-lane-cursor",
    version = "1",
    schema = "mfm.evm.structured_balance_lane_cursor"
)]
pub struct EvmBalanceLaneCursor {
    selected: EvmBalanceLaneInput,
    remaining: Vec<EvmBalanceLaneInput>,
}

impl EvmBalanceLaneCursor {
    fn first(input: &EvmBalanceOperationInput) -> Option<Self> {
        let (selected, remaining) = input.lanes.split_first()?;
        Some(Self {
            selected: selected.clone(),
            remaining: remaining.to_vec(),
        })
    }

    fn advance(&self) -> Option<Self> {
        let (selected, remaining) = self.remaining.split_first()?;
        Some(Self {
            selected: selected.clone(),
            remaining: remaining.to_vec(),
        })
    }

    fn selected(&self) -> &EvmBalanceLaneInput {
        &self.selected
    }

    fn is_exhausted(&self) -> bool {
        self.remaining.is_empty()
    }
}

impl EvmBalanceLaneInput {
    /// Creates one exact source lane. Caller context must be canonical JSON.
    pub fn new(
        collection_ordinal: u32,
        caller_context: String,
        binding: EvmNetworkBinding,
        native_decimals: u8,
        source: EvmBalanceSource,
    ) -> Result<Self, EvmBalanceCollectionError> {
        mfm_canonical::PlainCanonicalJsonBytes::from_canonical_json_slice(
            caller_context.as_bytes(),
        )
        .map_err(|_| EvmBalanceCollectionError::Invalid("caller context is not canonical"))?;
        source.account_address()?;
        source.contract_address_value()?;
        Ok(Self {
            collection_ordinal,
            caller_context,
            binding,
            native_decimals,
            source,
        })
    }

    /// Returns the caller-owned collection position.
    pub const fn collection_ordinal(&self) -> u32 {
        self.collection_ordinal
    }

    /// Returns canonical caller-owned context echoed without interpretation.
    pub fn caller_context(&self) -> &str {
        &self.caller_context
    }

    /// Returns the exact semantic network binding.
    pub const fn binding(&self) -> &EvmNetworkBinding {
        &self.binding
    }

    /// Returns the configured native decimal scale.
    pub const fn native_decimals(&self) -> u8 {
        self.native_decimals
    }

    /// Returns the one exact source demand.
    pub const fn source(&self) -> &EvmBalanceSource {
        &self.source
    }

    fn validate(&self) -> bool {
        mfm_canonical::PlainCanonicalJsonBytes::from_canonical_json_slice(
            self.caller_context.as_bytes(),
        )
        .is_ok()
            && self.source.account_address().is_ok()
            && self.source.contract_address_value().is_ok()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "structured-bootstrap-balance-work",
    version = "1",
    schema = "mfm.evm.structured_bootstrap_balance_work"
)]
struct BootstrapBalanceWork {
    lane: EvmBalanceLaneInput,
    checked_source: EvmCheckedSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "structured-anchored-balance-work",
    version = "1",
    schema = "mfm.evm.structured_anchored_balance_work"
)]
struct AnchoredBalanceWork {
    lane: EvmBalanceLaneInput,
    anchored_source: EvmAnchoredSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "structured-token-balance-work",
    version = "1",
    schema = "mfm.evm.structured_token_balance_work"
)]
struct TokenBalanceWork {
    anchored: AnchoredBalanceWork,
    decimals: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "structured-observed-balance-work",
    version = "1",
    schema = "mfm.evm.structured_observed_balance_work"
)]
struct ObservedBalanceWork {
    anchored: AnchoredBalanceWork,
    raw_units: String,
    decimals: u8,
}

/// One completely confirmed EVM source lane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "structured-balance-lane-result",
    version = "1",
    schema = "mfm.evm.structured_balance_lane_result"
)]
pub struct EvmBalanceLaneResult {
    lane: EvmBalanceLaneInput,
    anchored_source: EvmAnchoredSource,
    balance: EvmCollectedBalance,
}

impl EvmBalanceLaneResult {
    /// Returns the exact admitted lane input.
    pub const fn lane(&self) -> &EvmBalanceLaneInput {
        &self.lane
    }

    /// Returns the confirmed source and anchor.
    pub const fn anchored_source(&self) -> &EvmAnchoredSource {
        &self.anchored_source
    }

    /// Returns the collected balance.
    pub const fn balance(&self) -> &EvmCollectedBalance {
        &self.balance
    }
}

/// One caller-correlated collection produced by an inner EVM fan-out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "structured-balance-collection-result",
    version = "1",
    schema = "mfm.evm.structured_balance_collection_result"
)]
pub struct EvmBalanceCollectionResult {
    collection_ordinal: u32,
    caller_context: String,
    collection: EvmBalanceCollection,
}

impl EvmBalanceCollectionResult {
    /// Returns the caller-owned collection ordinal.
    pub const fn collection_ordinal(&self) -> u32 {
        self.collection_ordinal
    }

    /// Returns canonical caller-owned context.
    pub fn caller_context(&self) -> &str {
        &self.caller_context
    }

    /// Returns the EVM-owned exact balance collection.
    pub const fn collection(&self) -> &EvmBalanceCollection {
        &self.collection
    }
}

macro_rules! read_capability {
    ($name:ident, $request:ty, $returned:ty, $capability:literal, $adapter:literal) => {
        #[doc = concat!("Typed structured EVM Read capability for `", $capability, "`.")]
        pub enum $name {}

        impl ReadCapabilityContract for $name {
            type Request = $request;
            type Returned = $returned;
            type SafeFailure = EvmReadFailure;
        }

        impl RuntimeReadCapability for $name {
            fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
                StructuredLiveComponentContract::new_read_capability(
                    stable($capability)?,
                    mfm_spec::structured::structured_value_contract_ref::<$request>()?,
                    mfm_spec::structured::structured_value_contract_ref::<$returned>()?,
                    mfm_spec::structured::structured_value_contract_ref::<EvmReadFailure>()?,
                    balance_adapter_contract($adapter)?.content_ref()?,
                )
                .map_err(Into::into)
            }
        }
    };
}

read_capability!(
    EvmChainIdentityCapability,
    EvmChainIdentityRequest,
    EvmChainIdentityResponse,
    "mfm.evm.capability/chain-identity",
    "mfm.evm.adapter/chain-identity"
);
read_capability!(
    EvmLatestAnchorCapability,
    EvmLatestAnchorRequest,
    EvmBlockResponse,
    "mfm.evm.capability/latest-anchor",
    "mfm.evm.adapter/latest-anchor"
);
read_capability!(
    EvmTokenDecimalsCapability,
    EvmTokenDecimalsRequest,
    EvmTokenDecimalsResponse,
    "mfm.evm.capability/token-decimals",
    "mfm.evm.adapter/token-decimals"
);
read_capability!(
    EvmNativeBalanceCapability,
    EvmNativeBalanceRequest,
    EvmQuantityResponse,
    "mfm.evm.capability/native-balance",
    "mfm.evm.adapter/native-balance"
);
read_capability!(
    EvmTokenBalanceCapability,
    EvmTokenBalanceRequest,
    EvmQuantityResponse,
    "mfm.evm.capability/token-balance",
    "mfm.evm.adapter/token-balance"
);
read_capability!(
    EvmConfirmAnchorCapability,
    EvmAnchorConfirmationRequest,
    EvmBlockResponse,
    "mfm.evm.capability/confirm-anchor",
    "mfm.evm.adapter/confirm-anchor"
);

/// Returns one semantic leaf adapter contract for a structured balance read.
pub fn balance_adapter_contract(id: &str) -> mfm_program::Result<StructuredLiveComponentContract> {
    StructuredLiveComponentContract::new(StructuredComponentKind::Adapter, stable(id)?, Vec::new())
        .map_err(Into::into)
}

struct SelectFirstBalanceLaneState;
struct SelectNextBalanceLaneState;
struct AssertBalanceInputExhaustedState;
struct BootstrapBalanceState;
struct ReadInitialAnchorState;
struct ReadTokenDecimalsState;
struct ReadNativeBalanceState;
struct ReadTokenBalanceState;
struct ConfirmBalanceAnchorState;
struct AggregateBalanceLanesState;
struct MapEvmReadFailureState;

macro_rules! pure_balance_selector_state {
    ($state:ty, $input:ty, $id:literal) => {
        impl State for $state {
            type Input = $input;
            type Output = EvmBalanceLaneCursor;
            type Failure = EvmReadFailure;
            type Request = ();
            type Returned = ();
            type SafeFailure = ();
            type Execution = Pure;
            type SafeFailureDisposition = SafeFailureNotApplicable;
            type Capability = Direct;

            fn semantic_state_id() -> mfm_program::Result<StableId> {
                stable($id)
            }
        }
    };
}

pure_balance_selector_state!(
    SelectFirstBalanceLaneState,
    EvmBalanceOperationInput,
    "mfm.evm.state/structured-select-first-balance-lane"
);
pure_balance_selector_state!(
    SelectNextBalanceLaneState,
    EvmBalanceLaneCursor,
    "mfm.evm.state/structured-select-next-balance-lane"
);
pure_balance_selector_state!(
    AssertBalanceInputExhaustedState,
    EvmBalanceLaneCursor,
    "mfm.evm.state/structured-assert-balance-input-exhausted"
);

macro_rules! read_state {
    ($state:ty, $input:ty, $output:ty, $request:ty, $returned:ty, $capability:ty, $id:literal) => {
        impl State for $state {
            type Input = $input;
            type Output = $output;
            type Failure = EvmReadFailure;
            type Request = $request;
            type Returned = $returned;
            type SafeFailure = EvmReadFailure;
            type Execution = Read<$capability>;
            type SafeFailureDisposition = SafeFailureMayFail;
            type Capability = Direct;

            fn semantic_state_id() -> mfm_program::Result<StableId> {
                stable($id)
            }
        }
    };
}

read_state!(
    BootstrapBalanceState,
    EvmBalanceLaneCursor,
    BootstrapBalanceWork,
    EvmChainIdentityRequest,
    EvmChainIdentityResponse,
    EvmChainIdentityCapability,
    "mfm.evm.state/structured-bootstrap-balance"
);
read_state!(
    ReadInitialAnchorState,
    BootstrapBalanceWork,
    AnchoredBalanceWork,
    EvmLatestAnchorRequest,
    EvmBlockResponse,
    EvmLatestAnchorCapability,
    "mfm.evm.state/structured-read-initial-anchor"
);
read_state!(
    ReadTokenDecimalsState,
    AnchoredBalanceWork,
    TokenBalanceWork,
    EvmTokenDecimalsRequest,
    EvmTokenDecimalsResponse,
    EvmTokenDecimalsCapability,
    "mfm.evm.state/structured-read-token-decimals"
);
read_state!(
    ReadNativeBalanceState,
    AnchoredBalanceWork,
    ObservedBalanceWork,
    EvmNativeBalanceRequest,
    EvmQuantityResponse,
    EvmNativeBalanceCapability,
    "mfm.evm.state/structured-read-native-balance"
);
read_state!(
    ReadTokenBalanceState,
    TokenBalanceWork,
    ObservedBalanceWork,
    EvmTokenBalanceRequest,
    EvmQuantityResponse,
    EvmTokenBalanceCapability,
    "mfm.evm.state/structured-read-token-balance"
);
read_state!(
    ConfirmBalanceAnchorState,
    ObservedBalanceWork,
    EvmBalanceLaneResult,
    EvmAnchorConfirmationRequest,
    EvmBlockResponse,
    EvmConfirmAnchorCapability,
    "mfm.evm.state/structured-confirm-balance-anchor"
);

type BalanceLaneJoin = FanOutResults<EvmBalanceLaneResult, EvmReadFailure>;

impl State for AggregateBalanceLanesState {
    type Input = BalanceLaneJoin;
    type Output = EvmBalanceCollectionResult;
    type Failure = EvmReadFailure;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        stable("mfm.evm.state/structured-aggregate-balance-lanes")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "structured-read-failure-route",
    version = "1",
    schema = "mfm.evm.structured_read_failure_route"
)]
enum EvmReadFailureRoute {
    Propagate { failure: EvmReadFailure },
}

impl mfm_program::structured::ClosedSum for EvmReadFailureRoute {}

impl State for MapEvmReadFailureState {
    type Input = EvmReadFailure;
    type Output = EvmReadFailureRoute;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        stable("mfm.evm.state/structured-map-read-failure")
    }
}

/// Identity mapper used by every EVM-owned fallible fragment boundary.
struct EvmReadFailureMapper;

impl DefaultFailureMapper<EvmReadFailure, EvmReadFailure> for EvmReadFailureMapper {
    type Route = EvmReadFailureRoute;
    type Mapper = MapEvmReadFailureState;
}

/// Derives one certified lane cursor per planned source from a single nominal input root.
///
/// The caller must install an exact lexical mapper for `EvmReadFailure` in the containing scope.
/// The final assertion makes a shorter or longer runtime aggregate fail before any fan-out Read.
pub fn author_evm_balance_lane_selection<ScopeFailure, Policy>(
    block: &mut BlockBuilder<ScopeFailure, Policy>,
    input: &Value<EvmBalanceOperationInput>,
    planned_lanes: &[EvmBalanceLaneInput],
) -> mfm_program::Result<Vec<(EvmBalanceLaneInput, Value<EvmBalanceLaneCursor>)>>
where
    ScopeFailure: FailureValue + mfm_values::MfmValue,
    Policy: AuthoringPolicy + AllowsExecution<Pure>,
{
    let Some((first, remaining)) = planned_lanes.split_first() else {
        return Err(mfm_program::ProgramError::Authoring(
            "EVM balance lane selection must contain a source".to_owned(),
        ));
    };
    if planned_lanes.len() > EVM_BALANCE_PROGRAM_LANE_LIMIT {
        return Err(mfm_program::ProgramError::Authoring(
            "EVM balance lane selection exceeds the admitted bound".to_owned(),
        ));
    }

    let mut cursor = block
        .state::<SelectFirstBalanceLaneState>(stable("select-balance-lane/0000")?, input)?
        .or_default()?;
    let mut selected = Vec::with_capacity(planned_lanes.len());
    selected.push((first.clone(), cursor.clone()));
    for (offset, lane) in remaining.iter().enumerate() {
        let ordinal = offset + 1;
        cursor = block
            .state::<SelectNextBalanceLaneState>(
                stable(format!("select-balance-lane/{ordinal:04}"))?,
                &cursor,
            )?
            .or_default()?;
        selected.push((lane.clone(), cursor.clone()));
    }
    let _exhausted = block
        .state::<AssertBalanceInputExhaustedState>(
            stable("assert-balance-input-exhausted")?,
            &cursor,
        )?
        .or_default()?;
    Ok(selected)
}

/// Authors one EVM-owned inner fan-out and exact collection aggregation.
pub fn author_evm_balance_fan_out<Policy>(
    block: &mut BlockBuilder<EvmReadFailure, Policy>,
    label: StableId,
    inputs: &[(EvmBalanceLaneInput, Value<EvmBalanceLaneCursor>)],
) -> mfm_program::Result<Value<EvmBalanceCollectionResult>>
where
    Policy: AuthoringPolicy + AllowsFanOut + AllowsExecution<Pure>,
    Policy::LanePolicy: AllowsExecution<Read<EvmChainIdentityCapability>>
        + AllowsExecution<Read<EvmLatestAnchorCapability>>
        + AllowsExecution<Read<EvmTokenDecimalsCapability>>
        + AllowsExecution<Read<EvmNativeBalanceCapability>>
        + AllowsExecution<Read<EvmTokenBalanceCapability>>
        + AllowsExecution<Read<EvmConfirmAnchorCapability>>,
{
    if inputs.is_empty() {
        return Err(mfm_program::ProgramError::Authoring(
            "EVM balance fan-out must contain a source".to_owned(),
        ));
    }
    block.failure_map::<EvmReadFailure, EvmReadFailureMapper>()?;
    author_evm_balance_fan_out_in_mapped_scope(block, label, inputs)
}

fn author_evm_balance_fan_out_in_mapped_scope<Policy>(
    block: &mut BlockBuilder<EvmReadFailure, Policy>,
    label: StableId,
    inputs: &[(EvmBalanceLaneInput, Value<EvmBalanceLaneCursor>)],
) -> mfm_program::Result<Value<EvmBalanceCollectionResult>>
where
    Policy: AuthoringPolicy + AllowsFanOut + AllowsExecution<Pure>,
    Policy::LanePolicy: AllowsExecution<Read<EvmChainIdentityCapability>>
        + AllowsExecution<Read<EvmLatestAnchorCapability>>
        + AllowsExecution<Read<EvmTokenDecimalsCapability>>
        + AllowsExecution<Read<EvmNativeBalanceCapability>>
        + AllowsExecution<Read<EvmTokenBalanceCapability>>
        + AllowsExecution<Read<EvmConfirmAnchorCapability>>,
{
    let mut fan_out = block.fan_out::<EvmBalanceLaneResult, EvmReadFailure>(label)?;
    for (ordinal, (input, value)) in inputs.iter().enumerate() {
        let lane_label = stable(format!("balance-source/{ordinal:04}"))?;
        fan_out.lane(lane_label, |lane| {
            lane.failure_map::<EvmReadFailure, EvmReadFailureMapper>()?;
            let bootstrap = lane
                .state::<BootstrapBalanceState>(stable("bootstrap")?, value)?
                .or_default()?;
            let anchored = lane
                .state::<ReadInitialAnchorState>(stable("latest-anchor")?, &bootstrap)?
                .or_default()?;
            let observed = match input.source().asset() {
                EvmBalanceAsset::Native => lane
                    .state::<ReadNativeBalanceState>(stable("native-balance")?, &anchored)?
                    .or_default()?,
                EvmBalanceAsset::Erc20 { .. } => {
                    let token = lane
                        .state::<ReadTokenDecimalsState>(stable("token-decimals")?, &anchored)?
                        .or_default()?;
                    lane.state::<ReadTokenBalanceState>(stable("token-balance")?, &token)?
                        .or_default()?
                }
            };
            let confirmed = lane
                .state::<ConfirmBalanceAnchorState>(stable("confirm-anchor")?, &observed)?
                .or_default()?;
            lane.normal(&confirmed)
        })?;
    }
    let joined = fan_out.finish()?;
    block
        .state::<AggregateBalanceLanesState>(stable("aggregate")?, &joined)?
        .or_default()
}

/// Authors one standalone structured EVM balance operation for an exact topology.
pub fn structured_evm_balance_collection_program(
    operation_id: StableId,
    scope_id: StableId,
    lane_inputs: &[EvmBalanceLaneInput],
) -> mfm_program::Result<mfm_spec::structured::AuthoredStructuredProgram> {
    let mut builder = OperationBuilder::<EvmBalanceCollectionResult, EvmReadFailure>::new(
        operation_id,
        scope_id,
    )?;
    let input = builder.input::<EvmBalanceOperationInput>(stable("balance-input")?)?;
    builder
        .root()
        .failure_map::<EvmReadFailure, EvmReadFailureMapper>()?;
    let values = author_evm_balance_lane_selection(builder.root(), &input, lane_inputs)?;
    let output = author_evm_balance_fan_out_in_mapped_scope(
        builder.root(),
        stable("balance-fan-out")?,
        &values,
    )?;
    let completion = builder.succeed(&output)?;
    builder.finish(completion)
}

trait BalanceStateProcess: State {
    fn callbacks(fixture: &BalanceFixture) -> StructuredStateCallbacks<Self>
    where
        Self: Sized;
}

macro_rules! registered_read_state {
    ($state:ty, $field:ident, $request:expr, $settle_returned:expr) => {
        impl BalanceStateProcess for $state {
            fn callbacks(_fixture: &BalanceFixture) -> StructuredStateCallbacks<Self> {
                StructuredStateCallbacks::Read {
                    request: Arc::new($request),
                    settle_returned: Arc::new($settle_returned),
                    settle_safe_failure: Arc::new(|_frame, failure| {
                        ProposedStateOutcome::Failure(*failure)
                    }),
                }
            }
        }
    };
}

registered_read_state!(
    BootstrapBalanceState,
    cursor,
    bootstrap_request,
    settle_bootstrap
);
registered_read_state!(
    ReadInitialAnchorState,
    bootstrap,
    latest_request,
    settle_latest
);
registered_read_state!(
    ReadTokenDecimalsState,
    anchored,
    decimals_request,
    settle_decimals
);
registered_read_state!(
    ReadNativeBalanceState,
    anchored,
    native_request,
    settle_native
);
registered_read_state!(ReadTokenBalanceState, token, token_request, settle_token);
registered_read_state!(
    ConfirmBalanceAnchorState,
    observed,
    confirm_request,
    settle_confirmation
);

impl BalanceStateProcess for AggregateBalanceLanesState {
    fn callbacks(_fixture: &BalanceFixture) -> StructuredStateCallbacks<Self> {
        StructuredStateCallbacks::Pure {
            apply: Arc::new(|frame| aggregate_lanes(frame.input())),
        }
    }
}

impl BalanceStateProcess for SelectFirstBalanceLaneState {
    fn callbacks(_fixture: &BalanceFixture) -> StructuredStateCallbacks<Self> {
        StructuredStateCallbacks::Pure {
            apply: Arc::new(|frame| {
                EvmBalanceLaneCursor::first(frame.input()).map_or(
                    ProposedStateOutcome::Failure(EvmReadFailure::InvalidAggregate),
                    ProposedStateOutcome::Success,
                )
            }),
        }
    }
}

impl BalanceStateProcess for SelectNextBalanceLaneState {
    fn callbacks(_fixture: &BalanceFixture) -> StructuredStateCallbacks<Self> {
        StructuredStateCallbacks::Pure {
            apply: Arc::new(|frame| {
                frame.input().advance().map_or(
                    ProposedStateOutcome::Failure(EvmReadFailure::InvalidAggregate),
                    ProposedStateOutcome::Success,
                )
            }),
        }
    }
}

impl BalanceStateProcess for AssertBalanceInputExhaustedState {
    fn callbacks(_fixture: &BalanceFixture) -> StructuredStateCallbacks<Self> {
        StructuredStateCallbacks::Pure {
            apply: Arc::new(|frame| {
                let input = frame.input();
                if input.is_exhausted() {
                    ProposedStateOutcome::Success(input.clone())
                } else {
                    ProposedStateOutcome::Failure(EvmReadFailure::InvalidAggregate)
                }
            }),
        }
    }
}

impl BalanceStateProcess for MapEvmReadFailureState {
    fn callbacks(_fixture: &BalanceFixture) -> StructuredStateCallbacks<Self> {
        StructuredStateCallbacks::Pure {
            apply: Arc::new(|frame| {
                ProposedStateOutcome::Success(EvmReadFailureRoute::Propagate {
                    failure: *frame.input(),
                })
            }),
        }
    }
}

fn bootstrap_request(frame: StateFrame<'_, EvmBalanceLaneCursor>) -> EvmChainIdentityRequest {
    EvmChainIdentityRequest::new(frame.input().selected().binding().clone())
}

fn settle_bootstrap(
    frame: StateFrame<'_, EvmBalanceLaneCursor>,
    response: &EvmChainIdentityResponse,
) -> StateSettlement<BootstrapBalanceWork, EvmReadFailure> {
    let input = frame.input().selected();
    if !input.validate() || response.chain_id != input.binding().chain_id() {
        return failed(EvmReadFailure::SourceMismatch);
    }
    match EvmCheckedSource::new(
        input.binding().clone(),
        response.source_scope.clone(),
        response.implementation_id.clone(),
    ) {
        Ok(checked_source) => succeeded(BootstrapBalanceWork {
            lane: input.clone(),
            checked_source,
        }),
        Err(_) => StateSettlement::InvalidEvidence,
    }
}

fn latest_request(frame: StateFrame<'_, BootstrapBalanceWork>) -> EvmLatestAnchorRequest {
    EvmLatestAnchorRequest::new(frame.input().checked_source.clone())
}

fn settle_latest(
    frame: StateFrame<'_, BootstrapBalanceWork>,
    response: &EvmBlockResponse,
) -> StateSettlement<AnchoredBalanceWork, EvmReadFailure> {
    let input = frame.input();
    match EvmAnchoredSource::new(input.checked_source.clone(), response.anchor.clone()) {
        Ok(anchored_source) => succeeded(AnchoredBalanceWork {
            lane: input.lane.clone(),
            anchored_source,
        }),
        Err(_) => StateSettlement::InvalidEvidence,
    }
}

fn decimals_request(frame: StateFrame<'_, AnchoredBalanceWork>) -> EvmTokenDecimalsRequest {
    let input = frame.input();
    let contract = input
        .lane
        .source()
        .contract_address_value()
        .ok()
        .flatten()
        .unwrap_or(Address::ZERO);
    EvmTokenDecimalsRequest::new(input.anchored_source.clone(), contract)
}

fn settle_decimals(
    frame: StateFrame<'_, AnchoredBalanceWork>,
    response: &EvmTokenDecimalsResponse,
) -> StateSettlement<TokenBalanceWork, EvmReadFailure> {
    let input = frame.input();
    if matches!(input.lane.source().asset(), EvmBalanceAsset::Erc20 { .. }) {
        succeeded(TokenBalanceWork {
            anchored: input.clone(),
            decimals: response.decimals,
        })
    } else {
        StateSettlement::InvalidEvidence
    }
}

fn native_request(frame: StateFrame<'_, AnchoredBalanceWork>) -> EvmNativeBalanceRequest {
    let input = frame.input();
    let account = input
        .lane
        .source()
        .account_address()
        .unwrap_or(Address::ZERO);
    EvmNativeBalanceRequest::new(input.anchored_source.clone(), account)
}

fn settle_native(
    frame: StateFrame<'_, AnchoredBalanceWork>,
    response: &EvmQuantityResponse,
) -> StateSettlement<ObservedBalanceWork, EvmReadFailure> {
    let input = frame.input();
    if matches!(input.lane.source().asset(), EvmBalanceAsset::Native) && response.quantity().is_ok()
    {
        succeeded(ObservedBalanceWork {
            anchored: input.clone(),
            raw_units: response.quantity_dec().to_owned(),
            decimals: input.lane.native_decimals(),
        })
    } else {
        StateSettlement::InvalidEvidence
    }
}

fn token_request(frame: StateFrame<'_, TokenBalanceWork>) -> EvmTokenBalanceRequest {
    let input = frame.input();
    let account = input
        .anchored
        .lane
        .source()
        .account_address()
        .unwrap_or(Address::ZERO);
    let contract = input
        .anchored
        .lane
        .source()
        .contract_address_value()
        .ok()
        .flatten()
        .unwrap_or(Address::ZERO);
    EvmTokenBalanceRequest::new(input.anchored.anchored_source.clone(), account, contract)
}

fn settle_token(
    frame: StateFrame<'_, TokenBalanceWork>,
    response: &EvmQuantityResponse,
) -> StateSettlement<ObservedBalanceWork, EvmReadFailure> {
    let input = frame.input();
    if matches!(
        input.anchored.lane.source().asset(),
        EvmBalanceAsset::Erc20 { .. }
    ) && response.quantity().is_ok()
    {
        succeeded(ObservedBalanceWork {
            anchored: input.anchored.clone(),
            raw_units: response.quantity_dec().to_owned(),
            decimals: input.decimals,
        })
    } else {
        StateSettlement::InvalidEvidence
    }
}

fn confirm_request(frame: StateFrame<'_, ObservedBalanceWork>) -> EvmAnchorConfirmationRequest {
    EvmAnchorConfirmationRequest::new(frame.input().anchored.anchored_source.clone())
}

fn settle_confirmation(
    frame: StateFrame<'_, ObservedBalanceWork>,
    response: &EvmBlockResponse,
) -> StateSettlement<EvmBalanceLaneResult, EvmReadFailure> {
    let input = frame.input();
    let anchored = &input.anchored;
    if response.anchor.number() != anchored.anchored_source.anchor().number() {
        return StateSettlement::InvalidEvidence;
    }
    if response.anchor.hash() != anchored.anchored_source.anchor().hash() {
        return failed(EvmReadFailure::AnchorChanged);
    }
    succeeded(EvmBalanceLaneResult {
        lane: anchored.lane.clone(),
        anchored_source: anchored.anchored_source.clone(),
        balance: EvmCollectedBalance::new(
            anchored.lane.source().clone(),
            input.raw_units.clone(),
            input.decimals,
        ),
    })
}

fn aggregate_lanes(
    joined: &BalanceLaneJoin,
) -> ProposedStateOutcome<EvmBalanceCollectionResult, EvmReadFailure> {
    let mut successful = Vec::with_capacity(joined.len());
    for outcome in joined.iter() {
        match outcome {
            LaneOutcome::Failure(failure) => return ProposedStateOutcome::Failure(*failure),
            LaneOutcome::Success(result) => successful.push(result),
        }
    }
    let Some(first) = successful.first() else {
        return ProposedStateOutcome::Failure(EvmReadFailure::InvalidAggregate);
    };
    let mut sources = BTreeSet::new();
    if successful.iter().any(|result| {
        result.lane.collection_ordinal != first.lane.collection_ordinal
            || result.lane.caller_context != first.lane.caller_context
            || result.lane.binding != first.lane.binding
            || result.anchored_source != first.anchored_source
            || !sources.insert(result.balance.source().clone())
    }) {
        return ProposedStateOutcome::Failure(EvmReadFailure::InvalidAggregate);
    }
    ProposedStateOutcome::Success(EvmBalanceCollectionResult {
        collection_ordinal: first.lane.collection_ordinal,
        caller_context: first.lane.caller_context.clone(),
        collection: EvmBalanceCollection::new(
            first.anchored_source.clone(),
            successful
                .into_iter()
                .map(|result| result.balance.clone())
                .collect(),
        ),
    })
}

fn succeeded<T>(value: T) -> StateSettlement<T, EvmReadFailure> {
    StateSettlement::Proposed(ProposedStateOutcome::Success(value))
}

fn failed<T>(failure: EvmReadFailure) -> StateSettlement<T, EvmReadFailure> {
    StateSettlement::Proposed(ProposedStateOutcome::Failure(failure))
}

#[allow(dead_code)] // retained for process-registration fixture construction
struct BalanceFixture {
    cursor: EvmBalanceLaneCursor,
    bootstrap: BootstrapBalanceWork,
    anchored: AnchoredBalanceWork,
    token: TokenBalanceWork,
    observed: ObservedBalanceWork,
}

impl BalanceFixture {
    fn new() -> mfm_certify::Result<Self> {
        let schema = mfm_ids::SchemaId::new(
            "mfm.evm.fixture.route",
            "1",
            mfm_ids::DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"structured balance fixture schema"),
        )
        .map_err(certification_error)?;
        let route_ref = ContentRef::new(
            schema,
            mfm_ids::ContentDigest::from_digest(
                mfm_ids::DigestAlgorithm::Sha256V1,
                mfm_canonical::sha256_digest_bytes(b"structured balance fixture route"),
            ),
        )
        .map_err(certification_error)?;
        let chain_registry_ref = EvmWalletReference::from_content_ref(route_ref.clone());
        let declaration = ChainInstanceDeclaration::new(
            chain_registry_ref.clone(),
            StableId::new("mfm.evm.fixture/balance-chain").map_err(certification_error)?,
            1,
            B256::repeat_byte(0x31),
            U256::from(1_u64),
            B256::repeat_byte(0x32),
        )
        .map_err(certification_error)?;
        let chain_instance = ChainInstanceRegistryAttestation::new(
            declaration,
            chain_registry_ref.clone(),
            chain_registry_ref,
        )
        .and_then(|attestation| attestation.binding())
        .map_err(certification_error)?;
        let binding = EvmNetworkBinding::new(
            "fixture-network",
            chain_instance,
            crate::EvmRoutingGenerationRef::from_content_ref(route_ref)
                .map_err(certification_error)?,
        )
        .map_err(certification_error)?;
        let source = EvmBalanceSource::new(
            Address::repeat_byte(0x11),
            EvmBalanceAsset::erc20(Address::repeat_byte(0x22)).map_err(certification_error)?,
        )
        .map_err(certification_error)?;
        let lane = EvmBalanceLaneInput::new(0, "{}".to_owned(), binding.clone(), 18, source)
            .map_err(certification_error)?;
        let cursor = EvmBalanceLaneCursor {
            selected: lane.clone(),
            remaining: Vec::new(),
        };
        let checked_source =
            EvmCheckedSource::new(binding, "fixture-source", "fixture-implementation")
                .map_err(certification_error)?;
        let bootstrap = BootstrapBalanceWork {
            lane: lane.clone(),
            checked_source: checked_source.clone(),
        };
        let anchored_source = EvmAnchoredSource::new(
            checked_source,
            EvmBlockAnchor::new(U256::from(1), B256::repeat_byte(0x33)),
        )
        .map_err(certification_error)?;
        let anchored = AnchoredBalanceWork {
            lane: lane.clone(),
            anchored_source,
        };
        let token = TokenBalanceWork {
            anchored: anchored.clone(),
            decimals: 18,
        };
        let observed = ObservedBalanceWork {
            anchored: anchored.clone(),
            raw_units: "1".to_owned(),
            decimals: 18,
        };
        Ok(Self {
            cursor,
            bootstrap,
            anchored,
            token,
            observed,
        })
    }
}

#[derive(Default)]
struct BalanceCapabilityImplementation;

macro_rules! capability_implementation {
    ($capability:ty) => {
        impl ReadCapabilityImplementation<$capability> for BalanceCapabilityImplementation {
            fn validate_request(
                &self,
                _request: &<$capability as ReadCapabilityContract>::Request,
            ) -> Result<(), CapabilityContractFault> {
                Ok(())
            }

            fn validate_returned(
                &self,
                _returned: &<$capability as ReadCapabilityContract>::Returned,
            ) -> Result<(), CapabilityContractFault> {
                Ok(())
            }

            fn validate_safe_failure(
                &self,
                _failure: &EvmReadFailure,
            ) -> Result<(), CapabilityContractFault> {
                Ok(())
            }
        }
    };
}

capability_implementation!(EvmChainIdentityCapability);
capability_implementation!(EvmLatestAnchorCapability);
capability_implementation!(EvmTokenDecimalsCapability);
capability_implementation!(EvmNativeBalanceCapability);
capability_implementation!(EvmTokenBalanceCapability);
capability_implementation!(EvmConfirmAnchorCapability);

/// Registers the structured balance states and six typed Read capabilities.
pub fn register_evm_balance_process(
    registry: &mut ProgramRegistryBuilder,
    qualification: &EvmSubmissionProcessQualification,
) -> mfm_certify::Result<()> {
    macro_rules! values {
        ($($value:ty),+ $(,)?) => {
            $(registry.register_value::<$value>()?;)+
        };
    }
    values!(
        EvmBalanceLaneInput,
        EvmBalanceOperationInput,
        EvmBalanceLaneCursor,
        BootstrapBalanceWork,
        AnchoredBalanceWork,
        TokenBalanceWork,
        ObservedBalanceWork,
        EvmBalanceLaneResult,
        EvmBalanceCollectionResult,
        EvmReadFailure,
        EvmReadFailureRoute,
        EvmChainIdentityRequest,
        EvmChainIdentityResponse,
        EvmLatestAnchorRequest,
        EvmBlockResponse,
        EvmTokenDecimalsRequest,
        EvmTokenDecimalsResponse,
        EvmNativeBalanceRequest,
        EvmQuantityResponse,
        EvmTokenBalanceRequest,
        EvmAnchorConfirmationRequest,
        EvmBalanceCollection,
    );
    registry.register_closed_sum::<EvmReadFailureRoute>()?;
    let fixture = BalanceFixture::new()?;
    register_state::<SelectFirstBalanceLaneState>(registry, qualification, &fixture)?;
    register_state::<SelectNextBalanceLaneState>(registry, qualification, &fixture)?;
    register_state::<AssertBalanceInputExhaustedState>(registry, qualification, &fixture)?;
    register_state::<BootstrapBalanceState>(registry, qualification, &fixture)?;
    register_state::<ReadInitialAnchorState>(registry, qualification, &fixture)?;
    register_state::<ReadTokenDecimalsState>(registry, qualification, &fixture)?;
    register_state::<ReadNativeBalanceState>(registry, qualification, &fixture)?;
    register_state::<ReadTokenBalanceState>(registry, qualification, &fixture)?;
    register_state::<ConfirmBalanceAnchorState>(registry, qualification, &fixture)?;
    register_state::<AggregateBalanceLanesState>(registry, qualification, &fixture)?;
    register_state::<MapEvmReadFailureState>(registry, qualification, &fixture)?;

    let implementation = Arc::new(BalanceCapabilityImplementation);
    register_capability::<EvmChainIdentityCapability>(
        registry,
        qualification,
        Arc::clone(&implementation),
    )?;
    register_capability::<EvmLatestAnchorCapability>(
        registry,
        qualification,
        Arc::clone(&implementation),
    )?;
    register_capability::<EvmTokenDecimalsCapability>(
        registry,
        qualification,
        Arc::clone(&implementation),
    )?;
    register_capability::<EvmNativeBalanceCapability>(
        registry,
        qualification,
        Arc::clone(&implementation),
    )?;
    register_capability::<EvmTokenBalanceCapability>(
        registry,
        qualification,
        Arc::clone(&implementation),
    )?;
    register_capability::<EvmConfirmAnchorCapability>(registry, qualification, implementation)?;
    Ok(())
}

fn register_state<S>(
    registry: &mut ProgramRegistryBuilder,
    qualification: &EvmSubmissionProcessQualification,
    fixture: &BalanceFixture,
) -> mfm_certify::Result<()>
where
    S: BalanceStateProcess,
{
    let contract = state_contract::<S>().map_err(certification_error)?;
    registry.register_state::<S>(
        descriptor(
            StructuredComponentKind::State,
            contract.state_contract_ref,
            stable(format!(
                "mfm.evm.implementation/{}",
                S::semantic_state_id()
                    .map_err(certification_error)?
                    .as_str()
            ))
            .map_err(certification_error)?,
            qualification,
        ),
        S::callbacks(fixture),
    )?;
    Ok(())
}

fn register_capability<C>(
    registry: &mut ProgramRegistryBuilder,
    qualification: &EvmSubmissionProcessQualification,
    implementation: Arc<BalanceCapabilityImplementation>,
) -> mfm_certify::Result<()>
where
    C: RuntimeReadCapability,
    BalanceCapabilityImplementation: ReadCapabilityImplementation<C>,
{
    let contract = C::contract()
        .and_then(|contract| contract.content_ref().map_err(Into::into))
        .map_err(certification_error)?;
    registry.register_read_capability::<C, _>(
        descriptor(
            StructuredComponentKind::Capability,
            contract.clone(),
            stable(format!(
                "mfm.evm.implementation/balance-capability-{}",
                contract.content_digest().digest()
            ))
            .map_err(certification_error)?,
            qualification,
        ),
        implementation,
    )?;
    Ok(())
}

fn descriptor(
    component_kind: StructuredComponentKind,
    semantic_contract_ref: ContentRef,
    implementation_id: StableId,
    qualification: &EvmSubmissionProcessQualification,
) -> SecretFreeImplementationDescriptor {
    SecretFreeImplementationDescriptor {
        component_kind,
        semantic_contract_ref,
        implementation_id,
        executable_identity_ref: qualification.executable_identity_ref().clone(),
        qualification_artifact_ref: qualification.qualification_artifact_ref().clone(),
    }
}

fn stable(value: impl AsRef<str>) -> mfm_program::Result<StableId> {
    StableId::new(value.as_ref())
        .map_err(|error| mfm_program::ProgramError::Authoring(error.to_string()))
}

fn certification_error(error: impl std::fmt::Display) -> mfm_certify::CertifyError {
    mfm_certify::CertifyError::Certification(error.to_string())
}
