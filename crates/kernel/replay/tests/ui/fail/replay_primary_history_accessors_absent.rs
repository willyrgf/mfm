fn inspect(broker: &mfm_replay::v1::ReplayBroker<'_>) {
    let _ = broker.events();
    let _ = broker.projection_snapshot();
}

fn main() {}
