use mfm_program_derive::{MfmFactType, MfmValue};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.trybuild",
    name = "duplicate_fact_subject",
    version = "1",
    schema = "mfm.trybuild.duplicate_fact_subject"
)]
struct FactSubject {
    chain: String,
}

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.trybuild",
    name = "duplicate_fact_response",
    version = "1",
    schema = "mfm.trybuild.duplicate_fact_response"
)]
struct FactResponse {
    height: u64,
}

#[derive(Clone, Serialize, Deserialize, MfmValue, MfmFactType)]
#[mfm(
    namespace = "mfm.trybuild",
    name = "duplicate_fact",
    version = "1",
    schema = "mfm.trybuild.duplicate_fact"
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
        id = "subject.chain",
        source = "result",
        path = "height",
        value_type = "unsigned_integer",
        exposure = "returnable"
    )
)]
struct BadFact {
    subject: FactSubject,
    response: FactResponse,
}

fn main() {}
