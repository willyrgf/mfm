use mfm_program_derive::{MfmFactType, MfmValue};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.trybuild",
    name = "optional_fact_subject",
    version = "1",
    schema = "mfm.trybuild.optional_fact_subject"
)]
struct FactSubject {
    chain: String,
}

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.trybuild",
    name = "optional_fact_response",
    version = "1",
    schema = "mfm.trybuild.optional_fact_response"
)]
struct FactResponse {
    height: u64,
}

#[derive(Clone, Serialize, Deserialize, MfmValue, MfmFactType)]
#[mfm(
    namespace = "mfm.trybuild",
    name = "optional_subject_fact",
    version = "1",
    schema = "mfm.trybuild.optional_subject_fact"
)]
#[mfm_fact(
    kind = "chain.head",
    field(
        id = "subject.chain",
        source = "subject",
        path = "chain",
        value_type = "string",
        exposure = "returnable",
        optional
    )
)]
struct BadFact {
    subject: FactSubject,
    response: FactResponse,
}

fn main() {}
