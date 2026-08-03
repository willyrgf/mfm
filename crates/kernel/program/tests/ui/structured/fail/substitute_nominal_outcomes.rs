use mfm_spec::structured::{LaneOutcome, OperationOutcome, StateOutcome};

fn state_as_lane(value: StateOutcome<u64, u64>) -> LaneOutcome<u64, u64> {
    value
}

fn lane_as_operation(value: LaneOutcome<u64, u64>) -> OperationOutcome<u64, u64> {
    value
}

fn operation_as_state(value: OperationOutcome<u64, u64>) -> StateOutcome<u64, u64> {
    value
}

fn main() {}
