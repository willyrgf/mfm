pub mod cmd;
pub mod config;

use crate::{
    asset::Asset, config::wallet::Wallet, rebalancer::config::RebalancerConfig,
    utils::blockchain::display_amount_to_float,
};

use num_bigint::{BigInt, Sign};
use serde::{Deserialize, Serialize};
use web3::types::U256;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetBalances {
    //TODO add a equal operation for Asset
    pub(crate) asset: Asset,
    pub(crate) asset_decimals: u8,
    pub(crate) percent: f64,
    pub(crate) balance: U256,
    pub(crate) quoted_asset_decimals: u8,
    pub(crate) quoted_balance: U256,
    pub(crate) quoted_unit_price: U256,
    pub(crate) max_tx_amount: Option<U256>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Kind {
    ToParking,
    FromParking,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetRebalancer {
    pub(crate) kind: Kind,
    pub(crate) rebalancer_config: RebalancerConfig,
    pub(crate) asset_balances: AssetBalances,
    pub(crate) quoted_amount_to_trade: U256,
    pub(crate) asset_amount_to_trade: U256,
    pub(crate) parking_amount_to_trade: U256,
}

impl AssetRebalancer {
    //TODO: refact this initialization to split initialization, build and validations
    pub async fn new(
        kind: Kind,
        rebalancer_config: RebalancerConfig,
        asset_balances: AssetBalances,
        quoted_amount_to_trade: U256,
    ) -> Option<Self> {
        let quoted_asset = rebalancer_config.get_quoted_asset();
        let parking_asset = rebalancer_config.get_parking_asset();

        let asset_exchange = asset_balances.asset
            .get_network()
            .get_exchange_by_liquidity(&asset_balances.asset, &quoted_asset, asset_balances.balance)
            .await.unwrap_or_else(||{
                tracing::error!(
                    "AssetRebalancer::new(): network.get_exchange_by_liquidity(): None, asset_in: {:?}, asset_out: {:?}",
                    &asset_balances.asset,
                    quoted_asset
                );
                panic!()
            });

        let parking_exchange = quoted_asset
            .get_network()
            .get_exchange_by_liquidity(&quoted_asset, &parking_asset, quoted_amount_to_trade)
            .await.unwrap_or_else(||{
                tracing::error!(
                    "AssetRebalancer::new(): network.get_exchange_by_liquidity(): None, asset_in: {:?}, asset_out: {:?}",
                    quoted_asset,
                    parking_asset
                );
                panic!()
            });

        let quoted_asset_path = asset_exchange
            .build_route_for(&quoted_asset, &asset_balances.asset)
            .await;

        let quoted_parking_asset_path = parking_exchange
            .build_route_for(&quoted_asset, &rebalancer_config.get_parking_asset())
            .await;

        let parking_amount_to_trade: U256 = parking_exchange
            .get_amounts_out(quoted_amount_to_trade, quoted_parking_asset_path.clone())
            .await
            .last()
            .unwrap()
            .into();

        let asset_amount_to_trade: U256 = asset_exchange
            .get_amounts_out(quoted_amount_to_trade, quoted_asset_path.clone())
            .await
            .last()
            .unwrap()
            .into();

        Some(Self {
            kind,
            rebalancer_config,
            asset_balances,
            quoted_amount_to_trade,
            asset_amount_to_trade,
            parking_amount_to_trade,
        })
    }

    pub fn display_amount_with_sign(&self, amount: U256, decimals: u8) -> String {
        match self.kind {
            Kind::ToParking => format!("-{}", display_amount_to_float(amount, decimals)),
            Kind::FromParking => format!("+{}", display_amount_to_float(amount, decimals)),
        }
    }
}

impl AssetBalances {
    pub async fn new(rebalancer_config: &RebalancerConfig, asset: Asset) -> AssetBalances {
        let quoted_asset = rebalancer_config.get_quoted_asset();
        let asset_decimals = asset.decimals().await.unwrap();
        let unit_amount = U256::from(1_u32) * U256::exp10(asset_decimals.into());

        let exchange = asset
            .get_network()
            .get_exchange_by_liquidity(&asset, &quoted_asset, unit_amount)
            .await.unwrap_or_else(||{
                tracing::error!(
                    "AssetBalances::new(): network.get_exchange_by_liquidity(): None, asset_in: {:?}, asset_out: {:?}",
                    asset,
                    quoted_asset
                );
                panic!()
            });

        let quoted_asset_path = exchange.build_route_for(&asset, &quoted_asset).await;
        let quoted_unit_price: U256 = exchange
            .get_amounts_out(unit_amount, quoted_asset_path.clone())
            .await
            .last()
            .unwrap()
            .into();

        Self {
            asset: asset.clone(),
            quoted_unit_price,
            asset_decimals,
            percent: rebalancer_config.get_asset_config_percent(asset.name()),
            balance: asset
                .balance_of(rebalancer_config.get_wallet().address())
                .await,
            quoted_asset_decimals: quoted_asset.decimals().await.unwrap(),
            quoted_balance: asset
                .balance_of_quoted_in(rebalancer_config.get_wallet(), &quoted_asset)
                .await,
            max_tx_amount: asset.max_tx_amount().await,
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
    rebalancer_config: &RebalancerConfig,
    assets: Vec<Asset>,
) -> Vec<AssetBalances> {
    futures::future::join_all(
        assets
            .into_iter()
            .map(|asset| AssetBalances::new(rebalancer_config, asset)),
    )
    .await
}

pub async fn add_parking_asset(
    rebalancer_config: &RebalancerConfig,
    ab: Vec<AssetBalances>,
) -> Vec<AssetBalances> {
    let parking_asset = rebalancer_config.get_parking_asset();

    if ab.iter().any(|a| a.asset.name() == parking_asset.name()) {
        return ab;
    }

    let pab = AssetBalances::new(rebalancer_config, parking_asset).await;

    ab.into_iter().chain(vec![pab].into_iter()).collect()
}

pub fn get_total_quoted_balance(assets_balances: &[AssetBalances]) -> U256 {
    let total = assets_balances
        .iter()
        .fold(U256::from(0_i32), |acc, x| acc + x.quoted_balance);

    total
}

pub async fn get_total_parking_balance(parking_asset: &Asset, from_wallet: &Wallet) -> U256 {
    parking_asset.balance_of(from_wallet.address()).await
}

pub async fn move_asset_with_slippage(
    rebalancer_config: &RebalancerConfig,
    asset_in: &Asset,
    asset_out: &Asset,
    mut amount_in: U256,
    mut amount_out: U256,
) {
    let from_wallet = rebalancer_config.get_wallet();
    let balance = asset_in.balance_of(from_wallet.address()).await;
    let exchange = rebalancer_config.get_network().get_exchange_by_liquidity(asset_in, asset_out, amount_in).await
        .unwrap_or_else(|| {
            tracing::error!(
                "move_asset_with_slippage(): network.get_exchange_by_liquidity(): None, asset_in: {:?}, asset_out: {:?}",
                asset_in,
                asset_out
            );
            panic!()
        });

    //TODO: handle with it before in another place
    if balance < amount_in {
        amount_in = balance;
        let p = exchange.build_route_for(asset_in, asset_out).await;
        amount_out = exchange
            .get_amounts_out(amount_in, p)
            .await
            .last()
            .unwrap()
            .into();
    }

    let asset_out_decimals = asset_out.decimals().await.unwrap();
    let amount_in_slippage = asset_in.slippage_u256(asset_out_decimals);
    let amount_out_slippage = asset_out.slippage_u256(asset_out_decimals);

    let slippage = amount_in_slippage + amount_out_slippage;
    let slippage_amount = (amount_out * slippage) / U256::exp10(asset_out_decimals.into());
    let asset_out_amount_slip = amount_out - slippage_amount;
    tracing::debug!("asset_out_amount_slip: {:?}", asset_out_amount_slip);

    exchange
        .swap_tokens_for_tokens(
            from_wallet,
            amount_in,
            asset_out_amount_slip,
            asset_in.clone(),
            asset_out.clone(),
            Some(slippage),
        )
        .await;
}

pub async fn move_assets_to_parking(
    rebalancer_config: &RebalancerConfig,
    assets_balances: &[AssetBalances],
) {
    let parking_asset = rebalancer_config.get_parking_asset();

    //TODO: do it to until all the balance are in the parking asset
    for ab in assets_balances.iter() {
        if ab.asset.name() == rebalancer_config.parking_asset_id() {
            continue;
        }

        let exchange = ab.asset
            .get_network()
            .get_exchange_by_liquidity(&ab.asset, &parking_asset, ab.balance)
            .await.unwrap_or_else(||{
                tracing::error!(
                    "move_assets_to_parking(): network.get_exchange_by_liquidity(): None, asset_in: {:?}, asset_out: {:?}",
                    ab.asset,
                    parking_asset
                );
                panic!()
            });

        let parking_asset_path = exchange.build_route_for(&ab.asset, &parking_asset).await;

        let parking_amount_out: U256 = exchange
            .get_amounts_out(ab.balance, parking_asset_path.clone())
            .await
            .last()
            .unwrap()
            .into();

        let min_move =
            rebalancer_config.parking_asset_min_move_u256(parking_asset.decimals().await.unwrap());
        if min_move >= parking_amount_out {
            tracing::warn!(
                "min_move not satisfied: min_move {}, parking_amount_out {}",
                min_move,
                parking_amount_out
            );
            //TODO: return this error?
            continue;
        }

        move_asset_with_slippage(
            rebalancer_config,
            &ab.asset,
            &parking_asset,
            ab.balance,
            parking_amount_out,
        )
        .await;
    }
}

pub async fn move_parking_to_assets(
    rebalancer_config: &RebalancerConfig,
    assets_balances: &[AssetBalances],
) {
    let from_wallet = rebalancer_config.get_wallet();
    let parking_asset = rebalancer_config.get_parking_asset();
    let parking_asset_decimals = parking_asset.decimals().await.unwrap();

    //TODO: check if doenst exist balance in other assets
    let total_parking_balance = parking_asset.balance_of(from_wallet.address()).await;

    //TODO: do it to until all the parking balance are in the respective assets
    for ab in assets_balances.iter() {
        if ab.asset.name() == rebalancer_config.parking_asset_id() {
            continue;
        }

        let parking_slip = parking_asset.slippage_u256(ab.asset_decimals);
        let parking_amount =
            ab.desired_parking_to_move(total_parking_balance, parking_asset_decimals);
        tracing::debug!("desired_parking_to_move: {}", parking_amount);

        let exchange = ab.asset
            .get_network()
            .get_exchange_by_liquidity(&parking_asset, &ab.asset, parking_amount)
            .await.unwrap_or_else(||{
                tracing::error!(
                    "move_parking_to_assets(): network.get_exchange_by_liquidity(): None, asset_in: {:?}, asset_out: {:?}",
                    parking_asset,
                    ab.asset
                );
                panic!()
            });

        let asset_route = exchange.build_route_for(&parking_asset, &ab.asset).await;

        let asset_amount_out: U256 = exchange
            .get_amounts_out(parking_amount, asset_route.clone())
            .await
            .last()
            .unwrap()
            .into();
        tracing::debug!("asset_amount_out: {:?}", asset_amount_out);

        let ab_slip = ab.asset.slippage_u256(ab.asset_decimals);
        let slippage = ab_slip + parking_slip;
        tracing::debug!("slippage: {:?}", slippage);

        let slippage_amount = (asset_amount_out * slippage) / U256::exp10(ab.asset_decimals.into());
        tracing::debug!("slippage_amount: {:?}", slippage_amount);

        let asset_amount_out_slip = asset_amount_out - slippage_amount;
        tracing::debug!("asset_amount_out_slip: {:?}", asset_amount_out_slip);

        let min_move = rebalancer_config.parking_asset_min_move_u256(parking_asset_decimals);
        if min_move >= parking_amount {
            tracing::error!(
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
    let (_, unb) = b.into_parts();
    let bytes = unb.to_bytes_le();
    U256::from_little_endian(&bytes)
}

//TODO: break validation and threshold
pub async fn validate(config: &RebalancerConfig) -> Result<(), anyhow::Error> {
    if !config.is_valid_portfolio_total_percentage() {
        tracing::error!(
            "rebalancer: {}, sum of portfolio percent should be 100, current is: {}",
            config.name(),
            config.total_percentage()
        );
        panic!()
    }

    let assets = config.get_assets()?;
    let assets_balances = get_assets_balances(config, assets).await;

    if !config.reach_min_threshold(&assets_balances) {
        return Err(anyhow::anyhow!(format!(
            "rebalancer: {}, the minimum threshold configured was not reached",
            config.name()
        )));
    }

    Ok(())
}

pub async fn run_full_parking(config: &RebalancerConfig) -> Result<(), anyhow::Error> {
    //TODO: doc it
    // the general idea of a full parking
    // calc how much we need for each asset
    // calc the diff of expect with current balance per asset
    // swap the balances of all assets to the parking_asset
    // calc quoted_balance of the parking_asset
    // buy from parking_asset the percent of each asset
    // check if the portfolio is balanced
    let from_wallet = config.get_wallet();
    let parking_asset = config.get_parking_asset();
    let assets = config.get_assets()?;
    let assets_balances = get_assets_balances(config, assets).await;

    move_assets_to_parking(config, &assets_balances).await;

    let total_parking_balance = get_total_parking_balance(&parking_asset, from_wallet).await;
    tracing::debug!(
        "run_rebalancer_full_parking(): after move assets_to_parking: total_parking_balance: {}",
        total_parking_balance
    );

    move_parking_to_assets(config, &assets_balances).await;

    let total_parking_balance = get_total_parking_balance(&parking_asset, from_wallet).await;
    tracing::debug!(
        "run_rebalancer_full_parking(): after move parking_to_assets: total_parking_balance: {}",
        total_parking_balance
    );

    Ok(())
}

pub async fn run_diff_parking_per_kind(
    config: &RebalancerConfig,
    kind: Kind,
    ar: Vec<AssetRebalancer>,
) {
    let parking_asset = config.get_parking_asset();

    //TODO: when from_parking, check if weve balance from parking, if not, use all balance
    for ar in ar
        .iter()
        .filter(|ar| ar.kind == kind && ar.asset_balances.asset.name() != parking_asset.name())
    {
        let (asset_in, asset_out, amount_in, amount_out) = match kind {
            Kind::ToParking => (
                &ar.asset_balances.asset,
                &parking_asset,
                ar.asset_amount_to_trade,
                ar.parking_amount_to_trade,
            ),
            Kind::FromParking => (
                &parking_asset,
                &ar.asset_balances.asset,
                ar.parking_amount_to_trade,
                ar.asset_amount_to_trade,
            ),
        };

        let min_move = config
            .parking_asset_min_move_u256(config.get_parking_asset().decimals().await.unwrap());

        if min_move >= ar.parking_amount_to_trade {
            tracing::debug!("run_diff_parking_per_kind(): min_move >= ar.parking_amount_to_trade, min_move: {}, ar.parking_amount_to_trade: {}", min_move, ar.parking_amount_to_trade);
            continue;
        }

        tracing::debug!("diff_parking: parking_to_asset: asset_in.name: {}, asset_out.name: {}, amount_in: {:?}, amount_out: {:?}", asset_in.name(), asset_out.name(), amount_in, amount_out);

        move_asset_with_slippage(config, asset_in, asset_out, amount_in, amount_out).await
    }
}

pub async fn generate_asset_rebalances(
    config: &RebalancerConfig,
) -> Result<Vec<AssetRebalancer>, anyhow::Error> {
    let assets = config.get_assets()?;

    //TODO: add thresould per position

    let mut total = BigInt::from(0_i32);

    let assets_balances = get_assets_balances(config, assets.clone()).await;
    let assets_balances_with_parking = add_parking_asset(config, assets_balances).await;

    //TODO: add a sum_U256 in the module of U256 ops
    let total_quoted_balance = assets_balances_with_parking
        .iter()
        .fold(U256::from(0_i32), |acc, x| acc + x.quoted_balance());

    tracing::debug!(
        "diff_parking: total_quoted_balance: {}",
        total_quoted_balance
    );

    let mut asset_rebalances = vec![];

    let tqb = u256_to_bigint(total_quoted_balance);
    tracing::debug!("diff_parking: tqb: {}", tqb);

    // TODO: break this for in functions to return rp_to_parking rp_from_parking
    for ab in assets_balances_with_parking.clone() {
        let quoted_balance = u256_to_bigint(ab.quoted_balance());
        let diff = tqb.clone() - quoted_balance.clone();

        let pow = 10_u32.pow(4);
        // let percent_diff = (diff.clone() * pow) / quoted_balance.clone();
        let percent: BigInt = ((quoted_balance.clone() * pow) / tqb.clone()) * 100_i32;
        let percent_to_buy = (ab.percent() * 10_f64.powf(4.0)) as u32 - percent.clone();
        // ((2730469751527576947)*((35,68/100)*1e18))/1e18
        // TODO: may add slippage in this calcs
        let amount_to_trade: BigInt = (tqb.clone()
            * (percent_to_buy.clone() * 10_u128.pow((ab.asset_decimals - 4 - 2).into())))
            / 10_u128.pow(ab.asset_decimals.into());

        total += amount_to_trade.clone();

        let quoted_amount_to_trade = bigint_to_u256(amount_to_trade.clone());

        // if amount_to_trade is negative, move to parking
        let kind = if amount_to_trade <= BigInt::from(0_i32) {
            Kind::ToParking
        } else {
            Kind::FromParking
        };

        match AssetRebalancer::new(kind, config.clone(), ab.clone(), quoted_amount_to_trade).await {
            Some(ar) => asset_rebalances.push(ar),
            None => {
                tracing::debug!("diff_parking: rebalancer_parking cant be created, continue.");
                continue;
            }
        };

        tracing::debug!("diff_parking: ab: {}, quoted_balance: {}, ab.percent(): {}, percent: {}, diff: {}, percent_to_buy: {}, amount_to_trade: {}, total: {}",
				ab.asset.name(),
				quoted_balance,
				ab.percent(),
				percent,
				diff,
				percent_to_buy,
				amount_to_trade,
				total,
            );
    }

    Ok(asset_rebalances)
}

pub async fn run_diff_parking(config: &RebalancerConfig) -> Result<(), anyhow::Error> {
    let asset_balances = generate_asset_rebalances(config).await?;

    run_diff_parking_per_kind(config, Kind::ToParking, asset_balances.clone()).await;
    run_diff_parking_per_kind(config, Kind::FromParking, asset_balances.clone()).await;

    Ok(())
}
