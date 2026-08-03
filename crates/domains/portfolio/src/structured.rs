//! Structured portfolio snapshot authoring, aggregation, and process registration.

use std::sync::Arc;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_certify::structured::ProgramRegistryBuilder;
use mfm_evm::{
    author_evm_balance_fan_out, author_evm_balance_lane_selection, EvmBalanceCollection,
    EvmBalanceCollectionResult, EvmBalanceLaneCursor, EvmBalanceLaneInput,
    EvmBalanceOperationInput, EvmReadFailure,
};
use mfm_ids::{ContentRef, StableId};
use mfm_program::structured::{
    state_contract, DefaultFailureMapper, Direct, FanOutResults, Never, OperationBuilder, Pure,
    SafeFailureNotApplicable, State, StateFrame, StructuredStateCallbacks, Value,
};
use mfm_program_derive::MfmValue;
use mfm_spec::structured::{
    LaneOutcome, ProposedStateOutcome, SecretFreeImplementationDescriptor, StructuredComponentKind,
};
use serde::{Deserialize, Serialize};

use crate::state::{assemble_structured_public_outputs, validate_structured_snapshot_selection};
use crate::{
    PortfolioConfig, PortfolioPublicOutputs, PortfolioRoutingManifest, PortfolioSnapshotFailure,
    PortfolioSnapshotSelector,
};

/// Stable operation identity for the structured portfolio snapshot.
pub const STRUCTURED_PORTFOLIO_SNAPSHOT_OPERATION_ID: &str = "mfm.portfolio/snapshot";

/// One nominal run input binding the selector, configured portfolio, and qualified routing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "structured-snapshot-input",
    version = "1",
    schema = "mfm.portfolio.structured_snapshot_input"
)]
pub struct PortfolioSnapshotInput {
    selector: PortfolioSnapshotSelector,
    portfolio: PortfolioConfig,
    routing_manifest: PortfolioRoutingManifest,
}

impl PortfolioSnapshotInput {
    /// Constructs the one nominal structured-program input.
    pub fn new(
        selector: PortfolioSnapshotSelector,
        portfolio: PortfolioConfig,
        routing_manifest: PortfolioRoutingManifest,
    ) -> Self {
        Self {
            selector,
            portfolio,
            routing_manifest,
        }
    }

    /// Returns the public selector retained by this run input.
    pub const fn selector(&self) -> &PortfolioSnapshotSelector {
        &self.selector
    }

    /// Returns the exact configured portfolio retained by this run input.
    pub const fn portfolio(&self) -> &PortfolioConfig {
        &self.portfolio
    }

    /// Returns the exact qualified routing manifest retained by this run input.
    pub const fn routing_manifest(&self) -> &PortfolioRoutingManifest {
        &self.routing_manifest
    }

    fn canonical_json(&self) -> mfm_program::Result<String> {
        let json = serde_json::to_string(self)
            .map_err(|error| mfm_program::ProgramError::Authoring(error.to_string()))?;
        PlainCanonicalJsonBytes::from_json_str(&json)
            .map(|canonical| canonical.as_str().to_owned())
            .map_err(|error| mfm_program::ProgramError::Authoring(error.to_string()))
    }

    fn decode(canonical: &str) -> Result<Self, ()> {
        PlainCanonicalJsonBytes::from_canonical_json_slice(canonical.as_bytes()).map_err(|_| ())?;
        serde_json::from_str(canonical).map_err(|_| ())
    }
}

/// Compiles one exact admitted portfolio configuration into EVM source roots.
pub fn structured_portfolio_lane_inputs(
    selector: PortfolioSnapshotSelector,
    portfolio: PortfolioConfig,
    routing_manifest: PortfolioRoutingManifest,
) -> mfm_program::Result<Vec<EvmBalanceLaneInput>> {
    structured_portfolio_lane_inputs_from_input(&PortfolioSnapshotInput::new(
        selector,
        portfolio,
        routing_manifest,
    ))
}

