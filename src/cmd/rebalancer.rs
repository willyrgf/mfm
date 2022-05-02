use crate::{
    asset::Asset,
    cmd,
    config::{
        self,
        rebalancer::{Rebalancer, Strategy},
    },
};

use clap::{ArgMatches, Command};
use num_bigint::{BigInt, Sign};
use web3::types::U256;

pub const REBALANCER_COMMAND: &str = "rebalancer";

pub fn generate_cmd<'a>() -> Command<'a> {
    Command::new(REBALANCER_COMMAND)
        .about("Fires a rebalancer")
        .arg(
            clap::arg!(-n --"name" <REBALANCER_NAME> "Rebalancer name from config file")
                .required(true),
        )
}

#[derive(Debug, Clone)]
pub struct AssetBalances {
    //TODO add a equal operation for Asset
    asset: Asset,
    asset_decimals: u8,
    percent: f64,
    balance: U256,
    quoted_asset_decimals: u8,
    quoted_balance: U256,
}

#[derive(Debug, Clone)]
pub struct RebalancerParking {
    kind: String, //to_parking, from_parking
    rebalancer: Rebalancer,
    asset_balances: AssetBalances,
    quoted_amount_to_trade: U256,
    asset_amount_to_trade: U256,
    parking_amount_to_trade: U256,
}

impl RebalancerParking {
    //TODO: refact this initialization
    pub async fn new(
        rebalancer: Rebalancer,
        kind: String,
        asset_balances: AssetBalances,
        quoted_amount_to_trade: U256,
    ) -> Option<Self> {
        let quoted_asset = rebalancer.get_quoted_asset();

        let quoted_asset_path = asset_balances
            .asset
            .get_exchange()
            .build_route_for(&quoted_asset, &asset_balances.asset)
            .await;

        let quoted_parking_asset_path = rebalancer
            .get_parking_asset()
            .get_exchange()
            .build_route_for(&quoted_asset, &rebalancer.get_parking_asset())
            .await;

        let parking_amount_to_trade: U256 = rebalancer
            .get_parking_asset()
            .get_exchange()
            .get_amounts_out(quoted_amount_to_trade, quoted_parking_asset_path.clone())
            .await
            .last()
            .unwrap()
            .into();

        let asset_amount_to_trade: U256 = asset_balances
            .asset
            .get_exchange()
            .get_amounts_out(quoted_amount_to_trade, quoted_asset_path.clone())
            .await
            .last()
            .unwrap()
            .into();

        let min_move =
            rebalancer.parking_asset_min_move_u256(rebalancer.get_parking_asset().decimals().await);

        log::debug!(
            "RebalancerParking::new(): quoted_amount_to_trade: {} min_move: {}, parking_amount_to_trade: {}",
            quoted_amount_to_trade,
            min_move,
            parking_amount_to_trade
        );

        if min_move >= parking_amount_to_trade {
            return None;
        }

        Some(Self {
            kind,
            rebalancer,
            asset_balances,
            quoted_amount_to_trade,
            asset_amount_to_trade,
            parking_amount_to_trade,
        })
    }
}

impl AssetBalances {
    pub async fn new(rebalancer: &Rebalancer, asset: Asset) -> AssetBalances {
        let quoted_asset = rebalancer.get_quoted_asset();
        Self {
            asset: asset.clone(),
            asset_decimals: asset.decimals().await,
            percent: rebalancer.get_asset_config_percent(asset.name()),
            balance: asset.balance_of(rebalancer.get_wallet().address()).await,
            quoted_asset_decimals: quoted_asset.decimals().await,
            quoted_balance: asset
                .balance_of_quoted_in(rebalancer.get_wallet(), &quoted_asset)
                .await,
        }
    }

    pub fn desired_quoted_in_balance(&self, total_quoted_balance: U256) -> U256 {
        //self.quoted_balance / total_quoted_balance
        let p = ((self.percent / 100.0) * 10_f64.powf(self.quoted_asset_decimals.into())) as u128;
        (total_quoted_balance * U256::from(p)) / U256::exp10(self.quoted_asset_decimals.into())
    }

