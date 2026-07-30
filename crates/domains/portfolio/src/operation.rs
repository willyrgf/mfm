//! Deterministic EVM-only portfolio snapshot graph authoring.

use mfm_ids::{ContentRef, FieldPath, StableId};
use mfm_journal::ValueRef;
use mfm_program::{AuthoredHandle, AuthoredProgramBuilder, Operation, StateBindings};
use mfm_program_derive::MfmValue;
use mfm_spec::{
    AuthoredSourceSelector, CanonicalAuthoredProgram, CanonicalJsonValue, EntryPointContract,
    EntryPointId, PlanningProfile,
};
use mfm_values::{MfmValue as _, PublicOutputDescriptor};
use serde::{Deserialize, Serialize};

use crate::state::compile_evm_collections_from_parts;
use crate::{
    AssemblePortfolioSnapshotState, PortfolioConfig, PortfolioId, PortfolioPublicOutputs,
    PortfolioReport, PortfolioRoutingManifest, PortfolioSnapshot, ProjectPortfolioReportState,
    ValidatePortfolioSnapshotSelectionState,
};

/// Sole published product entry-point id.
pub const PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID: &str = "mfm.portfolio/snapshot@1";
/// Stable operation id shared by all admitted snapshot runs.
pub const PORTFOLIO_SNAPSHOT_OPERATION_ID: &str = "mfm.portfolio/snapshot";

/// Public value-only selector for the configured portfolio snapshot target.
///
/// Admission resolves this target to a separately retained [`PortfolioConfig`].
/// The selector itself never carries portfolio configuration or routing data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "snapshot-selector",
    version = "1",
    schema = "mfm.portfolio.snapshot_selector"
)]
pub struct PortfolioSnapshotSelector {
    target: PortfolioId,
}

impl PortfolioSnapshotSelector {
    /// Creates a selector from one already checked configured-value target.
    pub const fn new(target: PortfolioId) -> Self {
        Self { target }
    }

    /// Returns the exact configured-value target.
    pub const fn target(&self) -> &PortfolioId {
        &self.target
    }
}

pub(crate) struct PortfolioSnapshotOperationOutputs {
    snapshot: AuthoredHandle<PortfolioSnapshot>,
    report: AuthoredHandle<PortfolioReport>,
}

pub(crate) struct PortfolioSnapshotOperation {
    topology: Vec<mfm_evm::EvmBalanceCollectionConfig>,
    unit_config_ref: ValueRef,
    routing_manifest_member_path: FieldPath,
}

impl PortfolioSnapshotOperation {
    pub(crate) fn new(
        portfolio: &PortfolioConfig,
        routing_manifest: &PortfolioRoutingManifest,
        unit_config_ref: ValueRef,
        routing_manifest_member_path: FieldPath,
    ) -> mfm_program::Result<Self> {
        let topology =
            compile_evm_collections_from_parts(portfolio, routing_manifest.evm_routing_bindings())
                .map_err(|error| mfm_program::ProgramError::Authoring(error.to_string()))?;
        if topology.is_empty() {
            return Err(mfm_program::ProgramError::Authoring(
                "portfolio snapshot topology must contain an EVM collection".to_owned(),
            ));
        }
        Ok(Self {
            topology,
            unit_config_ref,
            routing_manifest_member_path,
        })
    }

    fn bindings(&self) -> StateBindings {
        StateBindings::new(self.unit_config_ref.clone(), None)
    }
}

impl Operation for PortfolioSnapshotOperation {
    type Output = PortfolioSnapshotOperationOutputs;

