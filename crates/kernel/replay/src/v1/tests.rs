use super::*;
use mfm_store::v1::test_support::{
    persisted_kernel_event_envelope_for_test as store_persisted_kernel_event_envelope_for_test,
    persisted_kernel_event_envelope_with_ordinal_for_test as store_persisted_kernel_event_envelope_with_ordinal_for_test,
    run_artifact_ref_from_store_artifact_for_test,
};

#[path = "side_effect_fixtures.rs"]
mod side_effect_fixtures;
use self::side_effect_fixtures::*;
#[path = "fact_fixtures.rs"]
mod fact_fixtures;
use self::fact_fixtures::*;
#[path = "cell_fixtures.rs"]
mod cell_fixtures;
use self::cell_fixtures::*;
#[path = "fact_stream_fixtures.rs"]
mod fact_stream_fixtures;
use self::fact_stream_fixtures::*;
#[path = "primitive_fixtures.rs"]
mod primitive_fixtures;
use self::primitive_fixtures::*;

#[path = "tests/behavior.rs"]
mod behavior;
