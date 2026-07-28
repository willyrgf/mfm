//! Package-owned callback-surface identities for portfolio snapshot states.

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{ContentRef, DigestAlgorithm, FieldPath, FieldSegment, SemanticTypeId, StableId};
use mfm_program::{
    boundary_content_ref, mfm_value_contract, state_input_value_contract,
    unit_config_value_contract, CanonicalCodec, ProgramError, QualifiedInputContract,
    QualifiedOutputProjector, QualifiedOutputSourceContract, QualifiedProgramRegistryBuilder,
    QualifiedSettlementCodecs, QualifiedSourceContract, QualifiedSourcePathPattern,
    QualifiedSourcePathSegment, QualifiedSourceProjection, QualifiedStateContract,
    QualifiedStateRegistration, Result, State, StateExecution,
};
use mfm_spec::{
    CertifiedInputDestination, CertifiedOutputSlot, CertifiedSettlementContract,
    CertifiedStateExecution, ComponentImplementationDescriptor, ComponentKind,
};
use mfm_values::{MfmValue, PublicOutputDescriptor, RetainedValueContract, StateInput};
use serde::Serialize;

use crate::{
    state::{
        portfolio_contract_schema_id, portfolio_state_contract_canonical,
        portfolio_state_contract_schema_id, portfolio_state_contract_support_contract,
        portfolio_support_contract, report_projection_execution, snapshot_assembly_execution,
        validation_execution,
    },
    AssemblePortfolioSnapshotState, InvalidPortfolioSnapshotSelection, PortfolioConfig,
    PortfolioPublicOutputs, PortfolioReport, PortfolioReportProjectionInput,
    PortfolioRoutingManifest, PortfolioSnapshot, PortfolioSnapshotAssemblyInput,
    PortfolioSnapshotFailure, PortfolioSnapshotSelectionInput, PortfolioSnapshotSelector,
    ProjectPortfolioReportState, ValidatePortfolioSnapshotSelectionState,
    ValidatedPortfolioSnapshotSelection,
};

/// Exact version of every package-owned portfolio callback-surface descriptor.
pub const PORTFOLIO_STATE_CALLBACK_SURFACE_VERSION: &str =
    "mfm.portfolio.state-callback-surface.v1";

/// One exact package-owned portfolio state callback surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortfolioStateCallbackSurface {
    state_contract_ref: ContentRef,
    state_contract_canonical: PlainCanonicalJsonBytes,
    canonical: PlainCanonicalJsonBytes,
    content_ref: ContentRef,
}

impl PortfolioStateCallbackSurface {
    fn pure<S: State>(state_name: &'static str) -> Result<Self> {
        #[derive(Serialize)]
        struct Descriptor<'a> {
            callbacks: [&'static str; 1],
            external_operation_id: Option<&'static str>,
            state_contract_ref: &'a ContentRef,
            version: &'static str,
        }

        let state_contract_ref = S::state_contract_ref()?;
        let state_contract_canonical = portfolio_state_contract_canonical(state_name)?;
        let described_state_contract_ref = boundary_content_ref(
            portfolio_state_contract_schema_id()?,
            &state_contract_canonical,
        )?;
        if described_state_contract_ref != state_contract_ref {
            return Err(ProgramError::Registry(
                "portfolio state semantic contract differs from its package state identity"
                    .to_owned(),
            ));
        }
        let descriptor = Descriptor {
            callbacks: ["apply"],
            external_operation_id: None,
            state_contract_ref: &state_contract_ref,
            version: PORTFOLIO_STATE_CALLBACK_SURFACE_VERSION,
        };
        let encoded = serde_json::to_string(&descriptor)
            .map_err(|error| ProgramError::Codec(error.to_string()))?;
        let canonical = PlainCanonicalJsonBytes::from_json_str(&encoded)
            .map_err(|error| ProgramError::Codec(error.to_string()))?;
        let content_ref = boundary_content_ref(
            portfolio_contract_schema_id("mfm.portfolio.state-callback-surface")?,
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
        portfolio_state_contract_support_contract(role, evidence_contract_ref)
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
        portfolio_support_contract(
            "mfm.portfolio.state-callback-surface",
            "state-callback-surface",
            role,
            evidence_contract_ref,
        )
    }
}

/// Closed named callback-surface inventory for the three portfolio states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortfolioSnapshotCallbackSurfaces {
    validate_selection: PortfolioStateCallbackSurface,
    assemble_snapshot: PortfolioStateCallbackSurface,
    project_report: PortfolioStateCallbackSurface,
}

