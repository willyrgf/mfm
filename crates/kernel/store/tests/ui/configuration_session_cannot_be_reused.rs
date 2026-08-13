use mfm_ids::AppendRequestId;
use mfm_store::ConfigurationWriteSession;
use mfm_values::ValidatedConfig;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, mfm_program_derive::MfmConfig)]
#[serde(deny_unknown_fields)]
struct Config {}

fn reuse(
    session: ConfigurationWriteSession<Config>,
    first: ValidatedConfig<Config>,
    second: ValidatedConfig<Config>,
) {
    let _ = session.prepare_local(
        AppendRequestId::new("configuration-session-first-00001").unwrap(),
        first,
    );
    let _ = session.prepare_local(
        AppendRequestId::new("configuration-session-second-0001").unwrap(),
        second,
    );
}

fn main() {}
