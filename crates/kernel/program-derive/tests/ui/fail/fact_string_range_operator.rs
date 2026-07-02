use mfm_program_derive::{MfmFactType, MfmValue};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.trybuild",
    name = "range_fact_subject",
    version = "1",
    schema = "mfm.trybuild.range_fact_subject"
)]
struct FactSubject {
    chain: String,
}

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.trybuild",
    name = "range_fact_response",
    version = "1",
    schema = "mfm.trybuild.range_fact_response"
)]
struct FactResponse {
    label: String,
}

#[derive(Clone, Serialize, Deserialize, MfmValue, MfmFactType)]
#[mfm(
    namespace = "mfm.trybuild",
    name = "string_range_fact",
    version = "1",
    schema = "mfm.trybuild.string_range_fact"
)]
#[mfm_fact(
    kind = "chain.head",
    field(
        id = "subject.chain",
        source = "subject",
        path = "chain",
        value_type = "string",
        exposure = "returnable"
    ),
    field(
        id = "result.label",
        source = "result",
        path = "label",
        value_type = "string",
        operator = "greater_than",
        exposure = "returnable"
    )
)]
struct BadFact {
    subject: FactSubject,
    response: FactResponse,
}

fn main() {}
