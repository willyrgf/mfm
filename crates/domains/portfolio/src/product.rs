//! Qualified product registration for the sole portfolio snapshot entry point.

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{ContentRef, FieldPath};
use mfm_program::{
    decode_boundary, unit_config_value_contract, ProgramError, QualifiedAuthoringInputs,
    QualifiedAuthoringRootRequirement, QualifiedEntryPointInputContract,
    QualifiedEntryPointRegistration, QualifiedSourceContract, Result, UnitConfig,
};
use mfm_values::RetainedValueContract;

use crate::{
    operation::portfolio_snapshot_authored_program, portfolio_snapshot_entry_point_contract,
    portfolio_snapshot_value_contracts, PortfolioConfig, PortfolioRoutingManifest,
};

/// Fixed support member containing the retained framework unit configuration.
pub const PORTFOLIO_SNAPSHOT_UNIT_CONFIG_MEMBER_PATH: &str = "framework/unit-config";
/// Fixed support member containing the product-qualified routing manifest.
pub const PORTFOLIO_SNAPSHOT_ROUTING_MANIFEST_MEMBER_PATH: &str = "product/portfolio-routing";

/// Product-owned identities required to register the snapshot entry point.
pub struct PortfolioSnapshotEntryPointArtifacts {
    planner_contract_ref: ContentRef,
    planner_implementation_ref: ContentRef,
    object_evidence_contract_ref: ContentRef,
    unit_config_contract: RetainedValueContract,
}

impl PortfolioSnapshotEntryPointArtifacts {
    /// Constructs and validates the complete entry-point artifact set.
    pub fn new(
        planner_contract_ref: ContentRef,
        planner_implementation_ref: ContentRef,
        object_evidence_contract_ref: ContentRef,
        unit_config_contract: RetainedValueContract,
    ) -> Result<Self> {
        let expected_unit = unit_config_value_contract(
            unit_config_contract.role().clone(),
            object_evidence_contract_ref.clone(),
        )?;
        if unit_config_contract != expected_unit {
            return Err(ProgramError::Registry(
                "portfolio entry point requires the exact retained framework UnitConfig contract"
                    .to_owned(),
            ));
        }
        Ok(Self {
            planner_contract_ref,
            planner_implementation_ref,
            object_evidence_contract_ref,
            unit_config_contract,
        })
    }

    /// Returns the shared reviewed object-evidence contract.
    pub const fn object_evidence_contract_ref(&self) -> &ContentRef {
        &self.object_evidence_contract_ref
    }

    /// Returns the retained framework unit configuration contract.
    pub const fn unit_config_contract(&self) -> &RetainedValueContract {
        &self.unit_config_contract
    }
}

/// Returns the one fixed unit-configuration support member path.
pub fn portfolio_snapshot_unit_config_member_path() -> Result<FieldPath> {
    field_path(PORTFOLIO_SNAPSHOT_UNIT_CONFIG_MEMBER_PATH)
}

/// Returns the one fixed routing-manifest support member path.
pub fn portfolio_snapshot_routing_manifest_member_path() -> Result<FieldPath> {
    field_path(PORTFOLIO_SNAPSHOT_ROUTING_MANIFEST_MEMBER_PATH)
}

/// Builds the sole published snapshot entry registration and deterministic author.
pub fn portfolio_snapshot_entry_point_registration(
    artifacts: PortfolioSnapshotEntryPointArtifacts,
) -> Result<QualifiedEntryPointRegistration> {
    let contracts = portfolio_snapshot_value_contracts(artifacts.object_evidence_contract_ref)?;
    let entry_point = portfolio_snapshot_entry_point_contract(
        artifacts.planner_contract_ref,
        artifacts.planner_implementation_ref,
    )?;
    let input_contract = QualifiedEntryPointInputContract::new(
        QualifiedSourceContract::new(contracts.selector().clone(), Vec::new())?,
        QualifiedSourceContract::new(contracts.portfolio().clone(), Vec::new())?,
        None,
        None,
        None,
        Vec::new(),
    )?;
    let roots = vec![
        QualifiedAuthoringRootRequirement::new(
            portfolio_snapshot_unit_config_member_path()?,
            QualifiedSourceContract::new(artifacts.unit_config_contract, Vec::new())?,
        ),
        QualifiedAuthoringRootRequirement::new(
            portfolio_snapshot_routing_manifest_member_path()?,
            QualifiedSourceContract::new(contracts.routing_manifest().clone(), Vec::new())?,
        ),
    ];
    QualifiedEntryPointRegistration::new(
        entry_point,
        input_contract,
        contracts.public_outputs().clone(),
        roots,
        author_portfolio_snapshot,
    )
}

fn author_portfolio_snapshot(
    inputs: &QualifiedAuthoringInputs<'_>,
) -> Result<mfm_spec::CanonicalAuthoredProgram> {
    let portfolio = decode_boundary::<PortfolioConfig>(inputs.configured().canonical())?;
    let unit_path = portfolio_snapshot_unit_config_member_path()?;
    let unit = inputs.root(&unit_path).ok_or_else(|| {
        ProgramError::Registry("portfolio authoring input omitted framework UnitConfig".to_owned())
    })?;
    let unit_canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(unit.bytes())
        .map_err(|error| ProgramError::Codec(error.to_string()))?;
    decode_boundary::<UnitConfig>(&unit_canonical)?;

    let routing_path = portfolio_snapshot_routing_manifest_member_path()?;
    let routing = inputs.root(&routing_path).ok_or_else(|| {
        ProgramError::Registry(
            "portfolio authoring input omitted qualified routing manifest".to_owned(),
        )
    })?;
    let routing_canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(routing.bytes())
        .map_err(|error| ProgramError::Codec(error.to_string()))?;
    let routing_manifest = decode_boundary::<PortfolioRoutingManifest>(&routing_canonical)?;

    portfolio_snapshot_authored_program(
        &portfolio,
        &routing_manifest,
        unit.value_ref().clone(),
        routing_path,
    )
}

fn field_path(value: &'static str) -> Result<FieldPath> {
    FieldPath::new(value).map_err(|error| ProgramError::Codec(error.to_string()))
}
