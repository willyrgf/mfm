use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::ContentDigest;
use mfm_portfolio_config::{
    canonicalize_portfolio_snapshot_authored_config, PortfolioSnapshotAuthoredConfig,
    PortfolioSnapshotConfigError,
};

use crate::{
    AuthoredConfig, CanonicalConfigMaterial, CanonicalizerIdentity, ConfigFormat, EntryPointOpId,
    EntryPointOpPlan, EntryPointOpRegistry, LaunchableOp, LoweringIdentity, OpLaunchError,
    OpVersion, PublicOpName,
};

const PORTFOLIO_SNAPSHOT_PUBLIC_NAME: &str = "portfolio_snapshot";
const PORTFOLIO_SNAPSHOT_NAMESPACE: &str = "mfm.portfolio";
const PORTFOLIO_SNAPSHOT_VERSION: u32 = 1;
const PORTFOLIO_SNAPSHOT_LOWERING_IDENTITY: &str = "mfm.portfolio.snapshot.lowering.v1";
const PORTFOLIO_SNAPSHOT_CANONICALIZER_IDENTITY: &str = "mfm.portfolio.snapshot.canonicalizer.v1";
static PORTFOLIO_CONFIG_FORMATS: &[ConfigFormat] = &[ConfigFormat::Toml, ConfigFormat::Json];

/// Builds the production entry-point operation registry for this process.
pub(crate) fn production_entry_point_op_registry() -> Result<EntryPointOpRegistry, crate::AppError>
{
    let mut registry = EntryPointOpRegistry::new();
    registry.register(PortfolioSnapshotEntryPointOp)?;
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
        PORTFOLIO_CONFIG_FORMATS
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
        let canonical_config_digest = canonical_portfolio_config_digest(&canonical)?;
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
            public_output_schema_id: Some(planned.public_schema_id),
            lowering_identity: LoweringIdentity::new(PORTFOLIO_SNAPSHOT_LOWERING_IDENTITY)?,
            canonicalizer_identity: CanonicalizerIdentity::new(
                PORTFOLIO_SNAPSHOT_CANONICALIZER_IDENTITY,
            )?,
            authored_config_digest: authored.authored_digest,
            canonical_config_digest,
        })
    }
}

fn canonical_portfolio_config_digest(
    canonical: &mfm_portfolio_config::PortfolioSnapshotCanonicalConfig,
) -> Result<ContentDigest, OpLaunchError> {
    let value = canonical.to_json_value().map_err(portfolio_config_error)?;
    let bytes = PlainCanonicalJsonBytes::from_json_str(&value.to_string()).map_err(|_| {
        OpLaunchError::new(
            "PortfolioSnapshotConfigInvalid",
            "portfolio snapshot config could not be canonicalized",
        )
    })?;
    Ok(bytes.content_digest())
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
        assert!(plan.public_output_schema_id.is_some());
        assert_eq!(
            plan.lowering_identity.as_str(),
            PORTFOLIO_SNAPSHOT_LOWERING_IDENTITY
        );
        assert_eq!(
            plan.canonicalizer_identity.as_str(),
            PORTFOLIO_SNAPSHOT_CANONICALIZER_IDENTITY
        );
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
}
