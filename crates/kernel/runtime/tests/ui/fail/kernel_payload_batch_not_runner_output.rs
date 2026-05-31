use mfm_events::v1 as events;
use mfm_runtime::ErasedRunnerOutput;

fn run_completed() -> events::RunCompleted {
    unimplemented!()
}

fn main() {
    let _ = ErasedRunnerOutput::new(vec![events::KernelEventPayload::RunCompleted(
        run_completed(),
    )]);
}