fn structured_portfolio_lane_inputs_from_input(
    input: &PortfolioSnapshotInput,
) -> mfm_program::Result<Vec<EvmBalanceLaneInput>> {
    let selection = validate_structured_snapshot_selection(
        input.selector.clone(),
        input.portfolio.clone(),
        input.routing_manifest.clone(),
    )
    .map_err(|()| {
        mfm_program::ProgramError::Authoring(
            "portfolio snapshot selection is not semantically valid".to_owned(),
        )
    })?;
    let caller_context = input.canonical_json()?;

    let mut inputs = Vec::new();
    for position in selection.positions() {
        let config = position
            .collection_config()
            .map_err(|error| mfm_program::ProgramError::Authoring(error.to_string()))?;
        for source in config.sources() {
            inputs.push(
                EvmBalanceLaneInput::new(
                    position.position_ordinal(),
                    caller_context.clone(),
                    config.binding().clone(),
                    config.native_decimals(),
                    source.clone(),
                )
                .map_err(|error| mfm_program::ProgramError::Authoring(error.to_string()))?,
            );
        }
    }
    if inputs.is_empty() {
        return Err(mfm_program::ProgramError::Authoring(
            "portfolio snapshot must demand an EVM balance source".to_owned(),
        ));
    }
    Ok(inputs)
}

type PortfolioCollectionJoin = FanOutResults<EvmBalanceCollectionResult, EvmReadFailure>;

struct PlanPortfolioBalanceInputState;
struct AggregatePortfolioCollectionsState;

impl State for PlanPortfolioBalanceInputState {
    type Input = PortfolioSnapshotInput;
    type Output = EvmBalanceOperationInput;
    type Failure = PortfolioSnapshotFailure;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        stable("mfm.portfolio.state/structured-plan-balance-input")
    }
}

impl State for AggregatePortfolioCollectionsState {
    type Input = PortfolioCollectionJoin;
    type Output = PortfolioPublicOutputs;
    type Failure = PortfolioSnapshotFailure;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        stable("mfm.portfolio.state/structured-aggregate-snapshot")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "structured-snapshot-failure-route",
    version = "1",
    schema = "mfm.portfolio.structured_snapshot_failure_route"
)]
enum PortfolioSnapshotFailureRoute {
    Propagate { failure: PortfolioSnapshotFailure },
}

impl mfm_program::structured::ClosedSum for PortfolioSnapshotFailureRoute {}

struct MapPortfolioSnapshotFailureState;

impl State for MapPortfolioSnapshotFailureState {
    type Input = PortfolioSnapshotFailure;
    type Output = PortfolioSnapshotFailureRoute;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        stable("mfm.portfolio.state/structured-map-snapshot-failure")
    }
}

struct PortfolioSnapshotFailureMapper;

impl DefaultFailureMapper<PortfolioSnapshotFailure, PortfolioSnapshotFailure>
    for PortfolioSnapshotFailureMapper
{
    type Route = PortfolioSnapshotFailureRoute;
    type Mapper = MapPortfolioSnapshotFailureState;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "structured-evm-read-failure-route",
    version = "1",
    schema = "mfm.portfolio.structured_evm_read_failure_route"
)]
enum PortfolioEvmReadFailureRoute {
    Propagate { failure: PortfolioSnapshotFailure },
}

impl mfm_program::structured::ClosedSum for PortfolioEvmReadFailureRoute {}

struct MapPortfolioEvmReadFailureState;

impl State for MapPortfolioEvmReadFailureState {
    type Input = EvmReadFailure;
    type Output = PortfolioEvmReadFailureRoute;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        stable("mfm.portfolio.state/structured-map-evm-read-failure")
    }
}

struct PortfolioEvmReadFailureMapper;

impl DefaultFailureMapper<EvmReadFailure, PortfolioSnapshotFailure>
    for PortfolioEvmReadFailureMapper
{
    type Route = PortfolioEvmReadFailureRoute;
    type Mapper = MapPortfolioEvmReadFailureState;
}