impl PortfolioSnapshotCallbackSurfaces {
    /// Returns the selection-validator callback surface.
    pub const fn validate_selection(&self) -> &PortfolioStateCallbackSurface {
        &self.validate_selection
    }

    /// Returns the snapshot-assembly callback surface.
    pub const fn assemble_snapshot(&self) -> &PortfolioStateCallbackSurface {
        &self.assemble_snapshot
    }

    /// Returns the report-projection callback surface.
    pub const fn project_report(&self) -> &PortfolioStateCallbackSurface {
        &self.project_report
    }

    /// Returns the exact three entries in graph lifecycle order.
    pub fn ordered(&self) -> [&PortfolioStateCallbackSurface; 3] {
        [
            &self.validate_selection,
            &self.assemble_snapshot,
            &self.project_report,
        ]
    }
}

/// Builds the exact named callback-surface inventory for portfolio snapshots.
pub fn portfolio_snapshot_callback_surfaces() -> Result<PortfolioSnapshotCallbackSurfaces> {
    Ok(PortfolioSnapshotCallbackSurfaces {
        validate_selection: PortfolioStateCallbackSurface::pure::<
            ValidatePortfolioSnapshotSelectionState,
        >("validate_snapshot_selection")?,
        assemble_snapshot: PortfolioStateCallbackSurface::pure::<AssemblePortfolioSnapshotState>(
            "assemble_snapshot",
        )?,
        project_report: PortfolioStateCallbackSurface::pure::<ProjectPortfolioReportState>(
            "project_report",
        )?,
    })
}

/// Exact named implementation descriptors for all three portfolio states.
pub struct PortfolioSnapshotStateImplementations {
    /// Selection-validator state implementation.
    pub validate_selection: ComponentImplementationDescriptor,
    /// Snapshot-assembly state implementation.
    pub assemble_snapshot: ComponentImplementationDescriptor,
    /// Report-projection state implementation.
    pub project_report: ComponentImplementationDescriptor,
}

impl PortfolioSnapshotStateImplementations {
    fn validate(&self, surfaces: &PortfolioSnapshotCallbackSurfaces) -> Result<ContentRef> {
        let pairs = [
            (&self.validate_selection, surfaces.validate_selection()),
            (&self.assemble_snapshot, surfaces.assemble_snapshot()),
            (&self.project_report, surfaces.project_report()),
        ];
        let qualification_ref = pairs[0].0.qualification_ref().clone();
        if pairs.iter().any(|(implementation, surface)| {
            implementation.component_kind() != ComponentKind::State
                || implementation.semantic_contract_ref() != surface.state_contract_ref()
                || implementation.callback_surface_ref() != surface.content_ref()
                || implementation.qualification_ref() != &qualification_ref
        }) {
            return Err(ProgramError::Registry(
                "portfolio state implementation inventory differs from its exact package callbacks"
                    .to_owned(),
            ));
        }
        Ok(qualification_ref)
    }
}

/// Product-owned identities required to qualify the three portfolio states.
pub struct PortfolioSnapshotStateArtifacts {
    object_evidence_contract_ref: ContentRef,
    unit_config_contract: RetainedValueContract,
    implementations: PortfolioSnapshotStateImplementations,
}