    pub fn desired_parking_to_move(&self, total_parking_balance: U256, decimals: u8) -> U256 {
        //self.quoted_balance / total_quoted_balance
        let p = ((self.percent / 100.0) * 10_f64.powf(decimals.into())) as u128;
        (total_parking_balance * U256::from(p)) / U256::exp10(decimals.into())
    }

    pub fn quoted_balance(&self) -> U256 {
        self.quoted_balance
    }
    pub fn quoted_asset_decimals(&self) -> u8 {
        self.quoted_asset_decimals
    }
    pub fn percent(&self) -> f64 {
        self.percent
    }
    pub fn quoted_asset_percent_u256(&self) -> U256 {
        U256::from((self.percent * 10_f64.powf(self.quoted_asset_decimals.into())) as u128)
    }
}

pub async fn get_assets_balances(
    rebalancer: &Rebalancer,
    assets: Vec<Asset>,
) -> Vec<AssetBalances> {
    let assets_balances = futures::future::join_all(
        assets
            .into_iter()
            .map(|asset| AssetBalances::new(rebalancer, asset)),
    )
    .await;

    assets_balances
}

pub async fn add_parking_asset(
    rebalancer: &Rebalancer,
    ab: Vec<AssetBalances>,
) -> Vec<AssetBalances> {
    let parking_asset = rebalancer.get_parking_asset();

    if ab.iter().any(|a| a.asset.name() == parking_asset.name()) {
        return ab;
    }

    let pab = AssetBalances::new(rebalancer, parking_asset).await;

    ab.into_iter().chain(vec![pab].into_iter()).collect()
}

pub fn get_total_quoted_balance(assets_balances: &[AssetBalances]) -> U256 {
    let total = assets_balances
        .iter()
        .fold(U256::from(0_i32), |acc, x| acc + x.quoted_balance);

    total
}

pub async fn get_total_parking_balance(
    parking_asset: &Asset,
    from_wallet: &config::wallet::Wallet,
) -> U256 {
    parking_asset.balance_of(from_wallet.address()).await
}

pub async fn move_assets_to_parking(assets_balances: &[AssetBalances], rebalancer: &Rebalancer) {
    let from_wallet = rebalancer.get_wallet();
    let parking_asset = rebalancer.get_parking_asset();
    let parking_asset_decimals = parking_asset.decimals().await;

    //TODO: do it to until all the balance are in the parking asset
    for ab in assets_balances.iter() {
        if ab.asset.name() == rebalancer.parking_asset_id() {
            continue;
        }
        let exchange = ab.asset.get_exchange();
        let parking_slip = parking_asset.slippage_u256(parking_asset_decimals);
        let parking_asset_path = exchange.build_route_for(&ab.asset, &parking_asset).await;

        let parking_amount_out: U256 = exchange
            .get_amounts_out(ab.balance, parking_asset_path.clone())
            .await
            .last()
            .unwrap()
            .into();

        let ab_slip = ab.asset.slippage_u256(parking_asset_decimals);
        let slippage = ab_slip + parking_slip;
        let slippage_amount =
            (parking_amount_out * slippage) / U256::exp10(parking_asset_decimals.into());
        let parking_amount_out_slip = parking_amount_out - slippage_amount;
        log::debug!("parking_amount_out_slip: {:?}", parking_amount_out_slip);

        let min_move = rebalancer.parking_asset_min_move_u256(parking_asset_decimals);
        if min_move >= parking_amount_out_slip {
            log::error!(
                "min_move not sattisfied: min_move {}, parking_amounts_out {}",
                min_move,
                parking_amount_out_slip
            );
            continue;
        }

        exchange
            .swap_tokens_for_tokens(
                from_wallet,
                ab.balance,
                parking_amount_out_slip,
                ab.asset.clone(),
                parking_asset.clone(),
                Some(slippage),
            )
            .await;
    }
}