    fn author(&self, builder: &mut AuthoredProgramBuilder) -> mfm_program::Result<Self::Output> {
        let validation = builder.state::<ValidatePortfolioSnapshotSelectionState>(
            stable_id("validate_selection")?,
            self.bindings(),
        )?;
        builder.bind_input_source(
            &validation,
            0,
            AuthoredSourceSelector::Config {
                source_field_path: None,
            },
        )?;
        builder.bind_input_source(
            &validation,
            1,
            AuthoredSourceSelector::QualifiedSupport {
                member_path: self.routing_manifest_member_path.clone(),
                source_field_path: None,
            },
        )?;
        builder.bind_input_source(
            &validation,
            2,
            AuthoredSourceSelector::RunAdmission {
                source_field_path: None,
            },
        )?;

        let mut collections = Vec::with_capacity(self.topology.len());
        for (position, topology) in self.topology.iter().enumerate() {
            let position_path = format!("positions.{position}");
            let binding = validation
                .output()
                .field::<mfm_evm::EvmNetworkBinding>(field_path(format!(
                    "{position_path}.binding"
                ))?);
            let native_decimals = validation
                .output()
                .field::<u8>(field_path(format!("{position_path}.native_decimals"))?);
            let sources = topology
                .sources()
                .iter()
                .enumerate()
                .map(|(source, _)| {
                    Ok(validation
                        .output()
                        .field::<mfm_evm::EvmBalanceSource>(field_path(format!(
                            "{position_path}.sources.{source}"
                        ))?))
                })
                .collect::<mfm_program::Result<Vec<_>>>()?;
            let token_contracts = topology
                .token_contracts()
                .iter()
                .enumerate()
                .map(|(token, _)| {
                    Ok(validation.output().field::<String>(field_path(format!(
                        "{position_path}.token_contracts.{token}"
                    ))?))
                })
                .collect::<mfm_program::Result<Vec<_>>>()?;
            let inputs = mfm_evm::EvmBalanceCollectionAuthoringInputs::new(
                self.unit_config_ref.clone(),
                binding,
                native_decimals,
                sources,
                token_contracts,
            );
            let operation = mfm_evm::EvmBalanceCollectionOperation::new(topology.clone(), inputs)?;
            let output = builder.child(indexed_key("evm_collection", position)?, |child| {
                operation.author(child)
            })?;
            collections.push(output.collection);
        }

        let snapshot = builder.state::<AssemblePortfolioSnapshotState>(
            stable_id("assemble_snapshot")?,
            self.bindings(),
        )?;
        for collection in &collections {
            builder.connect_to(collection, &snapshot, 0)?;
        }
        builder.connect_to(validation.output(), &snapshot, 1)?;

        let report = builder
            .state::<ProjectPortfolioReportState>(stable_id("project_report")?, self.bindings())?;
        builder.connect_to(snapshot.output(), &report, 0)?;
        builder.required_success(report.output())?;

        Ok(PortfolioSnapshotOperationOutputs {
            snapshot: snapshot.into_output(),
            report: report.into_output(),
        })
    }
}

pub(crate) fn portfolio_snapshot_authored_program(
    portfolio: &PortfolioConfig,
    routing_manifest: &PortfolioRoutingManifest,
    unit_config_ref: ValueRef,
    routing_manifest_member_path: FieldPath,
) -> mfm_program::Result<CanonicalAuthoredProgram> {
    let operation = PortfolioSnapshotOperation::new(
        portfolio,
        routing_manifest,
        unit_config_ref,
        routing_manifest_member_path,
    )?;
    let mut builder = AuthoredProgramBuilder::new(stable_id(PORTFOLIO_SNAPSHOT_OPERATION_ID)?);
    let output = operation.author(&mut builder)?;
    builder.public_output(stable_id("snapshot")?, &output.snapshot)?;
    builder.public_output(stable_id("report")?, &output.report)?;
    builder.finish()
}

/// Builds the exact empty-policy planning profile used by the sole entry point.
pub fn portfolio_snapshot_planning_profile(
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

/// Builds the sole published entry-point contract.
pub fn portfolio_snapshot_entry_point_contract(
    planner_contract_ref: ContentRef,
    planner_implementation_ref: ContentRef,
) -> mfm_spec::Result<EntryPointContract> {
    let profile =
        portfolio_snapshot_planning_profile(planner_contract_ref, planner_implementation_ref)?;
    EntryPointContract::new(
        EntryPointId::new(PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID)
            .map_err(|error| mfm_spec::SpecError::Contract(error.to_string()))?,
        spec_stable_id(PORTFOLIO_SNAPSHOT_OPERATION_ID)?,
        profile,
        PortfolioSnapshotSelector::schema_id()
            .map_err(|error| mfm_spec::SpecError::Contract(error.to_string()))?,
        PortfolioPublicOutputs::public_schema_id()
            .map_err(|error| mfm_spec::SpecError::Contract(error.to_string()))?,
    )
}

fn indexed_key(prefix: &str, index: usize) -> mfm_program::Result<StableId> {
    stable_id(format!("{prefix}/{index:04}"))
}

fn field_path(value: impl AsRef<str>) -> mfm_program::Result<FieldPath> {
    FieldPath::new(value).map_err(|error| mfm_program::ProgramError::Authoring(error.to_string()))
}

fn stable_id(value: impl AsRef<str>) -> mfm_program::Result<StableId> {
    StableId::new(value).map_err(|error| mfm_program::ProgramError::Authoring(error.to_string()))
}

fn spec_stable_id(value: &'static str) -> mfm_spec::Result<StableId> {
    StableId::new(value).map_err(|error| mfm_spec::SpecError::Contract(error.to_string()))
}

#[cfg(test)]
#[path = "operation_tests.rs"]
mod tests;
