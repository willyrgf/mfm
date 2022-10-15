mod common;

use common::App;
use mfm::asset::Asset;
use web3::types::U256;

#[tokio::test]
async fn test_get_exchange_by_liquidity() {
    let app = App::new();

    let config = app.config();

    let busd_config = config.assets.get("busd").unwrap();
    let busd_bsc_assets: Vec<Asset> = busd_config
        .new_assets_list()
        .unwrap()
        .into_iter()
        .filter(|a| a.network_id() == "bsc")
        .collect();

    let busd_bsc = busd_bsc_assets.last().unwrap();

    let wbnb_config = config.assets.get("wbnb").unwrap();
    let wbnb_bsc_assets: Vec<Asset> = wbnb_config
        .new_assets_list()
        .unwrap()
        .into_iter()
        .filter(|a| a.network_id() == "bsc")
        .collect();

    let wbnb_bsc = wbnb_bsc_assets.last().unwrap();

    let bsc_network = config.networks.get("bsc").unwrap();

    let amount_in = U256::exp10(18);

    let exchange = bsc_network
        .get_exchange_by_liquidity(wbnb_bsc, busd_bsc, amount_in)
        .await
        .unwrap();

    assert_eq!(exchange.name(), "biswap");
}