impl PortfolioSnapshotStateArtifacts {
    /// Constructs and validates the complete named portfolio state artifact set.
    pub fn new(
        object_evidence_contract_ref: ContentRef,
        unit_config_contract: RetainedValueContract,
        implementations: PortfolioSnapshotStateImplementations,
    ) -> Result<Self> {
        let expected_unit_config = unit_config_value_contract(
            unit_config_contract.role().clone(),
            object_evidence_contract_ref.clone(),
        )?;
        if unit_config_contract != expected_unit_config {
            return Err(ProgramError::Registry(
                "portfolio state artifacts require the exact retained framework UnitConfig contract"
                    .to_owned(),
            ));
        }
        let surfaces = portfolio_snapshot_callback_surfaces()?;
        implementations.validate(&surfaces)?;
        Ok(Self {
            object_evidence_contract_ref,
            unit_config_contract,
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

    /// Returns the exact named state implementation inventory.
    pub const fn implementations(&self) -> &PortfolioSnapshotStateImplementations {
        &self.implementations
    }

    /// Returns the one shared app-owned qualification identity.
    pub fn qualification_ref(&self) -> Result<&ContentRef> {
        let surfaces = portfolio_snapshot_callback_surfaces()?;
        self.implementations.validate(&surfaces)?;
        Ok(self.implementations.validate_selection.qualification_ref())
    }
}

/// Closed retained-value contract inventory for the portfolio snapshot product.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortfolioSnapshotValueContracts {
    selector: RetainedValueContract,
    portfolio: RetainedValueContract,
    routing_manifest: RetainedValueContract,
    validated_selection: RetainedValueContract,
    snapshot: RetainedValueContract,
    report: RetainedValueContract,
    public_outputs: RetainedValueContract,
    invalid_selection: RetainedValueContract,
    snapshot_failure: RetainedValueContract,
    selection_input: RetainedValueContract,
    assembly_input: RetainedValueContract,
    report_input: RetainedValueContract,
}

impl PortfolioSnapshotValueContracts {
    /// Returns the public selector contract.
    pub const fn selector(&self) -> &RetainedValueContract {
        &self.selector
    }

    /// Returns the separately configured portfolio contract.
    pub const fn portfolio(&self) -> &RetainedValueContract {
        &self.portfolio
    }

    /// Returns the qualified routing-manifest contract.
    pub const fn routing_manifest(&self) -> &RetainedValueContract {
        &self.routing_manifest
    }

    /// Returns the pure validator output contract.
    pub const fn validated_selection(&self) -> &RetainedValueContract {
        &self.validated_selection
    }

    /// Returns the canonical snapshot contract.
    pub const fn snapshot(&self) -> &RetainedValueContract {
        &self.snapshot
    }

    /// Returns the canonical report contract.
    pub const fn report(&self) -> &RetainedValueContract {
        &self.report
    }

    /// Returns the exact assembled public-output contract.
    pub const fn public_outputs(&self) -> &RetainedValueContract {
        &self.public_outputs
    }
}

/// Builds the domain-owned retained contract inventory used by entry and states.
pub fn portfolio_snapshot_value_contracts(
    object_evidence_contract_ref: ContentRef,
) -> Result<PortfolioSnapshotValueContracts> {
    Ok(PortfolioSnapshotValueContracts {
        selector: value_contract::<PortfolioSnapshotSelector>(
            "mfm.portfolio.value.snapshot-selector",
            object_evidence_contract_ref.clone(),
        )?,
        portfolio: value_contract::<PortfolioConfig>(
            "mfm.portfolio.value.configured-portfolio",
            object_evidence_contract_ref.clone(),
        )?,
        routing_manifest: value_contract::<PortfolioRoutingManifest>(
            "mfm.portfolio.value.routing-manifest",
            object_evidence_contract_ref.clone(),
        )?,
        validated_selection: value_contract::<ValidatedPortfolioSnapshotSelection>(
            "mfm.portfolio.value.validated-selection",
            object_evidence_contract_ref.clone(),
        )?,
        snapshot: value_contract::<PortfolioSnapshot>(
            "mfm.portfolio.value.snapshot",
            object_evidence_contract_ref.clone(),
        )?,
        report: value_contract::<PortfolioReport>(
            "mfm.portfolio.value.report",
            object_evidence_contract_ref.clone(),
        )?,
        public_outputs: RetainedValueContract::new(
            PortfolioPublicOutputs::public_schema_id()
                .map_err(|error| ProgramError::Codec(error.to_string()))?,
            semantic_type_id("public-outputs")?,
            stable_id("mfm.portfolio.value.public-outputs")?,
            "application/json",
            object_evidence_contract_ref.clone(),
        )
        .map_err(|error| ProgramError::Codec(error.to_string()))?,
        invalid_selection: value_contract::<InvalidPortfolioSnapshotSelection>(
            "mfm.portfolio.failure.invalid-selection",
            object_evidence_contract_ref.clone(),
        )?,
        snapshot_failure: value_contract::<PortfolioSnapshotFailure>(
            "mfm.portfolio.failure.snapshot",
            object_evidence_contract_ref.clone(),
        )?,
        selection_input: input_contract::<PortfolioSnapshotSelectionInput>(
            "snapshot-selection",
            object_evidence_contract_ref.clone(),
        )?,
        assembly_input: input_contract::<PortfolioSnapshotAssemblyInput>(
            "snapshot-assembly",
            object_evidence_contract_ref.clone(),
        )?,
        report_input: input_contract::<PortfolioReportProjectionInput>(
            "report-projection",
            object_evidence_contract_ref,
        )?,
    })
}

/// Closed typed registrations for all three portfolio snapshot states.
pub struct QualifiedPortfolioSnapshotStates {
    validate_selection: QualifiedStateRegistration<ValidatePortfolioSnapshotSelectionState>,
    assemble_snapshot: QualifiedStateRegistration<AssemblePortfolioSnapshotState>,
    project_report: QualifiedStateRegistration<ProjectPortfolioReportState>,
}

impl QualifiedPortfolioSnapshotStates {
    /// Registers the exact three-state inventory into the product registry.
    pub fn register_into(
        self,
        registry: &mut QualifiedProgramRegistryBuilder,
    ) -> Result<&mut QualifiedProgramRegistryBuilder> {
        registry
            .register_state(self.validate_selection)?
            .register_state(self.assemble_snapshot)?
            .register_state(self.project_report)
    }
}

/// Qualifies the exact three portfolio callbacks and retained contracts.
pub fn qualify_portfolio_snapshot_states(
    artifacts: PortfolioSnapshotStateArtifacts,
) -> Result<QualifiedPortfolioSnapshotStates> {
    let surfaces = portfolio_snapshot_callback_surfaces()?;
    artifacts.implementations.validate(&surfaces)?;
    let contracts =
        portfolio_snapshot_value_contracts(artifacts.object_evidence_contract_ref.clone())?;
    let evm_contracts =
        mfm_evm::evm_balance_collection_value_contracts(artifacts.object_evidence_contract_ref)?;
    let unit = artifacts.unit_config_contract;
    let implementations = artifacts.implementations;

    let selection_source = QualifiedSourceContract::new(
        contracts.validated_selection.clone(),
        vec![
            source_projection(
                vec![field("positions")?, any_index(), field("binding")?],
                evm_contracts.network_binding().clone(),
            )?,
            source_projection(
                vec![field("positions")?, any_index(), field("native_decimals")?],
                evm_contracts.native_decimals().clone(),
            )?,
            source_projection(
                vec![
                    field("positions")?,
                    any_index(),
                    field("sources")?,
                    any_index(),
                ],
                evm_contracts.balance_source().clone(),
            )?,
            source_projection(
                vec![
                    field("positions")?,
                    any_index(),
                    field("token_contracts")?,
                    any_index(),
                ],
                evm_contracts.token_contract().clone(),
            )?,
        ],
    )?;
    let validate_selection = qualify_pure_state(
        implementations.validate_selection,
        unit.clone(),
        contracts.selection_input.clone(),
        vec![
            ordinary_input("portfolio", contracts.portfolio.clone())?,
            ordinary_input("routing_manifest", contracts.routing_manifest.clone())?,
            ordinary_input("selector", contracts.selector.clone())?,
        ],
        selection_source,
        contracts.invalid_selection.clone(),
        validation_execution(),
    )?;
    let assemble_snapshot = qualify_pure_state(
        implementations.assemble_snapshot,
        unit.clone(),
        contracts.assembly_input.clone(),
        vec![
            ordinary_input("collections", evm_contracts.balance_collection().clone())?,
            ordinary_input("selection", contracts.validated_selection.clone())?,
        ],
        QualifiedSourceContract::new(contracts.snapshot.clone(), Vec::new())?,
        contracts.snapshot_failure.clone(),
        snapshot_assembly_execution(),
    )?;
    let project_report = qualify_pure_state(
        implementations.project_report,
        unit,
        contracts.report_input.clone(),
        vec![ordinary_input("snapshot", contracts.snapshot.clone())?],
        QualifiedSourceContract::new(contracts.report.clone(), Vec::new())?,
        contracts.snapshot_failure.clone(),
        report_projection_execution(),
    )?;

    Ok(QualifiedPortfolioSnapshotStates {
        validate_selection,
        assemble_snapshot,
        project_report,
    })
}

fn qualify_pure_state<S>(
    implementation: ComponentImplementationDescriptor,
    unit_config_contract: RetainedValueContract,
    input_contract: RetainedValueContract,
    input_destinations: Vec<QualifiedInputContract>,
    output_source: QualifiedSourceContract,
    failure_contract: RetainedValueContract,
    execution: StateExecution<S>,
) -> Result<QualifiedStateRegistration<S>>
where
    S: State,
    S::Output: MfmValue,
    S::Failure: MfmValue,
{
    let output_contract = output_source.root_contract().clone();
    let settlement = CertifiedSettlementContract::new(
        Some(failure_contract.clone()),
        vec![CertifiedOutputSlot::new(
            0,
            field_path("value")?,
            output_contract.clone(),
        )],
        Vec::new(),
    )?;
    let contract = QualifiedStateContract::new(
        S::state_contract_ref()?,
        unit_config_contract,
        None,
        input_contract,
        input_destinations,
        vec![QualifiedOutputSourceContract::new(0, output_source)],
        CertifiedStateExecution::Pure,
        settlement,
    )?;
    QualifiedStateRegistration::new(
        contract,
        implementation,
        execution,
        QualifiedSettlementCodecs::new(
            vec![QualifiedOutputProjector::new(
                0,
                field_path("value")?,
                CanonicalCodec::mfm_value(output_contract)?,
                identity::<S::Output>,
            )],
            Some(CanonicalCodec::mfm_value(failure_contract)?),
        )?,
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

fn source_projection(
    segments: Vec<QualifiedSourcePathSegment>,
    value_contract: RetainedValueContract,
) -> Result<QualifiedSourceProjection> {
    Ok(QualifiedSourceProjection::new(
        QualifiedSourcePathPattern::new(segments)?,
        value_contract,
    ))
}

fn field(value: &'static str) -> Result<QualifiedSourcePathSegment> {
    Ok(QualifiedSourcePathSegment::Field(
        FieldSegment::new(value).map_err(|error| ProgramError::Codec(error.to_string()))?,
    ))
}

const fn any_index() -> QualifiedSourcePathSegment {
    QualifiedSourcePathSegment::AnyIndex
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
        stable_id(format!("mfm.portfolio.input.{name}"))?,
        evidence_contract_ref,
    )
}

fn semantic_type_id(name: &str) -> Result<SemanticTypeId> {
    SemanticTypeId::new(
        "mfm.portfolio",
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        mfm_canonical::sha256_digest_bytes(format!("semantic:mfm.portfolio:{name}:1").as_bytes()),
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use mfm_ids::{ContentDigest, SchemaId};

    use super::*;

    fn content_ref(label: &'static [u8]) -> ContentRef {
        ContentRef::new(
            SchemaId::new(
                "mfm.test.portfolio-qualification",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                mfm_canonical::sha256_digest_bytes(b"portfolio qualification schema"),
            )
            .expect("schema"),
            ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                mfm_canonical::sha256_digest_bytes(label),
            ),
        )
        .expect("content ref")
    }

    fn qualified_states() -> QualifiedPortfolioSnapshotStates {
        let surfaces = portfolio_snapshot_callback_surfaces().expect("callback surfaces");
        let qualification = content_ref(b"qualification");
        let implementation = |surface: &PortfolioStateCallbackSurface| {
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
            StableId::new("mfm.test.portfolio.unit-config").expect("unit role"),
            evidence.clone(),
        )
        .expect("unit contract");
        qualify_portfolio_snapshot_states(
            PortfolioSnapshotStateArtifacts::new(
                evidence,
                unit,
                PortfolioSnapshotStateImplementations {
                    validate_selection: implementation(surfaces.validate_selection()),
                    assemble_snapshot: implementation(surfaces.assemble_snapshot()),
                    project_report: implementation(surfaces.project_report()),
                },
            )
            .expect("artifacts"),
        )
        .expect("qualified states")
    }

    fn pattern_string(pattern: &QualifiedSourcePathPattern) -> String {
        pattern
            .segments()
            .iter()
            .map(|segment| match segment {
                QualifiedSourcePathSegment::Field(field) => field.as_str(),
                QualifiedSourcePathSegment::AnyIndex => "*",
            })
            .collect::<Vec<_>>()
            .join(".")
    }

    #[test]
    fn callback_surface_inventory_is_exact_closed_and_unique() {
        let surfaces = portfolio_snapshot_callback_surfaces().expect("callback surfaces");
        let ordered = surfaces.ordered();
        let evidence_ref = content_ref(b"state contract evidence");
        assert_eq!(ordered.len(), 3);
        assert_eq!(
            ordered
                .iter()
                .map(|surface| surface.state_contract_ref())
                .collect::<BTreeSet<_>>()
                .len(),
            3
        );
        assert_eq!(
            ordered
                .iter()
                .map(|surface| surface.content_ref())
                .collect::<BTreeSet<_>>()
                .len(),
            3
        );
        for surface in ordered {
            let contract = surface
                .state_contract_support_contract(
                    StableId::new("mfm.test.portfolio.state-contract").expect("state role"),
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
            assert!(rendered.contains(PORTFOLIO_STATE_CALLBACK_SURFACE_VERSION));
            assert!(rendered.contains("\"external_operation_id\":null"));
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
    fn qualified_states_have_exact_named_inputs_and_validator_projections() {
        let states = qualified_states();
        let validator = states.validate_selection.contract();
        assert_eq!(
            validator
                .input_destinations()
                .iter()
                .map(|input| input.destination_field_path().as_str())
                .collect::<Vec<_>>(),
            ["portfolio", "routing_manifest", "selector"]
        );
        assert_eq!(
            validator.output_sources()[0]
                .source_contract()
                .projections()
                .iter()
                .map(|projection| pattern_string(projection.path()))
                .collect::<Vec<_>>(),
            [
                "positions.*.binding",
                "positions.*.native_decimals",
                "positions.*.sources.*",
                "positions.*.token_contracts.*",
            ]
        );
        assert_eq!(
            states
                .assemble_snapshot
                .contract()
                .input_destinations()
                .iter()
                .map(|input| input.destination_field_path().as_str())
                .collect::<Vec<_>>(),
            ["collections", "selection"]
        );
        assert_eq!(
            states
                .project_report
                .contract()
                .input_destinations()
                .iter()
                .map(|input| input.destination_field_path().as_str())
                .collect::<Vec<_>>(),
            ["snapshot"]
        );
        for contract in [
            states.validate_selection.contract(),
            states.assemble_snapshot.contract(),
            states.project_report.contract(),
        ] {
            assert_eq!(contract.execution(), &CertifiedStateExecution::Pure);
            assert!(contract.settlement_contract().fact_slots().is_empty());
            assert_eq!(
                contract.settlement_contract().output_slots()[0]
                    .field_path()
                    .as_str(),
                "value"
            );
        }
    }
}
