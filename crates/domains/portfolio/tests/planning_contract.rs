use std::num::NonZeroU64;

use mfm_evm::{EvmEndpoint, EvmPhysicalTarget};
use mfm_portfolio::{plan_snapshot, PortfolioConfig, PortfolioError, PortfolioSnapshotSelector};

#[test]
fn planning_uses_the_selected_routes_and_rejects_unusable_route_sets() {
    let config: PortfolioConfig = serde_json::from_value(serde_json::json!({
        "portfolio_id": "portfolio-example",
        "quotes": ["usd"],
        "collections": [
            {
                "correlation": "chain-one",
                "request": {
                    "sources": [{
                        "source_id": "wallet.one",
                        "chain_id": 1,
                        "address": "0x0000000000000000000000000000000000000001",
                        "token": null
                    }],
                    "decimals": 18
                }
            },
            {
                "correlation": "chain-two",
                "request": {
                    "sources": [{
                        "source_id": "wallet.two",
                        "chain_id": 2,
                        "address": "0x0000000000000000000000000000000000000002",
                        "token": null
                    }],
                    "decimals": 18
                }
            }
        ]
    }))
    .expect("Portfolio config");
    let selector: PortfolioSnapshotSelector = serde_json::from_value(serde_json::json!({
        "target": "portfolio-example",
        "quote": "usd"
    }))
    .expect("selector");
    let alpha = EvmPhysicalTarget::new(
        NonZeroU64::new(1).expect("nonzero chain"),
        EvmEndpoint::new("alpha")
            .and_then(|endpoint| endpoint.endpoint_ref())
            .expect("alpha endpoint"),
    );
    let beta = EvmPhysicalTarget::new(
        NonZeroU64::new(2).expect("nonzero chain"),
        EvmEndpoint::new("beta")
            .and_then(|endpoint| endpoint.endpoint_ref())
            .expect("beta endpoint"),
    );

    let (program, input) = plan_snapshot(selector.clone(), &config, &[alpha.clone(), beta.clone()])
        .expect("planned snapshot");
    let decoded = mfm_program::Program::decode_canonical(program.canonical_bytes())
        .expect("persisted planned Program");
    assert_eq!(decoded.content_ref(), program.content_ref());
    assert_eq!(decoded.canonical_bytes(), program.canonical_bytes());
    let mut noncanonical = program.canonical_bytes().to_vec();
    noncanonical.push(b' ');
    assert!(matches!(
        mfm_program::Program::decode_canonical(&noncanonical),
        Err(mfm_program::ProgramError::Canonical)
    ));
    let input = serde_json::to_value(input).expect("admitted input");
    assert_eq!(
        input["collections"][0]["route_ref"],
        serde_json::to_value(alpha.binding_ref().expect("alpha binding")).expect("binding JSON")
    );
    assert_eq!(
        input["collections"][1]["route_ref"],
        serde_json::to_value(beta.binding_ref().expect("beta binding")).expect("binding JSON")
    );

    let replacement = EvmPhysicalTarget::new(
        NonZeroU64::new(2).expect("nonzero chain"),
        EvmEndpoint::new("replacement")
            .and_then(|endpoint| endpoint.endpoint_ref())
            .expect("replacement endpoint"),
    );
    let (replacement_program, replacement_input) =
        plan_snapshot(selector.clone(), &config, &[alpha.clone(), replacement])
            .expect("replacement plan");
    assert_ne!(program.content_ref(), replacement_program.content_ref());
    assert_ne!(
        input,
        serde_json::to_value(replacement_input).expect("replacement input")
    );

    for targets in [
        Vec::new(),
        vec![beta.clone(), alpha.clone()],
        vec![alpha.clone()],
        vec![alpha.clone(), alpha],
    ] {
        assert!(matches!(
            plan_snapshot(selector.clone(), &config, &targets),
            Err(PortfolioError::InvalidValue)
        ));
    }
}

#[test]
fn the_supported_source_capacity_plans_and_one_more_is_rejected() {
    let sources = (0..64)
        .map(|index| {
            serde_json::json!({
                "source_id": format!("wallet.{index}"),
                "chain_id": 1,
                "address": format!("0x{index:040x}"),
                "token": null
            })
        })
        .collect::<Vec<_>>();
    let config: PortfolioConfig = serde_json::from_value(serde_json::json!({
        "portfolio_id": "portfolio-example",
        "quotes": ["usd"],
        "collections": [{
            "correlation": "native-collection",
            "request": { "sources": sources, "decimals": 18 }
        }]
    }))
    .expect("maximum Portfolio config");
    let selector: PortfolioSnapshotSelector = serde_json::from_value(serde_json::json!({
        "target": "portfolio-example",
        "quote": "usd"
    }))
    .expect("selector");
    let target = EvmPhysicalTarget::new(
        NonZeroU64::new(1).expect("nonzero chain"),
        EvmEndpoint::new("alpha")
            .and_then(|endpoint| endpoint.endpoint_ref())
            .expect("endpoint"),
    );

    let (program, input) = plan_snapshot(selector, &config, &[target]).expect("maximum plan");
    assert_eq!(program.declarations().len(), 518);
    assert_eq!(
        serde_json::to_value(input).expect("admitted input")["collections"][0]["request"]
            ["sources"]
            .as_array()
            .expect("sources")
            .len(),
        64
    );

    let oversized_sources = (0..65)
        .map(|index| {
            serde_json::json!({
                "source_id": format!("wallet.{index}"),
                "chain_id": 1,
                "address": format!("0x{index:040x}"),
                "token": null
            })
        })
        .collect::<Vec<_>>();
    assert!(
        serde_json::from_value::<PortfolioConfig>(serde_json::json!({
            "portfolio_id": "portfolio-example",
            "quotes": ["usd"],
            "collections": [{
                "correlation": "native-collection",
                "request": { "sources": oversized_sources, "decimals": 18 }
            }]
        }))
        .is_err()
    );
}