/// Authors the portfolio outer fan-out containing each EVM inner read fan-out.
pub fn structured_portfolio_snapshot_program(
    operation_id: StableId,
    scope_id: StableId,
    lane_inputs: &[EvmBalanceLaneInput],
) -> mfm_program::Result<mfm_spec::structured::AuthoredStructuredProgram> {
    if lane_inputs.is_empty() {
        return Err(mfm_program::ProgramError::Authoring(
            "portfolio snapshot must contain a balance source".to_owned(),
        ));
    }
    let mut builder = OperationBuilder::<PortfolioPublicOutputs, PortfolioSnapshotFailure>::new(
        operation_id,
        scope_id,
    )?;
    let input = builder.input::<PortfolioSnapshotInput>(stable("portfolio-input")?)?;
    builder
        .root()
        .failure_map::<PortfolioSnapshotFailure, PortfolioSnapshotFailureMapper>()?;
    builder
        .root()
        .failure_map::<EvmReadFailure, PortfolioEvmReadFailureMapper>()?;
    let balance_input = builder
        .root()
        .state::<PlanPortfolioBalanceInputState>(stable("plan-balance-input")?, &input)?
        .or_default()?;
    let selected = author_evm_balance_lane_selection(builder.root(), &balance_input, lane_inputs)?;

    let mut grouped = Vec::<Vec<(EvmBalanceLaneInput, Value<EvmBalanceLaneCursor>)>>::new();
    for (input, value) in selected {
        let collection = usize::try_from(input.collection_ordinal()).map_err(|_| {
            mfm_program::ProgramError::Authoring(
                "portfolio collection ordinal exceeds platform limits".to_owned(),
            )
        })?;
        if collection > grouped.len() {
            return Err(mfm_program::ProgramError::Authoring(
                "portfolio collection ordinals must be dense and ordered".to_owned(),
            ));
        }
        if collection == grouped.len() {
            grouped.push(Vec::new());
        }
        grouped[collection].push((input, value));
    }
    if grouped.iter().any(Vec::is_empty) {
        return Err(mfm_program::ProgramError::Authoring(
            "portfolio collection must contain a balance source".to_owned(),
        ));
    }
    let mut outer = builder
        .root()
        .fan_out::<EvmBalanceCollectionResult, EvmReadFailure>(stable(
            "portfolio-network-fan-out",
        )?)?;
    for (collection_ordinal, inputs) in grouped.iter().enumerate() {
        outer.lane(
            stable(format!("portfolio-network/{collection_ordinal:04}"))?,
            |lane| {
                let output =
                    author_evm_balance_fan_out(lane, stable("evm-source-fan-out")?, inputs)?;
                lane.normal(&output)
            },
        )?;
    }
    let collections = outer.finish()?;
    let output = builder
        .root()
        .state::<AggregatePortfolioCollectionsState>(stable("aggregate-portfolio")?, &collections)?
        .or_default()?;
    let completion = builder.succeed(&output)?;
    builder.finish(completion)
}

fn aggregate_portfolio_collections(
    joined: &PortfolioCollectionJoin,
) -> ProposedStateOutcome<PortfolioPublicOutputs, PortfolioSnapshotFailure> {
    let mut context = None;
    let mut collections = Vec::<EvmBalanceCollection>::with_capacity(joined.len());
    for (ordinal, outcome) in joined.iter().enumerate() {
        let result = match outcome {
            LaneOutcome::Failure(_) => {
                return ProposedStateOutcome::Failure(PortfolioSnapshotFailure::InvalidCollection)
            }
            LaneOutcome::Success(result) => result,
        };
        if usize::try_from(result.collection_ordinal()) != Ok(ordinal)
            || context
                .as_deref()
                .is_some_and(|expected| expected != result.caller_context())
        {
            return ProposedStateOutcome::Failure(PortfolioSnapshotFailure::InvalidCollection);
        }
        context.get_or_insert_with(|| result.caller_context().to_owned());
        collections.push(result.collection().clone());
    }
    let Some(context) = context.and_then(|value| PortfolioSnapshotInput::decode(&value).ok())
    else {
        return ProposedStateOutcome::Failure(PortfolioSnapshotFailure::InvalidCollection);
    };
    match assemble_structured_public_outputs(
        context.selector,
        context.portfolio,
        context.routing_manifest,
        &collections,
    ) {
        Ok(outputs) => ProposedStateOutcome::Success(outputs),
        Err(()) => ProposedStateOutcome::Failure(PortfolioSnapshotFailure::InvalidCollection),
    }
}

fn plan_portfolio_balance_input(
    input: &PortfolioSnapshotInput,
) -> ProposedStateOutcome<EvmBalanceOperationInput, PortfolioSnapshotFailure> {
    structured_portfolio_lane_inputs_from_input(input)
        .ok()
        .and_then(|lanes| EvmBalanceOperationInput::new(lanes).ok())
        .map_or(
            ProposedStateOutcome::Failure(PortfolioSnapshotFailure::InvalidCollection),
            ProposedStateOutcome::Success,
        )
}

