fn main() {
    let _ = core::mem::size_of::<mfm_runtime::StagedArtifactHandle>();
    let _ = core::mem::size_of::<mfm_runtime::StagedArtifactBindingKind>();
    let _ = core::mem::size_of::<mfm_runtime::StagedSideEffectArtifactPhase>();
    let _ = mfm_runtime::StagedArtifact::handle;
}
