#![warn(missing_docs)]
//! Typed portfolio tracker workflow operation.
//!
//! The portfolio tracker workflow is authored through `mfm-program` and lowers to certified typed
//! state programs. This crate exposes no legacy dynamic `PlannedOp`, `PortKey`, context-key, or
//! generic IO surface.
//!
//! # Examples
//!
//! ```rust
//! use mfm_op_portfolio_tracker::{portfolio_program_draft, PortfolioWorkflowConfig};
//!
//! # fn demo(config: PortfolioWorkflowConfig) -> mfm_program::Result<()> {
//! let draft = portfolio_program_draft(config)?;
//! assert!(!draft.state_nodes().is_empty());
//! # Ok(())
//! # }
//! ```

use std::collections::{BTreeMap, BTreeSet};

use mfm_certify::{certify_program_draft, CertifiedTypedSpec};
use mfm_ids::{
    ArtifactId, ContentDigest, DigestAlgorithm, OperationKind, OperationVersion, SchemaId,
};
use mfm_portfolio_model::domain_key::{
    ObservationBatchDomainKey, ReportDomainKey, SourceDomainKey, SubjectDomainKey,
    ValuationDomainKey, ViewDomainKey,
};
use mfm_portfolio_model::portfolio::{NetworkConfig, PortfolioConfig};
use mfm_portfolio_model::symbol::SymbolConfig;
use mfm_portfolio_model::wallet::WalletConfig;
use mfm_program::{
    build_root_with_registries, DomainKeyedNonEmptyHandles, Operation, OperationExpansion,
    OperationKey, OperationRegistryBuilder, PublicOutputKey, RootBuilder, ScopeKey, StateKey,
    StateRegistryBuilder,
};
use mfm_spec::v1 as spec;
pub use mfm_state_portfolio::{
    balance_reader_kind, observation_batch_id, portfolio_adapter_kind, portfolio_adapter_version,
    AssembleSnapshotConfig, AssembleSnapshotInput, AssembleSnapshotInputHandles,
    AssembleSnapshotState, MergeObservationsConfig, MergeObservationsState, ObservationBatch,
    ObserveBatchConfig, ObserveBatchInput, ObserveBatchInputHandles, ObserveBatchState,
    PinViewsConfig, PinViewsState, PortfolioOperationOutputs, PortfolioPublicOutputs,
    PortfolioWorkflowConfig, PrepareSourcesConfig, PrepareSourcesState, ProjectReportConfig,
    ProjectReportInput, ProjectReportInputHandles, ProjectReportState, ResolveSubjectsConfig,
    ResolveSubjectsState, ResolveValuationsConfig, ResolveValuationsState,
};

const PORTFOLIO_OPERATION_KIND_NAME: &str = "tracker_workflow";
const PORTFOLIO_OPERATION_VERSION: &str = "mfm.portfolio.operation.tracker_workflow.v1";
const ROOT_SCOPE: &str = "portfolio";
const OP_KEY: &str = "portfolio_tracker";
const PUBLIC_OUTPUT_KEY: &str = "portfolio";

/// Typed portfolio tracker workflow operation.
pub struct PortfolioTrackerWorkflowOperation;

impl Operation for PortfolioTrackerWorkflowOperation {
    type Config = PortfolioWorkflowConfig;
    type Input<'program, 'scope> = ();
    type Output<'program, 'scope> = PortfolioOperationOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        OperationKind::new(
            "mfm.portfolio",
            PORTFOLIO_OPERATION_KIND_NAME,
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.portfolio.operation:tracker_workflow"),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        OperationVersion::new(PORTFOLIO_OPERATION_VERSION)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.portfolio.tracker_workflow"
    }

