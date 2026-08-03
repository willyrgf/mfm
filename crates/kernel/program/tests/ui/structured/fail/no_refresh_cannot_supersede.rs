use mfm_capabilities::{EffectAdapterCompletion, NoRefreshEvidence};

fn main() {
    let evidence = NoRefreshEvidence {};
    let _: EffectAdapterCompletion<(), (), NoRefreshEvidence> =
        EffectAdapterCompletion::SupersededBeforeEntry(evidence);
}
