use mfm_portfolio_config::{
    canonicalize_portfolio_snapshot_authored_config,
    parse_portfolio_snapshot_authored_config_with_hint,
};

#[test]
fn dual_mainnet_portfolio_config_parses_and_canonicalizes() {
    let raw = include_str!("../../../examples/configs/portfolio-dual-mainnet.toml");
    let authored = parse_portfolio_snapshot_authored_config_with_hint(
        raw,
        Some("portfolio-dual-mainnet.toml".as_ref()),
    )
    .expect("authored portfolio config");
    let canonical =
        canonicalize_portfolio_snapshot_authored_config(authored).expect("canonical config");

    assert_eq!(canonical.portfolio.portfolio_id, "portfolio_dual_mainnet");
    assert_eq!(canonical.portfolio.networks.len(), 2);
    assert_eq!(canonical.portfolio.wallets.len(), 2);
    assert_eq!(canonical.portfolio.symbol_configs.len(), 2);
}
