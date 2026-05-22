use mfm_program_derive::{MfmConfig, MfmValue, OperationOutput, PublicOutputs, StateInput};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.trybuild",
    name = "asset_price",
    version = "1",
    schema = "mfm.trybuild.asset_price"
)]
struct AssetPrice {
    amount_minor: u64,
    symbol: String,
}

#[derive(Clone, Serialize, Deserialize, MfmConfig)]
#[serde(rename_all = "camelCase")]
struct PortfolioConfig {
    account_id: String,
    latest_price: Option<AssetPrice>,
    weights: BTreeMap<String, u64>,
    pair: (String, u64),
}

#[derive(Clone, Serialize, Deserialize, StateInput)]
struct SnapshotInput {
    prices: Vec<AssetPrice>,
}

#[derive(Clone, Serialize, Deserialize, OperationOutput)]
struct SnapshotOutput {
    price: AssetPrice,
}

#[derive(Clone, Serialize, Deserialize, PublicOutputs)]
struct PublicReport {
    #[serde(rename = "report_id")]
    id: String,
}

fn main() {
    let _ = <AssetPrice as mfm_values::MfmValue>::schema_descriptor().unwrap();
    let _ = <PortfolioConfig as mfm_values::MfmConfig>::schema_descriptor().unwrap();
    let _ = <SnapshotInput as mfm_values::StateInput>::input_schema_descriptor().unwrap();
    let _ = <SnapshotOutput as mfm_values::OperationOutput>::output_schema_descriptor().unwrap();
    let _ = <PublicReport as mfm_values::PublicOutputDescriptor>::public_schema_descriptor().unwrap();
}
