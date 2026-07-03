use mfm_program::MfmFactType as _;
use mfm_program_derive::{
    MfmConfig, MfmFactType, MfmValue, OperationOutput, PublicOutputs, StateInput,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
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

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
enum PriceKind {
    Spot,
    Twap,
}

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum PriceSource {
    Fixed { price: AssetPrice },
    Oracle { source_id: String },
}

#[derive(Clone, Serialize, Deserialize, MfmConfig)]
#[serde(rename_all = "camelCase")]
struct PortfolioConfig {
    account_id: String,
    latest_price: Option<AssetPrice>,
    price_kind: PriceKind,
    price_source: PriceSource,
    weights: BTreeMap<String, u64>,
    pair: (String, u64),
}

#[derive(Clone, Serialize, Deserialize, StateInput)]
struct SnapshotInput {
    prices: Vec<AssetPrice>,
}

#[derive(Clone, Serialize, Deserialize, StateInput)]
struct EmptyInput {}

#[derive(Clone, Serialize, Deserialize, OperationOutput)]
struct SnapshotOutput {
    price: AssetPrice,
}

#[derive(Clone, Serialize, Deserialize, PublicOutputs)]
struct PublicReport {
    #[serde(rename = "report_id")]
    id: String,
}

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.trybuild",
    name = "chain_head_subject",
    version = "1",
    schema = "mfm.trybuild.chain_head_subject"
)]
struct ChainHeadSubject {
    chain: String,
}

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.trybuild",
    name = "chain_head_response",
    version = "1",
    schema = "mfm.trybuild.chain_head_response"
)]
struct ChainHeadResponse {
    height: u64,
}

#[allow(clippy::duplicated_attributes)]
#[derive(Clone, Serialize, Deserialize, MfmValue, MfmFactType)]
#[mfm(
    namespace = "mfm.trybuild",
    name = "chain_head_fact",
    version = "1",
    schema = "mfm.trybuild.chain_head_fact"
)]
#[mfm_fact(kind = "chain.head")]
#[mfm_fact(field(
    id = "subject.chain",
    source = "subject",
    path = "chain",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.height",
    source = "result",
    path = "height",
    value_type = "unsigned_integer",
    operators(equal, greater_than),
    exposure = "returnable",
    sortable
))]
#[mfm_fact(ordering(
    name = "result.height.desc",
    term(field = "result.height", direction = "descending", nulls = "last")
))]
struct ChainHeadFact {
    subject: ChainHeadSubject,
    response: ChainHeadResponse,
}

fn main() {
    let _ = <AssetPrice as mfm_values::MfmValue>::schema_descriptor().unwrap();
    let _ = <PriceKind as mfm_values::MfmValue>::schema_descriptor().unwrap();
    let _ = <PriceSource as mfm_values::MfmValue>::schema_descriptor().unwrap();
    let _ = <PortfolioConfig as mfm_values::MfmConfig>::schema_descriptor().unwrap();
    let _ = <SnapshotInput as mfm_values::StateInput>::input_schema_descriptor().unwrap();
    let _ = <EmptyInput as mfm_values::StateInput>::input_schema_descriptor().unwrap();
    let _ = <SnapshotOutput as mfm_values::OperationOutput>::output_schema_descriptor().unwrap();
    let _ = <PublicReport as mfm_values::PublicOutputDescriptor>::public_schema_descriptor().unwrap();
    let fact = ChainHeadFact {
        subject: ChainHeadSubject {
            chain: "bitcoin".to_owned(),
        },
        response: ChainHeadResponse { height: 850_000 },
    };
    let _ = ChainHeadFact::descriptor().unwrap();
    let _ = fact.subject();
    let _ = fact.response();
}
