use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes, RecoverabilityContract};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, SchemaId, SemanticTypeId, StableId};
use mfm_journal::{ArtifactIdPreimage, ObjectEvidencePreimage, ProducerBinding, ValueRef};
use mfm_program::{encode_config, UnitConfig};
use mfm_spec::AuthoredSourceSelector;
use mfm_values::{MfmConfig, PublicOutputDescriptor, RetainedValueContract};
use serde_json::json;

use super::*;
use crate::{
    decode_portfolio_config, EvmRoutingBinding, PortfolioId, PortfolioPublicOutputs,
    PortfolioRoutingManifest,
};

const ACCOUNT: &str = "0x000000000000000000000000000000000000dead";
const TOKEN: &str = "0x0000000000000000000000000000000000000001";

fn generation() -> mfm_evm::EvmRoutingGenerationRef {
    let schema = SchemaId::new(
        "mfm.test.evm-routing",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"routing schema"),
    )
    .expect("schema");
    mfm_evm::EvmRoutingGenerationRef::from_content_ref(
        ContentRef::new(
            schema,
            ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                sha256_digest_bytes(b"generation"),
            ),
        )
        .expect("content ref"),
    )
    .expect("generation")
}

fn portfolio() -> PortfolioConfig {
    decode_portfolio_config(&json!({
        "portfolio_id": "portfolio-main",
        "quote_codes": ["USD"],
        "networks": [{
            "family": "evm",
            "network_id": "ethereum-mainnet",
            "chain_id": 1,
            "native_decimals": 18,
            "metadata": {}
        }],
        "wallets": [{
            "wallet_id": "wallet-main",
            "subject": {"kind": "evm_address", "address": ACCOUNT},
            "network_id": "ethereum-mainnet",
            "implementation": {"kind": "address_only"},
            "symbol_ids": ["eth-native", "usdc"],
            "metadata": {}
        }],
        "symbol_configs": [{
            "symbol_id": "eth-native",
            "display_symbol": "ETH",
            "network_id": "ethereum-mainnet",
            "source": {"kind": "native"},
            "valuation": {"quotes": [{
                "quote": "USD",
                "priced_symbol_id": "eth-native",
                "unit_price_dec": "2000"
            }]},
            "metadata": {}
        }, {
            "symbol_id": "usdc",
            "display_symbol": "USDC",
            "network_id": "ethereum-mainnet",
            "source": {"kind": "erc20", "contract_address": TOKEN},
            "valuation": {"quotes": [{
                "quote": "USD",
                "priced_symbol_id": "usdc",
                "unit_price_dec": "1"
            }]},
            "metadata": {}
        }],
        "metadata": {}
    }))
    .expect("portfolio")
}

fn routing_manifest() -> PortfolioRoutingManifest {
    PortfolioRoutingManifest::new(vec![EvmRoutingBinding::new(
        "ethereum-mainnet",
        generation(),
    )
    .expect("binding")])
    .expect("routing manifest")
}

fn routing_member_path() -> FieldPath {
    FieldPath::new("product/portfolio-routing").expect("routing member path")
}

fn authored_program() -> CanonicalAuthoredProgram {
    portfolio_snapshot_authored_program(
        &portfolio(),
        &routing_manifest(),
        unit_config_ref(),
        routing_member_path(),
    )
    .expect("authored program")
}

fn unit_config_ref() -> ValueRef {
    let (canonical, _) = encode_config(&UnitConfig {}).expect("unit config");
    let evidence_schema = SchemaId::new(
        "mfm.test.object-evidence",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"test object evidence schema"),
    )
    .expect("evidence schema");
    let evidence_ref = ContentRef::new(
        evidence_schema,
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            sha256_digest_bytes(b"test object evidence contract"),
        ),
    )
    .expect("evidence ref");
    let semantic = SemanticTypeId::new(
        "mfm.test",
        "unit-config",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"test unit config semantic"),
    )
    .expect("semantic");
    let contract = RetainedValueContract::new(
        UnitConfig::schema_id().expect("unit schema"),
        semantic,
        StableId::new("mfm.test.unit-config").expect("role"),
        "application/json",
        evidence_ref,
    )
    .expect("contract");
    derive_value_ref(
        &contract,
        &canonical,
        &ProducerBinding::this_admission(
            &StableId::new("mfm.test.unit-config").expect("producer slot"),
        )
        .expect("producer"),
    )
}

