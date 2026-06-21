use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_evm_contract_config::{ConfigurePhaseConfig, DeployPhaseConfig, ValidatePhaseConfig};
use mfm_evm_contract_model::{ConfiguredContract, DeployedContract};
use mfm_ids::ContentDigest;
use mfm_op_evm_contract_lifecycle::ContractLifecycleConfig;
use mfm_portfolio_config::{
    canonicalize_portfolio_snapshot_authored_config, PortfolioSnapshotAuthoredConfig,
    PortfolioSnapshotConfigError,
};
use serde::{Deserialize, Serialize};

use crate::{
    AuthoredConfig, CanonicalConfigMaterial, CanonicalSeedMaterial, ConfigFormat, EntryPointOpId,
    EntryPointOpPlan, EntryPointOpRegistry, LaunchableOp, OpLaunchError, OpVersion, PublicOpName,
};

const PORTFOLIO_SNAPSHOT_PUBLIC_NAME: &str = "portfolio_snapshot";
const PORTFOLIO_SNAPSHOT_NAMESPACE: &str = "mfm.portfolio";
const PORTFOLIO_SNAPSHOT_VERSION: u32 = 1;
static ENTRY_POINT_CONFIG_FORMATS: &[ConfigFormat] = &[ConfigFormat::Toml, ConfigFormat::Json];

/// Builds the production entry-point operation registry for this process.
pub(crate) fn production_entry_point_op_registry() -> Result<EntryPointOpRegistry, crate::AppError>
{
    let mut registry = EntryPointOpRegistry::new();
    registry.register(PortfolioSnapshotEntryPointOp)?;
    for kind in EvmContractEntryPointKind::ALL {
        registry.register(EvmContractEntryPointOp { kind })?;
    }
    Ok(registry)
}

struct PortfolioSnapshotEntryPointOp;

impl LaunchableOp for PortfolioSnapshotEntryPointOp {
    fn op_id(&self) -> EntryPointOpId {
        EntryPointOpId::new(
            PORTFOLIO_SNAPSHOT_NAMESPACE,
            PORTFOLIO_SNAPSHOT_PUBLIC_NAME,
            self.version(),
        )
        .expect("static portfolio entry-point op id is valid")
    }

    fn public_name(&self) -> PublicOpName {
        PublicOpName::new(PORTFOLIO_SNAPSHOT_PUBLIC_NAME)
            .expect("static portfolio public op name is valid")
    }

    fn version(&self) -> OpVersion {
        OpVersion::new(PORTFOLIO_SNAPSHOT_VERSION)
            .expect("static portfolio entry-point op version is valid")
    }

    fn accepted_config_formats(&self) -> &'static [ConfigFormat] {
        ENTRY_POINT_CONFIG_FORMATS
    }

    fn plan(&self, authored_config: AuthoredConfig) -> Result<EntryPointOpPlan, OpLaunchError> {
        if !self
            .accepted_config_formats()
            .contains(&authored_config.format())
        {
            return Err(OpLaunchError::new(
                "EntryPointOpConfigFormatUnsupported",
                "entry-point op does not accept the supplied config format",
            ));
        }

        let authored = authored_config.normalize::<PortfolioSnapshotAuthoredConfig>()?;
        let canonical = canonicalize_portfolio_snapshot_authored_config(authored.value.clone())
            .map_err(portfolio_config_error)?;
        let planned = mfm_op_portfolio_tracker::plan_portfolio_snapshot_program(canonical)
            .map_err(portfolio_plan_error)?;
        let config_material = planned
            .config_artifacts
            .into_iter()
            .map(|artifact| {
                let bytes = PlainCanonicalJsonBytes::from_canonical_json_slice(&artifact.bytes)
                    .map_err(|_| {
                        OpLaunchError::new(
                            "EntryPointOpConfigMaterialInvalid",
                            "entry-point op produced invalid canonical config material",
                        )
                    })?;
                Ok(CanonicalConfigMaterial {
                    schema_id: artifact.schema_id,
                    bytes,
                    media_type: artifact.media_type,
                })
            })
            .collect::<Result<Vec<_>, OpLaunchError>>()?;
        Ok(EntryPointOpPlan {
            draft: planned.draft,
            config_material,
            seed_material: Vec::new(),
            authored_config_digest: authored.authored_digest,
        })
    }
}

fn portfolio_config_error(_error: PortfolioSnapshotConfigError) -> OpLaunchError {
    OpLaunchError::new(
        "PortfolioSnapshotConfigInvalid",
        "portfolio snapshot config is invalid",
    )
}