pub async fn move_parking_to_assets(assets_balances: &[AssetBalances], rebalancer: &Rebalancer) {
    let from_wallet = rebalancer.get_wallet();
    let parking_asset = rebalancer.get_parking_asset();
    let parking_asset_decimals = parking_asset.decimals().await;

    //TODO: check if doenst exist balance in other assets
    let total_parking_balance = parking_asset.balance_of(from_wallet.address()).await;

    //TODO: do it to until all the parking balance are in the respective assets
    for ab in assets_balances.iter() {
        if ab.asset.name() == rebalancer.parking_asset_id() {
            continue;
        }
        let exchange = ab.asset.get_exchange();
        let asset_route = exchange.build_route_for(&parking_asset, &ab.asset).await;
        let parking_slip = parking_asset.slippage_u256(ab.asset_decimals);
        let parking_amount =
            ab.desired_parking_to_move(total_parking_balance, parking_asset_decimals);
        log::debug!("desired_parking_to_move: {}", parking_amount);

        let asset_amount_out: U256 = exchange
            .get_amounts_out(parking_amount, asset_route.clone())
            .await
            .last()
            .unwrap()
            .into();
        log::debug!("asset_amount_out: {:?}", asset_amount_out);

        let ab_slip = ab.asset.slippage_u256(ab.asset_decimals);
        let slippage = ab_slip + parking_slip;
        log::debug!("slippage: {:?}", slippage);

        let slippage_amount = (asset_amount_out * slippage) / U256::exp10(ab.asset_decimals.into());
        log::debug!("slippage_amount: {:?}", slippage_amount);

        let asset_amount_out_slip = asset_amount_out - slippage_amount;
        log::debug!("asset_amount_out_slip: {:?}", asset_amount_out_slip);

        let min_move = rebalancer.parking_asset_min_move_u256(parking_asset_decimals);
        if min_move >= parking_amount {
            log::error!(
                "min_move not sattisfied: min_move {}, parking_amounts_out {}",
                min_move,
                parking_amount
            );
            continue;
        }

        exchange
            .swap_tokens_for_tokens(
                from_wallet,
                parking_amount,
                asset_amount_out_slip,
                parking_asset.clone(),
                ab.asset.clone(),
                Some(slippage),
            )
            .await;
    }
}

