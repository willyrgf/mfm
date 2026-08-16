use mfm_app::application_catalog;
use mfm_canonical::{raw_content_digest, sha256_digest_bytes};
use mfm_capabilities::AccessCapabilityContract;
use mfm_evm::{EvmBalanceBindings, EvmBalanceRequest, EvmBalanceSource, EvmCapability, EvmState};
use mfm_ids::{ContentRef, DigestAlgorithm, DigestBytes, SchemaId};
use mfm_portfolio::{
    plan_snapshot, PortfolioConfig, PortfolioContinuation, PortfolioId, PortfolioSnapshotSelector,
};
use mfm_program::{
    canonical_value, capability_contract_ref, state_implementation_ref, BindingDescriptor,
    ProgramIngress, State,
};
use serde_json::json;

const PORTFOLIO_COLLECTIONS_MAX: usize = 64;

fn reference(label: &str) -> ContentRef {
    let schema = SchemaId::new(
        "mfm.capacity-envelope",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0; 32]),
    )
    .expect("schema");
    ContentRef::new(schema, raw_content_digest(label.as_bytes())).expect("reference")
}

fn read_binding<S, C>() -> BindingDescriptor
where
    S: State,
    C: AccessCapabilityContract,
{
    BindingDescriptor::new(
        state_implementation_ref::<S>().expect("state"),
        Some(capability_contract_ref::<C>().expect("capability")),
        Some(reference("adapter")),
        reference("target"),
        None,
        None,
    )
    .expect("read binding")
}

fn balance_bindings() -> EvmBalanceBindings {
    EvmBalanceBindings::new(
        1,
        [
            read_binding::<EvmState<1, 0, PortfolioContinuation>, EvmCapability<2>>(),
            read_binding::<EvmState<1, 1, PortfolioContinuation>, EvmCapability<6>>(),
            read_binding::<EvmState<1, 3, PortfolioContinuation>, EvmCapability<7>>(),
            read_binding::<EvmState<1, 4, PortfolioContinuation>, EvmCapability<7>>(),
            read_binding::<EvmState<1, 5, PortfolioContinuation>, EvmCapability<7>>(),
            read_binding::<EvmState<1, 6, PortfolioContinuation>, EvmCapability<6>>(),
        ],
    )
    .expect("balance bindings")
}

fn request(collection: usize) -> EvmBalanceRequest {
    EvmBalanceRequest::new(
        vec![EvmBalanceSource {
            source_id: format!("source-{collection}"),
            chain_id: 1,
            address: format!("0x{collection:040x}"),
            token: None,
        }],
        18,
    )
    .expect("balance request")
}

#[test]
fn maximum_portfolio_program_records_capacity_envelope() {
    let portfolio_id = PortfolioId {
        value: "portfolio-capacity-envelope".to_owned(),
    };
    let collections = (0..PORTFOLIO_COLLECTIONS_MAX)
        .map(|ordinal| {
            json!({
                "correlation": format!("collection-{ordinal}"),
                "request": request(ordinal),
            })
        })
        .collect::<Vec<_>>();
    let portfolio_config: PortfolioConfig = serde_json::from_value(json!({
        "portfolio_id": &portfolio_id,
        "quotes": ["usd"],
        "collections": collections,
    }))
    .expect("portfolio config");
    let balance_bindings = balance_bindings();
    let selector: PortfolioSnapshotSelector = serde_json::from_value(json!({
        "target": portfolio_id,
        "quote": "usd",
    }))
    .expect("portfolio selector");
    let (portfolio_input, portfolio_program, _) =
        plan_snapshot(selector, &portfolio_config, &[balance_bindings])
            .expect("Portfolio plan")
            .into_parts();
    let portfolio_bytes = portfolio_program
        .canonical_bytes()
        .expect("Portfolio Program bytes");
    assert_eq!(
        sha256_digest_bytes(portfolio_bytes.as_bytes()).to_string(),
        "2804cfbda713daf631f1292dd1e4efe9a2e75d46ac86807e42ab8a91ef481c23",
        "the surviving chain/anchor/balance Program wire must remain byte-for-byte stable",
    );
    let portfolio_c0_bytes = canonical_value(&portfolio_input)
        .expect("Portfolio C0 bytes")
        .as_bytes()
        .len();
    let completed = (0..PORTFOLIO_COLLECTIONS_MAX)
        .map(|ordinal| {
            let collection = request(ordinal);
            json!({
                "collection_ordinal": ordinal,
                "chain_id": 1,
                "anchor": {
                    "number": "100",
                    "hash": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                },
                "holdings": collection.sources.iter().map(|source| json!({
                    "source_id": &source.source_id,
                    "asset": { "kind": "native" },
                    "decimals": 18,
                    "raw_units": "0",
                    "amount_dec": "0.000000000000000000",
                })).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    let portfolio_context: PortfolioContinuation = serde_json::from_value(json!({
        "input": &portfolio_input,
        "completed_collections": completed,
    }))
    .expect("complete Portfolio context");
    let portfolio_cn_bytes = canonical_value(&portfolio_context)
        .expect("Portfolio Cn bytes")
        .as_bytes()
        .len();

    ProgramIngress::new(&application_catalog().expect("catalog"))
        .decode(portfolio_bytes.as_bytes())
        .expect("Portfolio Program ingress");

    eprintln!(
        "capacity-envelope app portfolio collections={} sources={} declarations={} bytes={} c0_bytes={} cn_bytes={}",
        PORTFOLIO_COLLECTIONS_MAX,
        PORTFOLIO_COLLECTIONS_MAX,
        portfolio_program.declarations().len(),
        portfolio_bytes.as_bytes().len(),
        portfolio_c0_bytes,
        portfolio_cn_bytes,
    );
}