fn derive_value_ref(
    contract: &RetainedValueContract,
    canonical: &PlainCanonicalJsonBytes,
    producer: &ProducerBinding,
) -> ValueRef {
    let recoverability = RecoverabilityContract::embedded().expect("recoverability contract");
    let content_digest = recoverability.raw_content_digest(canonical.as_bytes());
    let artifact_id = ArtifactIdPreimage::new(
        contract.schema_id(),
        &content_digest,
        contract.semantic_type_id(),
    )
    .expect("artifact preimage")
    .artifact_id()
    .expect("artifact id");
    let byte_length = u64::try_from(canonical.as_bytes().len()).expect("bounded bytes");
    let evidence_hash = ObjectEvidencePreimage::new(
        &artifact_id,
        &content_digest,
        contract.schema_id(),
        byte_length,
        contract.media_type(),
        contract.evidence_contract_ref(),
    )
    .expect("evidence preimage")
    .evidence_hash()
    .expect("evidence hash");
    ValueRef::new(
        &artifact_id,
        &content_digest,
        &evidence_hash,
        contract.schema_id(),
        contract.semantic_type_id(),
        contract.role(),
        byte_length,
        contract.media_type(),
        contract.evidence_contract_ref(),
        producer,
    )
    .expect("value ref")
}

#[test]
fn authors_exact_validator_gated_decomposed_graph() {
    let program = authored_program();
    let keys = program
        .nodes()
        .iter()
        .map(|node| node.stable_key().as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        [
            "assemble_snapshot",
            "evm_collection/0000/aggregate_balances",
            "evm_collection/0000/bootstrap_source",
            "evm_collection/0000/confirm_anchor",
            "evm_collection/0000/initial_anchor",
            "evm_collection/0000/native_balance/0000",
            "evm_collection/0000/token_balance/0001",
            "evm_collection/0000/token_decimals/0000",
            "project_report",
            "validate_selection",
        ]
    );

    let validator = program
        .input_bindings()
        .iter()
        .filter(|binding| binding.consumer_key().as_str() == "validate_selection")
        .collect::<Vec<_>>();
    assert_eq!(validator.len(), 3);
    assert!(matches!(
        validator[0].source(),
        AuthoredSourceSelector::Config {
            source_field_path: None
        }
    ));
    assert!(matches!(
        validator[1].source(),
        AuthoredSourceSelector::QualifiedSupport {
            member_path,
            source_field_path: None,
        } if member_path == &routing_member_path()
    ));
    assert!(matches!(
        validator[2].source(),
        AuthoredSourceSelector::RunAdmission {
            source_field_path: None
        }
    ));

    for binding in program
        .input_bindings()
        .iter()
        .filter(|binding| binding.consumer_key().as_str() != "validate_selection")
    {
        assert!(
            !matches!(
                binding.source(),
                AuthoredSourceSelector::Config { .. }
                    | AuthoredSourceSelector::QualifiedSupport { .. }
                    | AuthoredSourceSelector::RunAdmission { .. }
            ),
            "only the validator may consume admitted semantic roots"
        );
    }
}

