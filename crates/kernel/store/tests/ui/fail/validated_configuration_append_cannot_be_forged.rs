//! A downstream caller cannot construct a validated configuration append.
use mfm_store::structured::{ConfigurationRevisionObject, ValidatedConfigurationAppend};

fn forge_configuration(object: ConfigurationRevisionObject) {
    let _ = ValidatedConfigurationAppend::from_object(object);
}

fn main() {}