fn portfolio_plan_error(
    _error: mfm_op_portfolio_tracker::PortfolioSnapshotCompileError,
) -> OpLaunchError {
    OpLaunchError::new(
        "PortfolioSnapshotPlanFailed",
        "portfolio snapshot entry-point planning failed",
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EvmContractEntryPointKind {
    Deploy,
    Configure,
    Validate,
    Lifecycle,
}

impl EvmContractEntryPointKind {
    const ALL: [Self; 4] = [
        Self::Deploy,
        Self::Configure,
        Self::Validate,
        Self::Lifecycle,
    ];

    fn public_name(self) -> &'static str {
        match self {
            Self::Deploy => "evm_contract_deploy",
            Self::Configure => "evm_contract_configure",
            Self::Validate => "evm_contract_validate",
            Self::Lifecycle => "evm_contract_lifecycle",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Deploy => "contract_deploy",
            Self::Configure => "contract_configure",
            Self::Validate => "contract_validate",
            Self::Lifecycle => "contract_lifecycle",
        }
    }
}

struct EvmContractEntryPointOp {
    kind: EvmContractEntryPointKind,
}

impl LaunchableOp for EvmContractEntryPointOp {
    fn op_id(&self) -> EntryPointOpId {
        EntryPointOpId::new("mfm.evm.contract", self.kind.name(), self.version())
            .expect("static EVM contract entry-point op id is valid")
    }

    fn public_name(&self) -> PublicOpName {
        PublicOpName::new(self.kind.public_name())
            .expect("static EVM contract public op name is valid")
    }

    fn version(&self) -> OpVersion {
        OpVersion::new(1).expect("static EVM contract entry-point op version is valid")
    }

    fn accepted_config_formats(&self) -> &'static [ConfigFormat] {
        ENTRY_POINT_CONFIG_FORMATS
    }

    fn plan(&self, authored_config: AuthoredConfig) -> Result<EntryPointOpPlan, OpLaunchError> {
        if !self
            .accepted_config_formats()
            .contains(&authored_config.format())
        {
            return Err(OpLaunchError::new(
                "EntryPointOpConfigFormatUnsupported",
                "entry-point op does not accept the supplied config format",
            ));
        }
        match self.kind {
            EvmContractEntryPointKind::Deploy => {
                let normalized = authored_config.normalize::<DeployPhaseConfig>()?;
                let planned =
                    mfm_op_evm_contract_lifecycle::plan_contract_deploy_program(normalized.value)
                        .map_err(evm_contract_plan_error)?;
                self.entry_plan(planned, normalized.authored_digest)
            }
            EvmContractEntryPointKind::Configure => {
                let normalized = authored_config.normalize::<ConfigureEntryPointConfig>()?;
                let planned = mfm_op_evm_contract_lifecycle::plan_contract_configure_program(
                    normalized.value.config,
                    normalized.value.deployed,
                )
                .map_err(evm_contract_plan_error)?;
                self.entry_plan(planned, normalized.authored_digest)
            }
            EvmContractEntryPointKind::Validate => {
                let normalized = authored_config.normalize::<ValidateEntryPointConfig>()?;
                let planned = mfm_op_evm_contract_lifecycle::plan_contract_validate_program(
                    normalized.value.config,
                    normalized.value.configured,
                )
                .map_err(evm_contract_plan_error)?;
                self.entry_plan(planned, normalized.authored_digest)
            }
            EvmContractEntryPointKind::Lifecycle => {
                let normalized = authored_config.normalize::<ContractLifecycleConfig>()?;
                let planned = mfm_op_evm_contract_lifecycle::plan_contract_lifecycle_program(
                    normalized.value,
                )
                .map_err(evm_contract_plan_error)?;
                self.entry_plan(planned, normalized.authored_digest)
            }
        }
    }
}