/// Portfolio-owned registered executable and qualification objects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortfolioProcessQualification {
    executable_identity_ref: ContentRef,
    qualification_artifact_ref: ContentRef,
}

impl PortfolioProcessQualification {
    /// Binds already-registered secret-free process qualification objects.
    pub const fn new(
        executable_identity_ref: ContentRef,
        qualification_artifact_ref: ContentRef,
    ) -> Self {
        Self {
            executable_identity_ref,
            qualification_artifact_ref,
        }
    }

    /// Returns the exact executable identity object.
    pub const fn executable_identity_ref(&self) -> &ContentRef {
        &self.executable_identity_ref
    }

    /// Returns the exact qualification artifact object.
    pub const fn qualification_artifact_ref(&self) -> &ContentRef {
        &self.qualification_artifact_ref
    }
}

/// Registers the portfolio aggregate and its exact failure mapper.
pub fn register_portfolio_process(
    registry: &mut ProgramRegistryBuilder,
    qualification: &PortfolioProcessQualification,
) -> mfm_certify::Result<()> {
    registry.register_value::<PortfolioSnapshotInput>()?;
    registry.register_value::<PortfolioPublicOutputs>()?;
    registry.register_value::<PortfolioSnapshotFailure>()?;
    registry.register_value::<PortfolioSnapshotFailureRoute>()?;
    registry.register_value::<PortfolioEvmReadFailureRoute>()?;
    registry.register_closed_sum::<PortfolioSnapshotFailureRoute>()?;
    registry.register_closed_sum::<PortfolioEvmReadFailureRoute>()?;
    register_state::<PlanPortfolioBalanceInputState>(
        registry,
        qualification,
        StructuredStateCallbacks::Pure {
            apply: Arc::new(|frame: StateFrame<'_, PortfolioSnapshotInput>| {
                plan_portfolio_balance_input(frame.input())
            }),
        },
    )?;
    register_state::<AggregatePortfolioCollectionsState>(
        registry,
        qualification,
        StructuredStateCallbacks::Pure {
            apply: Arc::new(|frame: StateFrame<'_, PortfolioCollectionJoin>| {
                aggregate_portfolio_collections(frame.input())
            }),
        },
    )?;
    register_state::<MapPortfolioSnapshotFailureState>(
        registry,
        qualification,
        StructuredStateCallbacks::Pure {
            apply: Arc::new(|frame: StateFrame<'_, PortfolioSnapshotFailure>| {
                ProposedStateOutcome::Success(PortfolioSnapshotFailureRoute::Propagate {
                    failure: *frame.input(),
                })
            }),
        },
    )?;
    register_state::<MapPortfolioEvmReadFailureState>(
        registry,
        qualification,
        StructuredStateCallbacks::Pure {
            apply: Arc::new(|_frame: StateFrame<'_, EvmReadFailure>| {
                ProposedStateOutcome::Success(PortfolioEvmReadFailureRoute::Propagate {
                    failure: PortfolioSnapshotFailure::InvalidCollection,
                })
            }),
        },
    )?;
    Ok(())
}

fn register_state<S>(
    registry: &mut ProgramRegistryBuilder,
    qualification: &PortfolioProcessQualification,
    callbacks: StructuredStateCallbacks<S>,
) -> mfm_certify::Result<()>
where
    S: State,
{
    let contract = state_contract::<S>().map_err(certification_error)?;
    registry.register_state::<S>(
        SecretFreeImplementationDescriptor {
            component_kind: StructuredComponentKind::State,
            semantic_contract_ref: contract.state_contract_ref,
            implementation_id: stable(format!(
                "mfm.portfolio.implementation/{}",
                S::semantic_state_id()
                    .map_err(certification_error)?
                    .as_str()
            ))
            .map_err(certification_error)?,
            executable_identity_ref: qualification.executable_identity_ref.clone(),
            qualification_artifact_ref: qualification.qualification_artifact_ref.clone(),
        },
        callbacks,
    )?;
    Ok(())
}

fn stable(value: impl AsRef<str>) -> mfm_program::Result<StableId> {
    StableId::new(value.as_ref())
        .map_err(|error| mfm_program::ProgramError::Authoring(error.to_string()))
}

fn certification_error(error: impl std::fmt::Display) -> mfm_certify::CertifyError {
    mfm_certify::CertifyError::Certification(error.to_string())
}
