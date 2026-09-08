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
    let alpha = EvmPhysicalTarget {
        chain_id: NonZeroU64::new(1).expect("nonzero chain"),
        endpoint_ref: EvmEndpoint::new("alpha")
            .and_then(|endpoint| endpoint.endpoint_ref())
            .expect("alpha endpoint"),
    };
    let beta = EvmPhysicalTarget {
        chain_id: NonZeroU64::new(2).expect("nonzero chain"),
        endpoint_ref: EvmEndpoint::new("beta")
            .and_then(|endpoint| endpoint.endpoint_ref())
            .expect("beta endpoint"),
    };

    let (program, input) = plan_snapshot(
        selector.clone(),
        &config,
        &[alpha.clone(), beta.clone()],
        None,
    )
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

    let replacement = EvmPhysicalTarget {
        chain_id: NonZeroU64::new(2).expect("nonzero chain"),
        endpoint_ref: EvmEndpoint::new("replacement")
            .and_then(|endpoint| endpoint.endpoint_ref())
            .expect("replacement endpoint"),
    };
    let (replacement_program, replacement_input) = plan_snapshot(
        selector.clone(),
        &config,
        &[alpha.clone(), replacement],
        None,
    )
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
            plan_snapshot(selector.clone(), &config, &targets, None),
            Err(PortfolioError::InvalidValue)
        ));
    }
}

#[test]
fn the_supported_source_capacity_plans_and_one_more_is_rejected() {
    // Quotes require JSON escaping while remaining valid public identity characters.
    let portfolio_id = "\"".repeat(256);
    let selector: PortfolioSnapshotSelector = serde_json::from_value(serde_json::json!({
        "target": portfolio_id,
        "quote": "usd"
    }))
    .expect("selector");
    let target = EvmPhysicalTarget {
        chain_id: NonZeroU64::new(u64::MAX).expect("nonzero chain"),
        endpoint_ref: EvmEndpoint::new("\"".repeat(256))
            .and_then(|endpoint| endpoint.endpoint_ref())
            .expect("endpoint"),
    };

    for token_count in [0, 32, 64] {
        for collection_count in [1, 64] {
            let sources = (0..64)
                .map(|index| {
                    serde_json::json!({
                        "source_id": format!("{:02}{}", index, "\"".repeat(254)),
                        "chain_id": u64::MAX,
                        "address": format!("0x{index:040x}"),
                        "token": (index < token_count).then(|| format!("0x{:040x}", index + 64))
                    })
                })
                .collect::<Vec<_>>();
            let collections = sources
                .chunks(64 / collection_count)
                .enumerate()
                .map(|(index, sources)| {
                    serde_json::json!({
                        "correlation": format!("{:02}{}", index, "\"".repeat(254)),
                        "request": {"sources": sources, "decimals": 30}
                    })
                })
                .collect::<Vec<_>>();
            let config_json = serde_json::json!({
                "portfolio_id": portfolio_id,
                "quotes": ["usd"],
                "collections": collections
            });
            let config: PortfolioConfig =
                serde_json::from_value(config_json.clone()).expect("maximum Portfolio config");
            let (program, input) = plan_snapshot(
                selector.clone(),
                &config,
                std::slice::from_ref(&target),
                None,
            )
            .expect("maximum plan");
            // Four native Reads, five token Reads, enter/child consolidation/resume
            // per collection, and root initialization/consolidation.
            let conclusions = 4 * (64 - token_count) + 5 * token_count + 3 * collection_count + 2;
            assert_eq!(program.declarations().len(), conclusions);
            let (canonical_input, input_ref) =
                mfm_values::canonicalize_mfm_value(&input).expect("bounded admitted input");
            assert_eq!(program.initial_value_ref(), &input_ref);
            // Reserve a complete 64 KiB admission envelope in addition to both objects.
            // Actual Journal closure measurements belong to Runtime integration tests.
            let genesis =
                program.canonical_bytes().len() + canonical_input.as_bytes().len() + 65_536;
            let bound = program
                .history_bound(
                    mfm_program::ConclusionBound::new(genesis as u64).expect("genesis bound"),
                )
                .expect("finite history");
            assert_eq!(bound.frames(), 1 + conclusions as u64);
            assert!(bound.frames() <= 65_536);
            assert!(
                bound.bytes() <= 536_870_912,
                "{collection_count} collections/{token_count} tokens: {} bytes",
                bound.bytes()
            );
            eprintln!(
                "{collection_count} collections/{token_count} tokens: {} frames, {} bytes",
                bound.frames(),
                bound.bytes()
            );
            let decoded = mfm_program::Program::decode_canonical(program.canonical_bytes())
                .expect("persisted maximum Program");
            assert_eq!(decoded.content_ref(), program.content_ref());

            let mut oversized = config_json;
            let extra = serde_json::json!({
                "source_id": "one-too-many",
                "chain_id": u64::MAX,
                "address": format!("0x{:040x}", 65),
                "token": null
            });
            oversized["collections"][0]["request"]["sources"]
                .as_array_mut()
                .expect("sources")
                .push(extra);
            assert!(serde_json::from_value::<PortfolioConfig>(oversized).is_err());
        }
    }
}
