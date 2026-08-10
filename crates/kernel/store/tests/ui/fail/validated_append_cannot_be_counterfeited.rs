//! A downstream caller can neither consume a validated run append without the
//! workspace backend seal nor construct a validated configuration append.
use mfm_store::structured::{
    ConfigurationRevisionObject, ValidatedConfigurationAppend, ValidatedRunAppend,
};

fn discard_required_plans(command: ValidatedRunAppend) {
    let _ = command.into_parts(&());
}

fn discard_configuration_head_plan(command: ValidatedConfigurationAppend) {
    let _ = command.into_parts(&());
}

fn forge_configuration(object: ConfigurationRevisionObject) {
    let _ = ValidatedConfigurationAppend::from_object(object);
}

fn main() {}