#[test]
fn every_static_collection_value_is_selected_from_validator_output() {
    let program = authored_program();
    let validator_sources = program
        .input_bindings()
        .iter()
        .filter_map(|binding| match binding.source() {
            AuthoredSourceSelector::NodeOutput {
                producer_key,
                source_field_path: Some(path),
                ..
            } if producer_key.as_str() == "validate_selection" => Some(path.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    for required in [
        "positions.0.binding",
        "positions.0.native_decimals",
        "positions.0.sources.0",
        "positions.0.sources.1",
        "positions.0.token_contracts.0",
    ] {
        assert!(
            validator_sources.contains(&required),
            "missing validator projection {required}"
        );
    }
    assert!(program.input_bindings().iter().any(|binding| {
        binding.consumer_key().as_str() == "assemble_snapshot"
            && matches!(
                binding.source(),
                AuthoredSourceSelector::NodeOutput {
                    producer_key,
                    source_field_path: None,
                    ..
                } if producer_key.as_str() == "validate_selection"
            )
    }));
}

#[test]
fn fanout_confirmation_and_aggregate_edges_are_exact() {
    let program = authored_program();
    let confirmation_sources = program
        .input_bindings()
        .iter()
        .filter(|binding| binding.consumer_key().as_str() == "evm_collection/0000/confirm_anchor")
        .count();
    assert_eq!(confirmation_sources, 3, "one decimals plus two balances");

    let aggregate_results = program
        .input_bindings()
        .iter()
        .filter(|binding| {
            binding.consumer_key().as_str() == "evm_collection/0000/aggregate_balances"
                && binding.consumer_input_ordinal() == 2
        })
        .count();
    assert_eq!(
        aggregate_results, 4,
        "three fanout results plus final confirmation"
    );
}

#[test]
fn public_output_is_exact_snapshot_and_report() {
    let program = authored_program();
    let public = program.public_output_bindings();
    assert_eq!(public.len(), 2);
    assert_eq!(public[0].field().as_str(), "report");
    assert_eq!(public[0].producer_key().as_str(), "project_report");
    assert_eq!(public[1].field().as_str(), "snapshot");
    assert_eq!(public[1].producer_key().as_str(), "assemble_snapshot");
    assert!(public
        .iter()
        .all(|binding| binding.source_field_path().is_none()));
    assert_eq!(
        program
            .required_success_node_keys()
            .iter()
            .map(StableId::as_str)
            .collect::<Vec<_>>(),
        ["evm_collection/0000/aggregate_balances", "project_report"]
    );
}

#[test]
fn graph_contains_no_old_aggregate_bitcoin_retry_or_fallback_surface() {
    let rendered = serde_json::to_string(&authored_program()).expect("program JSON");
    for forbidden in [
        "fact_query",
        "select_holdings",
        "bitcoin",
        "retry",
        "fallback",
        "current_route",
        "aggregate_reader",
        "batch",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "authored graph retained forbidden surface {forbidden}"
        );
    }
}

#[test]
fn published_entry_point_is_exact_and_policy_empty() {
    let planner_contract = generation().to_content_ref().expect("planner contract");
    let planner_implementation = ContentRef::new(
        SchemaId::new(
            "mfm.test.planner-implementation",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"planner implementation schema"),
        )
        .expect("schema"),
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            sha256_digest_bytes(b"planner implementation"),
        ),
    )
    .expect("implementation");
    let entry = portfolio_snapshot_entry_point_contract(planner_contract, planner_implementation)
        .expect("entry point");
    assert_eq!(
        entry.entry_point_id().as_str(),
        PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID
    );
    assert_eq!(
        entry.entry_point_operation_id().as_str(),
        PORTFOLIO_SNAPSHOT_OPERATION_ID
    );
    assert!(entry.planning_profile().framework_policy_refs().is_empty());
    assert_eq!(
        entry
            .planning_profile()
            .canonical_profile_parameters()
            .as_json(),
        &json!({})
    );
    assert_eq!(
        entry.input_schema_id(),
        &PortfolioSnapshotSelector::schema_id().expect("input schema")
    );
    assert_eq!(
        entry.public_output_schema_id(),
        &PortfolioPublicOutputs::public_schema_id().expect("output schema")
    );
}

#[test]
fn selector_is_only_one_checked_configured_value_target() {
    let selector =
        PortfolioSnapshotSelector::new(PortfolioId::new("portfolio-main").expect("target id"));
    assert_eq!(selector.target().as_str(), "portfolio-main");
    assert_eq!(
        serde_json::to_value(&selector).expect("selector JSON"),
        json!({"target": "portfolio-main"})
    );
    assert!(serde_json::from_value::<PortfolioSnapshotSelector>(json!({
        "target": "portfolio-main",
        "routing_generation": "forbidden"
    }))
    .is_err());
    assert!(
        serde_json::from_value::<PortfolioSnapshotSelector>(json!({"target": "bad target"}))
            .is_err()
    );
}
