use super::*;

#[test]
fn executable_identity_summary_matches_golden() {
    assert_eq!(
        executable_identity_summary([READ_FACTORY, PURE_FACTORY]),
        [
            "factory=read_external;cargo_digest=content:sha256-jcs-v1:9cee33a7e03231e1baeb99725c9724361ba5763bc7f3cbe11698fa96696e6f1a;binary_digest=content:sha256-jcs-v1:ded559fbdf36801d1e3c95838014c5e7b94e50655014d7ddc612f288a508988e;nix_derivation=false;nix_output=false",
            "factory=pure;cargo_digest=content:sha256-jcs-v1:9cee33a7e03231e1baeb99725c9724361ba5763bc7f3cbe11698fa96696e6f1a;binary_digest=content:sha256-jcs-v1:ded559fbdf36801d1e3c95838014c5e7b94e50655014d7ddc612f288a508988e;nix_derivation=false;nix_output=false",
        ]
    );
}

#[test]
fn decode_config_bytes_rejects_invalid_serialized_config() {
    let error = decode_config_bytes::<ProjectReportConfig>(br#"{"report_version":0}"#)
        .expect_err("zero report version must fail decoding");
    let mfm_runtime::RuntimeError::InvalidRunnerOutput(message) = error else {
        panic!("unexpected error: {error}");
    };
    assert!(message.contains("nonzero u64"));
}

fn executable_identity_summary(factories: [&str; 2]) -> Vec<String> {
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
