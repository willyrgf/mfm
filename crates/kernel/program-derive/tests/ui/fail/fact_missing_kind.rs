use mfm_program_derive::{MfmFactType, MfmValue};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.trybuild",
    name = "fact_subject",
    version = "1",
    schema = "mfm.trybuild.fact_subject"
)]
struct FactSubject {
    chain: String,
}

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.trybuild",
    name = "fact_response",
    version = "1",
    schema = "mfm.trybuild.fact_response"
)]
struct FactResponse {
    height: u64,
}

#[derive(Clone, Serialize, Deserialize, MfmValue, MfmFactType)]
#[mfm(
    namespace = "mfm.trybuild",
    name = "bad_fact",
    version = "1",
    schema = "mfm.trybuild.bad_fact"
)]
#[mfm_fact(
    field(
        id = "subject.chain",
        source = "subject",
        path = "chain",
        value_type = "string",
        exposure = "returnable"
    )
)]
struct BadFact {
    subject: FactSubject,
    response: FactResponse,
}

fn main() {}
