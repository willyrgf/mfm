use mfm_program_derive::{MfmFactType, MfmValue};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.trybuild",
    name = "unknown_ordering_subject",
    version = "1",
    schema = "mfm.trybuild.unknown_ordering_subject"
)]
struct FactSubject {
    chain: String,
}

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.trybuild",
    name = "unknown_ordering_response",
    version = "1",
    schema = "mfm.trybuild.unknown_ordering_response"
)]
struct FactResponse {
    height: u64,
}

#[derive(Clone, Serialize, Deserialize, MfmValue, MfmFactType)]
#[mfm(
    namespace = "mfm.trybuild",
    name = "unknown_ordering_fact",
    version = "1",
    schema = "mfm.trybuild.unknown_ordering_fact"
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
    ordering(
        name = "result.height.desc",
        term(field = "result.height", direction = "descending", nulls = "last")
    )
)]
struct BadFact {
    subject: FactSubject,
    response: FactResponse,
}

fn main() {}
