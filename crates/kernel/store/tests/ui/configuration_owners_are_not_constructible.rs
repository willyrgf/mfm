use mfm_store::{
    ConfigurationWriteSession, PreparedConfigurationAppend, ResolvedConfiguration,
    ResolvedConfigurationHead, SuspendedConfigurationAppend,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, mfm_program_derive::MfmConfig)]
#[serde(deny_unknown_fields)]
struct Config {}

fn main() {
    let _ = ResolvedConfigurationHead {};
    let _ = ResolvedConfiguration::<Config> {};
    let _ = ConfigurationWriteSession::<Config> {};
    let _ = PreparedConfigurationAppend::<Config> {};
    let _ = SuspendedConfigurationAppend::<Config> {};
}
