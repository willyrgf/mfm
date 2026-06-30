use super::*;

#[test]
fn executable_identity_summary_matches_golden() {
    assert_eq!(
        executable_identity_summary([READ_FACTORY, SIDE_EFFECT_FACTORY, PURE_FACTORY]),
        [
            "factory=read_external;cargo_digest=content:sha256-jcs-v1:8e5756e097f23f2a6d5fe8c69846ba7ca609c5a7816528a85d7718cd760357f2;binary_digest=content:sha256-jcs-v1:67de41659eff846aa936862dfb86e7bb6acc9405aeb40f46bddfc4e287cb7f1b;nix_derivation=false;nix_output=false",
            "factory=apply_side_effect;cargo_digest=content:sha256-jcs-v1:8e5756e097f23f2a6d5fe8c69846ba7ca609c5a7816528a85d7718cd760357f2;binary_digest=content:sha256-jcs-v1:67de41659eff846aa936862dfb86e7bb6acc9405aeb40f46bddfc4e287cb7f1b;nix_derivation=false;nix_output=false",
            "factory=pure;cargo_digest=content:sha256-jcs-v1:8e5756e097f23f2a6d5fe8c69846ba7ca609c5a7816528a85d7718cd760357f2;binary_digest=content:sha256-jcs-v1:67de41659eff846aa936862dfb86e7bb6acc9405aeb40f46bddfc4e287cb7f1b;nix_derivation=false;nix_output=false",
        ]
    );
}

fn executable_identity_summary(factories: [&str; 3]) -> Vec<String> {
    factories
        .into_iter()
        .map(|factory| {
            let identity = executable(events::RunnerFactoryId::new(factory).expect("factory id"))
                .expect("executable identity");
            format!(
                "factory={};cargo_digest={};binary_digest={};nix_derivation={};nix_output={}",
                identity.factory_id,
                identity.cargo_package_digest,
                identity.binary_digest,
                identity.nix_derivation_hash.is_some(),
                identity.nix_output_hash.is_some()
            )
        })
        .collect()
}