pub async fn call_sub_commands(args: &ArgMatches) {
    let rebalancer = cmd::helpers::get_rebalancer(args);
    log::debug!("rebalancer: {:?}", rebalancer);

    if !rebalancer.is_valid_portfolio_total_percentage() {
        log::error!(
            "rebalancer: {}, sum of portfolio percent should be 100, current is: {}",
            rebalancer.name(),
            rebalancer.total_percentage()
        );
        panic!()
    }

    let assets = rebalancer.get_assets();
    let assets_balances = get_assets_balances(rebalancer, assets).await;
    log::debug!("assets_balances: {:?}", assets_balances);

    if !rebalancer.reach_min_threshold(&assets_balances) {
        log::error!(
            "rebalancer: {}, the minimum threshold configured was not reached",
            rebalancer.name()
        );
        panic!();
    }

    let total_quoted_balance: U256 = get_total_quoted_balance(&assets_balances);
    log::debug!("total_quoted_balance: {:?}", total_quoted_balance);

    let from_wallet = rebalancer.get_wallet();
    let parking_asset = rebalancer.get_parking_asset();

    //TODO: consider parking_asset to get the total balance and rebalance
    match rebalancer.strategy() {
        Strategy::FullParking => {
            //TODO: doc it
            // the general idea of a full parking
            // calc how much we need for each asset
            // calc the diff of expect with current balance per asset
            // swap the balances of all assets to the parking_asset
            // calc quoted_balance of the parking_asset
            // buy from parking_asset the percent of each asset
            // check if the portfolio is balanced

            // move all balances to parking asset
            move_assets_to_parking(&assets_balances, rebalancer).await;

            let total_parking_balance =
                get_total_parking_balance(&parking_asset, from_wallet).await;
            log::debug!("total_parking_balance: {}", total_parking_balance);

            //move parking to assets
            move_parking_to_assets(&assets_balances, rebalancer).await;
        }
        Strategy::DiffParking => {
            log::debug!("rebalancer::call_sub_commands() Strategy::DiffParking");

            // let t
            // ab.
            let total_quoted_balance = assets_balances
                .iter()
                .fold(U256::from(0_i32), |acc, x| acc + x.quoted_balance());

            log::debug!(
                "diff_parking: total_quoted_balance: {}",
                total_quoted_balance
            );

            let tqb = u256_to_bigint(total_quoted_balance);
            log::debug!("diff_parking: tqb: {}", tqb);

            let mut total = BigInt::from(0_i32);

            let assets_balances_with_parking = add_parking_asset(rebalancer, assets_balances).await;
            let mut asset_to_parking = vec![];
            for ab in assets_balances_with_parking {
                let quoted_balance = u256_to_bigint(ab.quoted_balance());
                let diff = tqb.clone() - quoted_balance.clone();

                let pow = 10_u32.pow(4);
                // let percent_diff = (diff.clone() * pow) / quoted_balance.clone();
                let percent: BigInt = ((quoted_balance.clone() * pow) / tqb.clone()) * 100_i32;
                let percent_to_buy = (ab.percent() * 10_f64.powf(4.0)) as u32 - percent.clone();
                // ((2730469751527576947)*((35,68/100)*1e18))/1e18
                let amount_to_trade: BigInt = (tqb.clone()
                    * (percent_to_buy.clone() * 10_u128.pow((ab.asset_decimals - 4 - 2).into())))
                    / 10_u128.pow(ab.asset_decimals.into());

                total += amount_to_trade.clone();

                let quoted_amount_to_trade = bigint_to_u256(amount_to_trade.clone());

                if amount_to_trade < BigInt::from(0_i32) {
                    match RebalancerParking::new(
                        rebalancer.clone(),
                        "to_parking".to_string(),
                        ab.clone(),
                        quoted_amount_to_trade,
                    )
                    .await
                    {
                        Some(rp) => asset_to_parking.push(rp),
                        None => {
                            log::info!(
                                "diff_parking: rebalancer_parking cant be created, continue."
                            );
                            continue;
                        }
                    }
                }

                log::debug!(
                    "diff_parking: ab: {}, quoted_balance: {}, ab.percent(): {}, percent: {}, diff: {}, percent_to_buy: {}, amount_to_trade: {}, total: {}",
                    ab.asset.name(),
                    quoted_balance,
                    ab.percent(),
                    percent,
                    diff,
                    percent_to_buy,
                    amount_to_trade,
                    total,
                );

                // log::debug!("diff_parking: percent_to_buy: {}", percent_to_buy);
                // log::debug!("diff_parking: amount_to_trade: {}", amount_to_trade)
            }

            log::debug!("diff_parking: asset_to_parking: {:?}", asset_to_parking);

            unimplemented!()
        }
    };
}

//TODO: create a mod to carry all the U256 ops
// https://github.com/graphprotocol/graph-node/blob/master/graph/src/data/store/scalar.rs
// U256 -> BigUint
// BigUint -> U256
pub fn u256_to_bigint(u: U256) -> BigInt {
    let mut bytes: [u8; 32] = [0; 32];
    u.to_little_endian(&mut bytes);
    BigInt::from_bytes_le(Sign::Plus, &bytes)
}

pub fn bigint_to_u256(b: BigInt) -> U256 {
    let bytes = b.to_signed_bytes_le();
    if b < BigInt::from(0) {
        assert!(
            bytes.len() <= 32,
            "BigInt value does not fit into signed U256"
        );
        let mut i_bytes: [u8; 32] = [255; 32];
        i_bytes[..bytes.len()].copy_from_slice(&bytes);
        U256::from_little_endian(&i_bytes)
    } else {
        U256::from_little_endian(&bytes)
    }
}

/*

wbnb, wbtc, weth

busd

 */