    fn expand<'program, 'scope>(
        &self,
        config: mfm_program::ValidatedConfig<Self::Config>,
        _input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let config = config.into_inner();
        let portfolio = config.portfolio().clone().normalized();
        let valuation_source_registry = config.valuation_source_registry().clone().normalized();
        let networks_by_id = networks_by_id(&portfolio.networks)?;

        let source_key = SourceDomainKey::new("portfolio_sources")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
        let subject_key = SubjectDomainKey::new("portfolio_subjects")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
        let view_key = ViewDomainKey::new("portfolio_views")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
        let valuation_key = ValuationDomainKey::new("portfolio_valuations")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
        let report_key = ReportDomainKey::new("portfolio_report")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;

        let prepared = builder.state_with_domain_keys::<PrepareSourcesState, _, _>(
            StateKey::new("prepare_sources")?,
            PrepareSourcesConfig::new(portfolio.networks.clone())
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
            (),
            vec![source_key],
        )?;
        let subjects = builder.state_with_domain_keys::<ResolveSubjectsState, _, _>(
            StateKey::new("resolve_subjects")?,
            ResolveSubjectsConfig::new(portfolio.wallets.clone())
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
            prepared.clone(),
            vec![subject_key],
        )?;
        let views = builder.state_with_domain_keys::<PinViewsState, _, _>(
            StateKey::new("pin_views")?,
            PinViewsConfig::new(portfolio.networks.clone())
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
            prepared,
            vec![view_key],
        )?;
        let valuations = builder.state_with_domain_keys::<ResolveValuationsState, _, _>(
            StateKey::new("resolve_valuations")?,
            ResolveValuationsConfig::new(
                portfolio.symbol_configs.clone(),
                valuation_source_registry,
            )
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
            views.clone(),
            vec![valuation_key],
        )?;

        let mut observation_handles = Vec::new();
        let mut seen_observation_keys = BTreeSet::new();
        for (wallet, symbol) in observation_targets(&portfolio)? {
            let network = networks_by_id
                .get(symbol.network_id.as_str())
                .copied()
                .ok_or_else(|| {
                    mfm_program::PlanError::Key(format!(
                        "missing network `{}` for symbol `{}`",
                        symbol.network_id, symbol.symbol_id
                    ))
                })?;
            let batch_key = observation_batch_id(&wallet.wallet_id, &symbol.symbol_id);
            if !seen_observation_keys.insert(batch_key.clone()) {
                return Err(mfm_program::PlanError::DuplicateDomainKey(batch_key));
            }
            let observation_key = ObservationBatchDomainKey::new(batch_key.clone())
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
            let handle = builder.state_with_domain_keys::<ObserveBatchState, _, _>(
                StateKey::new(format!("observe/{batch_key}"))?,
                ObserveBatchConfig::new(wallet.clone(), symbol.clone(), network.clone())
                    .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
                ObserveBatchInputHandles {
                    subjects: subjects.clone(),
                    views: views.clone(),
                    valuations: valuations.clone(),
                },
                vec![observation_key.clone()],
            )?;
            observation_handles.push((observation_key, handle));
        }

        let observations = builder.state::<MergeObservationsState, _>(
            StateKey::new("merge_observations")?,
            MergeObservationsConfig::new(),
            DomainKeyedNonEmptyHandles::<ObservationBatchDomainKey, ObservationBatch>::new(
                observation_handles,
            )?,
        )?;
        let snapshot = builder.state::<AssembleSnapshotState, _>(
            StateKey::new("assemble_snapshot")?,
            AssembleSnapshotConfig::new(2, portfolio.clone())
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
            AssembleSnapshotInputHandles {
                subjects,
                views,
                observations,
            },
        )?;
        let report = builder.state_with_domain_keys::<ProjectReportState, _, _>(
            StateKey::new("project_report")?,
            ProjectReportConfig::new(2)
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
            ProjectReportInputHandles {
                snapshot: snapshot.clone(),
            },
            vec![report_key],
        )?;

        Ok(PortfolioOperationOutputs { snapshot, report })
    }
}

/// Builds the portfolio state registry used for authoring and certification.
pub fn portfolio_state_registry() -> mfm_program::Result<mfm_program::StateRegistrySnapshot> {
    let mut states = StateRegistryBuilder::new();
    states.register::<PrepareSourcesState>()?;
    states.register::<ResolveSubjectsState>()?;
    states.register::<PinViewsState>()?;
    states.register::<ResolveValuationsState>()?;
    states.register::<ObserveBatchState>()?;
    states.register::<MergeObservationsState>()?;
    states.register::<AssembleSnapshotState>()?;
    states.register::<ProjectReportState>()?;
    Ok(states.into_snapshot())
}

