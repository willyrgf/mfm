mod common;

use common::App;
use mfm::asset::Asset;
use web3::types::U256;

#[tokio::test]
async fn test_get_exchange_by_liquidity() {
    struct TestCase {
        network_name: &'static str,
        input_asset_name: &'static str,
        output_asset_name: &'static str,
        exchange_expected: &'static str,
    }

    let app = App::new();
    let config = app.config();

    let test_cases = vec![
        TestCase {
            network_name: "polygon",
            input_asset_name: "usdc",
            output_asset_name: "bal",
            exchange_expected: "sushi_swap_polygon",
        },
        TestCase {
            network_name: "polygon",
            input_asset_name: "usdc",
            output_asset_name: "sol",
            exchange_expected: "quickswap_v2",
        },
    ];

    for test_case in test_cases {
        let network = config.networks.get(test_case.network_name).unwrap();

        let input_asset_config = config.assets.get(test_case.input_asset_name).unwrap();
        let input_asset_assets: Vec<Asset> = input_asset_config
            .assets_list_by_network()
            .unwrap()
            .into_iter()
            .filter(|a| a.network_id() == test_case.network_name)
            .collect();

        let input_asset = input_asset_assets.last().unwrap();

        let output_asset_config = config.assets.get(test_case.output_asset_name).unwrap();
        let output_asset_assets: Vec<Asset> = output_asset_config
            .assets_list_by_network()
            .unwrap()
            .into_iter()
            .filter(|a| a.network_id() == test_case.network_name)
            .collect();

        let output_asset = output_asset_assets.last().unwrap();

        let amount_in = U256::exp10(18);

        let exchange = network
            .get_exchange_by_liquidity(input_asset, output_asset, amount_in)
            .await
            .unwrap();

        assert_eq!(exchange.name(), test_case.exchange_expected);
    }
}
