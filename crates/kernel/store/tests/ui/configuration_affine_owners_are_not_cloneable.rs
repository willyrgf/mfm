use mfm_store::{
    ConfigurationWriteSession, PreparedConfigurationAppend, ResolvedConfiguration,
    SuspendedConfigurationAppend,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, mfm_program_derive::MfmConfig)]
#[serde(deny_unknown_fields)]
struct Config {}

fn requires_clone<T: Clone>() {}

fn main() {
    requires_clone::<ResolvedConfiguration<Config>>();
    requires_clone::<ConfigurationWriteSession<Config>>();
    requires_clone::<PreparedConfigurationAppend<Config>>();
    requires_clone::<SuspendedConfigurationAppend<Config>>();
}