/// Builds the portfolio operation registry used for authoring and certification.
pub fn portfolio_operation_registry() -> mfm_program::Result<mfm_program::OperationRegistrySnapshot>
{
    let mut operations = OperationRegistryBuilder::new();
    operations.register::<PortfolioTrackerWorkflowOperation>()?;
    Ok(operations.into_snapshot())
}

/// Adds portfolio workflow descriptors to a trusted certification registry.
pub fn register_portfolio_certification_descriptors(
    registry: &mut mfm_certify::CertificationRegistry,
) -> mfm_certify::Result<()> {
    let mut states = StateRegistryBuilder::new();
    registry.register_state(
        &states
            .register::<PrepareSourcesState>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    registry.register_state(
        &states
            .register::<ResolveSubjectsState>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    registry.register_state(
        &states
            .register::<PinViewsState>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    registry.register_state(
        &states
            .register::<ResolveValuationsState>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    registry.register_state(
        &states
            .register::<ObserveBatchState>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    registry.register_state(
        &states
            .register::<MergeObservationsState>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    registry.register_state(
        &states
            .register::<AssembleSnapshotState>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    registry.register_state(
        &states
            .register::<ProjectReportState>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    let mut operations = OperationRegistryBuilder::new();
    registry.register_operation(
        &operations
            .register::<PortfolioTrackerWorkflowOperation>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    Ok(())
}

/// Builds a typed portfolio program draft.
pub fn portfolio_program_draft(
    config: PortfolioWorkflowConfig,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    build_root_with_registries(
        ScopeKey::new(ROOT_SCOPE)?,
        portfolio_state_registry()?,
        portfolio_operation_registry()?,
        |root: &mut RootBuilder<'_, '_>| {
            let result = root.scope().call::<PortfolioTrackerWorkflowOperation, _>(
                OperationKey::new(OP_KEY)?,
                PortfolioTrackerWorkflowOperation,
                config,
                (),
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new(PUBLIC_OUTPUT_KEY)?,
                &PortfolioPublicOutputs {
                    snapshot: result.snapshot,
                    report: result.report,
                },
            )
        },
    )
}

/// Builds and certifies the typed portfolio program.
pub fn certified_portfolio_spec(
    config: PortfolioWorkflowConfig,
) -> mfm_certify::Result<CertifiedTypedSpec> {
    let draft = portfolio_program_draft(config)
        .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?;
    certify_program_draft(&draft)
}

/// Fully compiled portfolio snapshot launch program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledPortfolioSnapshotProgram {
    /// Certifier-backed typed spec authority.
    pub certified_spec: CertifiedTypedSpec,
    /// Public output schema id exposed by the certified spec.
    pub public_schema_id: SchemaId,
    /// Config artifacts required to launch the certified spec.
    pub config_artifacts: Vec<PortfolioConfigArtifact>,
}

/// Error returned while compiling a portfolio snapshot program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortfolioSnapshotCompileError {
    /// Program drafting or config artifact selection failed.
    Plan(mfm_program::PlanError),
    /// Certification failed.
    Certify(mfm_certify::CertifyError),
}

impl std::fmt::Display for PortfolioSnapshotCompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Plan(error) => write!(f, "portfolio snapshot planning failed: {error}"),
            Self::Certify(error) => write!(f, "portfolio snapshot certification failed: {error}"),
        }
    }
}

impl std::error::Error for PortfolioSnapshotCompileError {}

impl From<mfm_program::PlanError> for PortfolioSnapshotCompileError {
    fn from(error: mfm_program::PlanError) -> Self {
        Self::Plan(error)
    }
}

impl From<mfm_certify::CertifyError> for PortfolioSnapshotCompileError {
    fn from(error: mfm_certify::CertifyError) -> Self {
        Self::Certify(error)
    }
}

/// Builds, certifies, and gathers launch config artifacts for a portfolio snapshot program.
pub fn compile_portfolio_snapshot_program(
    config: PortfolioWorkflowConfig,
) -> Result<CompiledPortfolioSnapshotProgram, PortfolioSnapshotCompileError> {
    let draft = portfolio_program_draft(config)?;
    let certified_spec = certify_program_draft(&draft)?;
    let public_schema_id = certified_spec
        .envelope()
        .spec
        .public_outputs
        .public_schema_id
        .clone();
    let config_artifacts =
        portfolio_config_artifacts_for_spec(&draft, &certified_spec.envelope().spec)?;
    Ok(CompiledPortfolioSnapshotProgram {
        certified_spec,
        public_schema_id,
        config_artifacts,
    })
}

/// Canonical bytes for one config artifact required by a typed portfolio spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortfolioConfigArtifact {
    /// Content-addressed artifact id.
    pub artifact_id: ArtifactId,
    /// Canonical content digest.
    pub digest: ContentDigest,
    /// Canonical byte length.
    pub byte_len: u64,
    /// Canonical JSON bytes.
    pub bytes: Vec<u8>,
    /// Config schema id.
    pub schema_id: SchemaId,
    /// Config media type.
    pub media_type: spec::MediaType,
}

/// Returns all author-emitted config artifacts from a portfolio draft.
pub fn portfolio_draft_config_artifacts(
    draft: &mfm_program::TypedProgramDraft,
) -> mfm_program::Result<Vec<PortfolioConfigArtifact>> {
    let media_type = spec::MediaType::new("application/json")
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
    Ok(draft
        .state_nodes()
        .iter()
        .map(|node| &node.config)
        .chain(draft.operation_lineage().iter().map(|frame| &frame.config))
        .map(|config| {
            config_artifact(
                config.canonical_json.clone(),
                config.schema_id.clone(),
                media_type.clone(),
            )
        })
        .collect())
}

/// Returns framework config artifacts introduced during certification.
pub fn portfolio_framework_config_artifacts(
    typed_spec: &spec::TypedExecutionSpec,
) -> mfm_program::Result<Vec<PortfolioConfigArtifact>> {
    let mut artifacts = Vec::new();
    for node in &typed_spec.nodes {
        let Some(framework) = &node.framework else {
            continue;
        };
        let bytes = spec::framework_config_canonical_json(framework.config_kind(), &node.node_id)
            .map_err(|error| mfm_program::PlanError::Canonical(error.to_string()))?;
        if bytes.content_digest() != node.config_ref.digest
            || bytes.as_bytes().len() as u64 != node.config_ref.byte_len
        {
            return Err(mfm_program::PlanError::Canonical(format!(
                "framework config helper did not match certified config ref for node {}",
                node.node_id
            )));
        }
        artifacts.push(config_artifact(
            bytes,
            node.config_ref.schema_id.clone(),
            node.config_ref.media_type.clone(),
        ));
    }
    Ok(artifacts)
}

/// Returns only config artifacts required by the certified spec, with metadata matching
/// `config_refs`.
pub fn portfolio_config_artifacts_for_spec(
    draft: &mfm_program::TypedProgramDraft,
    typed_spec: &spec::TypedExecutionSpec,
) -> mfm_program::Result<Vec<PortfolioConfigArtifact>> {
    let mut candidates = portfolio_draft_config_artifacts(draft)?;
    candidates.extend(portfolio_framework_config_artifacts(typed_spec)?);
    let mut selected = Vec::new();
    let mut seen_artifact_ids = BTreeMap::new();
    for config_ref in &typed_spec.config_refs {
        if let Some(previous_schema) =
            seen_artifact_ids.insert(config_ref.artifact_id.clone(), config_ref.schema_id.clone())
        {
            if previous_schema != config_ref.schema_id {
                return Err(mfm_program::PlanError::Key(format!(
                    "certified spec contains duplicate config artifact {} with schemas {} and {}",
                    config_ref.artifact_id, previous_schema, config_ref.schema_id
                )));
            }
            continue;
        }
        let artifact = candidates
            .iter()
            .find(|artifact| {
                artifact.artifact_id == config_ref.artifact_id
                    && artifact.digest == config_ref.digest
                    && artifact.byte_len == config_ref.byte_len
                    && artifact.media_type == config_ref.media_type
                    && artifact.schema_id == config_ref.schema_id
            })
            .cloned()
            .ok_or_else(|| {
                mfm_program::PlanError::Key(format!(
                    "missing typed config artifact for {}",
                    config_ref.artifact_id
                ))
            })?;
        selected.push(artifact);
    }
    Ok(selected)
}

fn config_artifact(
    bytes: mfm_canonical::PlainCanonicalJsonBytes,
    schema_id: SchemaId,
    media_type: spec::MediaType,
) -> PortfolioConfigArtifact {
    let digest = bytes.content_digest();
    let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    let byte_len = bytes.as_bytes().len() as u64;
    PortfolioConfigArtifact {
        artifact_id,
        digest,
        byte_len,
        bytes: bytes.to_vec(),
        schema_id,
        media_type,
    }
}

fn networks_by_id(
    networks: &[NetworkConfig],
) -> mfm_program::Result<BTreeMap<&str, &NetworkConfig>> {
    let mut by_id = BTreeMap::new();
    for network in networks {
        if by_id.insert(network.network_id.as_str(), network).is_some() {
            return Err(mfm_program::PlanError::DuplicateDomainKey(
                network.network_id.clone(),
            ));
        }
    }
    Ok(by_id)
}

fn symbols_by_id(symbols: &[SymbolConfig]) -> mfm_program::Result<BTreeMap<&str, &SymbolConfig>> {
    let mut by_id = BTreeMap::new();
    for symbol in symbols {
        if by_id.insert(symbol.symbol_id.as_str(), symbol).is_some() {
            return Err(mfm_program::PlanError::DuplicateDomainKey(
                symbol.symbol_id.clone(),
            ));
        }
    }
    Ok(by_id)
}

fn observation_targets(
    portfolio: &PortfolioConfig,
) -> mfm_program::Result<Vec<(&WalletConfig, &SymbolConfig)>> {
    let symbols = symbols_by_id(&portfolio.symbol_configs)?;
    let mut targets = Vec::new();
    for wallet in &portfolio.wallets {
        for symbol_id in &wallet.symbol_ids {
            let symbol = symbols.get(symbol_id.as_str()).copied().ok_or_else(|| {
                mfm_program::PlanError::Key(format!(
                    "wallet `{}` referenced unknown symbol `{symbol_id}`",
                    wallet.wallet_id
                ))
            })?;
            targets.push((wallet, symbol));
        }
    }
    targets.sort_by(|left, right| {
        (
            left.0.wallet_id.as_str(),
            left.1.network_id.as_str(),
            left.1.symbol_id.as_str(),
        )
            .cmp(&(
                right.0.wallet_id.as_str(),
                right.1.network_id.as_str(),
                right.1.symbol_id.as_str(),
            ))
    });
    Ok(targets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_portfolio_model::portfolio::NetworkFamilyConfig;
    use mfm_portfolio_model::symbol::{
        BalanceReaderConfig, QuoteCode, QuoteValuationConfig, SymbolKind, SymbolRole,
        SymbolValuationConfig, ValuationReaderConfig, ValuationSourceRegistry,
    };
    use mfm_portfolio_model::wallet::{WalletImplementationConfig, WalletSubjectKind};
    use serde_json::Value;

    #[test]
    fn portfolio_program_lowers_to_typed_state_contracts() {
        let draft = portfolio_program_draft(sample_workflow_config()).expect("draft");
        assert_eq!(draft.state_nodes().len(), 8);
        assert_eq!(
            draft
                .state_nodes()
                .iter()
                .filter(|node| !node.output_domain_keys.is_empty())
                .count(),
            6,
            "source, subject, view, valuation, observation batch, and report outputs need domain-key lineage"
        );
        assert!(
            draft
                .state_nodes()
                .iter()
                .all(|node| !node.state_descriptor_name.contains("DynContext")),
            "portfolio descriptors must not expose dynamic context"
        );

        assert!(
            draft
                .state_nodes()
                .iter()
                .all(|node| node.key.as_str() != "publish_snapshot"),
            "portfolio snapshot graph must not include the removed publish wrapper"
        );

        let certified = certify_program_draft(&draft).expect("certified portfolio spec");
        assert!(
            certified
                .envelope()
                .spec
                .nodes
                .iter()
                .all(|node| !node.node_id.as_str().contains("publish_snapshot")),
            "certified portfolio spec must not include the removed publish wrapper"
        );
        assert_eq!(
            certified
                .envelope()
                .spec
                .value_lineages
                .iter()
                .filter(|lineage| !lineage.domain_keys.is_empty())
                .count(),
            6,
            "certified portfolio spec must retain value-lineage domain keys"
        );
        certified.envelope().verify_hash().expect("hash verifies");
        let spec_json = certified
            .envelope()
            .spec
            .canonical_json()
            .expect("canonical spec");
        let spec_value: Value = serde_json::from_slice(spec_json.as_bytes()).expect("spec json");
        assert!(spec_value
            .to_string()
            .contains("\"ordering\":\"stable_domain_key\""));
    }

    #[test]
    fn duplicate_observation_domain_key_is_rejected() {
        let mut portfolio = sample_portfolio_config();
        portfolio.wallets[0]
            .symbol_ids
            .push("eth.native.ethereum-mainnet".to_owned());
        let config = PortfolioWorkflowConfig::new(portfolio, sample_valuation_source_registry())
            .expect("workflow config");
        let err = portfolio_program_draft(config).expect_err("duplicate domain key");
        assert!(matches!(err, mfm_program::PlanError::DuplicateDomainKey(_)));
    }

    fn sample_workflow_config() -> PortfolioWorkflowConfig {
        PortfolioWorkflowConfig::new(
            sample_portfolio_config(),
            sample_valuation_source_registry(),
        )
        .expect("workflow config")
    }

    fn sample_valuation_source_registry() -> ValuationSourceRegistry {
        ValuationSourceRegistry {
            sources: Vec::new(),
        }
    }

    fn sample_portfolio_config() -> PortfolioConfig {
        PortfolioConfig {
            portfolio_id: "portfolio_main".to_owned(),
            quote_codes: vec![QuoteCode::Usd],
            networks: vec![NetworkConfig {
                network_id: "ethereum-mainnet".to_owned(),
                family: NetworkFamilyConfig::Evm,
                chain_id: Some(1),
                control_scope: "shared".to_owned(),
                metadata: BTreeMap::new(),
            }],
            wallets: vec![WalletConfig {
                wallet_id: "wallet_main".to_owned(),
                address: "0x000000000000000000000000000000000000dead".to_owned(),
                subject_kind: WalletSubjectKind::EvmAddress,
                network_id: "ethereum-mainnet".to_owned(),
                implementation: WalletImplementationConfig::AddressOnly {},
                symbol_ids: vec!["eth.native.ethereum-mainnet".to_owned()],
                metadata: BTreeMap::new(),
            }],
            symbol_configs: vec![SymbolConfig {
                symbol_id: "eth.native.ethereum-mainnet".to_owned(),
                display_symbol: Some("ETH".to_owned()),
                kind: SymbolKind::NativeBalance,
                role: SymbolRole::Native,
                network_id: "ethereum-mainnet".to_owned(),
                protocol: None,
                balance_reader: BalanceReaderConfig::NativeBalance {},
                valuation: SymbolValuationConfig {
                    quotes: vec![QuoteValuationConfig {
                        quote: QuoteCode::Usd,
                        priced_symbol_id: "eth.native.ethereum-mainnet".to_owned(),
                        reader: ValuationReaderConfig::FixedUnitPrice {
                            unit_price_dec: "1800.00".to_owned(),
                        },
                    }],
                },
                decimals: Some(18),
                underlying_symbol_id: None,
                metadata: BTreeMap::new(),
            }],
            metadata: BTreeMap::new(),
        }
    }
}