impl EvmContractEntryPointOp {
    fn entry_plan(
        &self,
        planned: mfm_op_evm_contract_lifecycle::PlannedContractLifecycleProgram,
        authored_config_digest: ContentDigest,
    ) -> Result<EntryPointOpPlan, OpLaunchError> {
        Ok(EntryPointOpPlan {
            draft: planned.draft,
            config_material: evm_config_material(planned.config_artifacts)?,
            seed_material: planned
                .seed_artifacts
                .into_iter()
                .map(|seed| CanonicalSeedMaterial {
                    seed_id: seed.seed_id,
                    bytes: seed.bytes,
                    media_type: seed.media_type,
                })
                .collect(),
            authored_config_digest,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ConfigureEntryPointConfig {
    config: ConfigurePhaseConfig,
    deployed: DeployedContract,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ValidateEntryPointConfig {
    config: ValidatePhaseConfig,
    configured: ConfiguredContract,
}

fn evm_config_material(
    artifacts: Vec<mfm_op_evm_contract_lifecycle::ContractLifecycleConfigArtifact>,
) -> Result<Vec<CanonicalConfigMaterial>, OpLaunchError> {
    artifacts
        .into_iter()
        .map(|artifact| {
            let bytes = PlainCanonicalJsonBytes::from_canonical_json_slice(&artifact.bytes)
                .map_err(|_| {
                    OpLaunchError::new(
                        "EntryPointOpConfigMaterialInvalid",
                        "entry-point op produced invalid canonical config material",
                    )
                })?;
            Ok(CanonicalConfigMaterial {
                schema_id: artifact.schema_id,
                bytes,
                media_type: artifact.media_type,
            })
        })
        .collect()
}

fn evm_contract_plan_error(
    _error: mfm_op_evm_contract_lifecycle::ContractLifecycleCompileError,
) -> OpLaunchError {
    OpLaunchError::new(
        "EvmContractPlanFailed",
        "EVM contract entry-point planning failed",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_registry_resolves_portfolio_snapshot_latest() {
        let registry = production_entry_point_op_registry().expect("registry");
        let name = PublicOpName::new(PORTFOLIO_SNAPSHOT_PUBLIC_NAME).expect("name");
        let op = registry.resolve_latest(&name).expect("portfolio op");

        assert_eq!(op.version(), OpVersion::new(1).unwrap());
        assert_eq!(
            op.accepted_config_formats(),
            &[ConfigFormat::Toml, ConfigFormat::Json]
        );
    }

    #[test]
    fn portfolio_snapshot_entry_point_plans_from_json_config() {
        let registry = production_entry_point_op_registry().expect("registry");
        let name = PublicOpName::new(PORTFOLIO_SNAPSHOT_PUBLIC_NAME).expect("name");
        let op = registry.resolve_latest(&name).expect("portfolio op");
        let authored = AuthoredConfig::new(ConfigFormat::Json, sample_portfolio_config_json())
            .expect("authored config");

        let plan = op.plan(authored).expect("portfolio plan");

        assert!(!plan.draft.state_nodes().is_empty());
        assert!(!plan.config_material.is_empty());
        assert!(plan.seed_material.is_empty());
    }

    #[test]
    fn app_prepare_entry_point_run_launch_records_evidence_and_certifies() {
        let entry_point_registry = production_entry_point_op_registry().expect("registry");
        let certification_registry = crate::production_certification_registry().expect("cert");
        let public_op_name = PublicOpName::new(PORTFOLIO_SNAPSHOT_PUBLIC_NAME).expect("name");
        let authored = AuthoredConfig::new(ConfigFormat::Json, sample_portfolio_config_json())
            .expect("authored config");
        let registry_digest = entry_point_registry.registry_digest().expect("digest");
        let run_id = crate::new_run_id();

        let prepared = crate::prepare_entry_point_run_launch(crate::EntryPointRunLaunchInput {
            entry_point_registry: &entry_point_registry,
            public_op_name: public_op_name.clone(),
            op_version: None,
            authored_config: authored,
            certification_registry: &certification_registry,
            run_id: run_id.clone(),
            drive: crate::DriveMode::AppendOnly,
        })
        .expect("prepared entry-point launch");

        assert_eq!(
            prepared.evidence.resolved_op_id.name,
            PORTFOLIO_SNAPSHOT_PUBLIC_NAME
        );
        assert_eq!(
            prepared.evidence.entry_point_registry_digest,
            registry_digest
        );
        assert!(prepared.public_output_schema_id.is_some());
        assert_eq!(prepared.request.run_id, run_id);
        assert!(!prepared.request.evidence.config_artifacts.is_empty());
    }

    #[test]
    fn evm_contract_entry_point_registry_resolves_all_versions() {
        let registry = production_entry_point_op_registry().expect("registry");

        for name in [
            "evm_contract_deploy",
            "evm_contract_configure",
            "evm_contract_validate",
            "evm_contract_lifecycle",
        ] {
            let op = registry
                .resolve_latest(&PublicOpName::new(name).expect("name"))
                .expect("EVM contract op");
            assert_eq!(op.version(), OpVersion::new(1).unwrap());
        }
    }

    #[test]
    fn evm_contract_entry_point_plans_deploy_and_lifecycle_without_seeds() {
        let registry = production_entry_point_op_registry().expect("registry");

        for (name, config) in [
            ("evm_contract_deploy", deploy_config_json().to_string()),
            (
                "evm_contract_lifecycle",
                lifecycle_config_json().to_string(),
            ),
        ] {
            let op = registry
                .resolve_latest(&PublicOpName::new(name).expect("name"))
                .expect("EVM contract op");
            let authored = AuthoredConfig::new(ConfigFormat::Json, config).expect("authored");
            let plan = op.plan(authored).expect("EVM contract plan");

            assert!(!plan.draft.state_nodes().is_empty());
            assert!(!plan.config_material.is_empty());
            assert!(plan.seed_material.is_empty());
        }
    }

    #[test]
    fn evm_contract_entry_point_plans_configure_and_validate_with_seed_material() {
        let registry = production_entry_point_op_registry().expect("registry");

        for (name, config) in [
            (
                "evm_contract_configure",
                configure_entry_config_json().to_string(),
            ),
            (
                "evm_contract_validate",
                validate_entry_config_json().to_string(),
            ),
        ] {
            let op = registry
                .resolve_latest(&PublicOpName::new(name).expect("name"))
                .expect("EVM contract op");
            let authored = AuthoredConfig::new(ConfigFormat::Json, config).expect("authored");
            let plan = op.plan(authored).expect("EVM contract plan");

            assert_eq!(plan.seed_material.len(), 1);
            assert_eq!(
                plan.seed_material[0].media_type.as_str(),
                "application/json"
            );
            assert_eq!(plan.seed_material[0].seed_id, plan.draft.seeds()[0].seed_id);
            assert!(!plan.config_material.is_empty());
        }
    }

    fn sample_portfolio_config_json() -> String {
        serde_json::json!({
            "portfolio": {
                "portfolio_id": "portfolio_main",
                "quote_codes": ["USD"],
                "networks": [
                    {
                        "network_id": "ethereum-mainnet",
                        "family": "evm",
                        "chain_id": 1,
                        "control_scope": "shared",
                        "metadata": {}
                    }
                ],
                "wallets": [
                    {
                        "wallet_id": "wallet_main",
                        "subject": {
                            "kind": "evm_address",
                            "address": "0x000000000000000000000000000000000000dead"
                        },
                        "implementation": { "kind": "address_only" },
                        "network_id": "ethereum-mainnet",
                        "symbol_ids": ["eth.native.ethereum-mainnet"],
                        "metadata": {}
                    }
                ],
                "symbol_configs": [
                    {
                        "symbol_id": "eth.native.ethereum-mainnet",
                        "display_symbol": "ETH",
                        "kind": "native_balance",
                        "role": "native",
                        "network_id": "ethereum-mainnet",
                        "protocol": null,
                        "balance_reader": { "kind": "native_balance" },
                        "valuation": {
                            "quotes": [
                                {
                                    "quote": "USD",
                                    "priced_symbol_id": "eth.native.ethereum-mainnet",
                                    "reader": {
                                        "kind": "fixed_unit_price",
                                        "unit_price_dec": "1800.00"
                                    }
                                }
                            ]
                        },
                        "decimals": 18,
                        "underlying_symbol_id": null,
                        "metadata": {}
                    }
                ],
                "metadata": {}
            },
            "valuation_source_registry": { "sources": [] }
        })
        .to_string()
    }

    fn network_json() -> serde_json::Value {
        serde_json::json!({
            "network_id": "ethereum-mainnet",
            "expected_chain_id": 1,
        })
    }

    fn signer_json() -> serde_json::Value {
        serde_json::json!({
            "signer_ref": "deployer",
            "expected_signer_address": "0x000000000000000000000000000000000000dead",
        })
    }

    fn deploy_config_json() -> serde_json::Value {
        serde_json::json!({
            "network": network_json(),
            "signer": signer_json(),
        })
    }

    fn configure_config_json() -> serde_json::Value {
        serde_json::json!({
            "network": network_json(),
            "signer": signer_json(),
            "calls": [],
        })
    }

    fn validate_config_json() -> serde_json::Value {
        serde_json::json!({
            "network": network_json(),
        })
    }

    fn lifecycle_config_json() -> serde_json::Value {
        serde_json::json!({
            "deploy": deploy_config_json(),
            "configure": configure_config_json(),
            "validate": validate_config_json(),
        })
    }

    fn deployed_contract_json() -> serde_json::Value {
        serde_json::json!({
            "lifecycle_version": 1,
            "network_id": "ethereum-mainnet",
            "expected_chain_id": 1,
            "contract_address": "0x000000000000000000000000000000000000dead",
            "deploy_tx_hash": "0x01",
            "deploy_receipt_evidence": null,
            "deployed_block_number": 1,
        })
    }

    fn configured_contract_json() -> serde_json::Value {
        serde_json::json!({
            "lifecycle_version": 1,
            "deployed": deployed_contract_json(),
            "configure_calls": [],
            "confirmation_read_assertions": [],
            "confirmation_event_assertions": [],
            "configure_tx_hashes": [],
            "configure_receipt_evidence": [],
            "configured_block_number": 2,
        })
    }

    fn configure_entry_config_json() -> serde_json::Value {
        serde_json::json!({
            "config": configure_config_json(),
            "deployed": deployed_contract_json(),
        })
    }

    fn validate_entry_config_json() -> serde_json::Value {
        serde_json::json!({
            "config": validate_config_json(),
            "configured": configured_contract_json(),
        })
    }
}
