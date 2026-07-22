use super::*;
use mfm_ids::{DigestAlgorithm, DigestBytes};

fn assert_effect<E: EffectSpec>(
    expected_class: EffectClass,
    expected_name: &'static str,
    expected_kind: &'static str,
) {
    let descriptor = E::descriptor().expect("descriptor");

    assert_eq!(descriptor.class, expected_class);
    assert_eq!(descriptor.name, expected_name);
    assert_eq!(descriptor.kind.as_str(), expected_kind);
    assert_eq!(descriptor.version.as_str(), "mfm.effect.v1");
    assert_eq!(E::kind().expect("kind"), descriptor.kind);
    assert_eq!(E::version().expect("version"), descriptor.version);
    assert_eq!(E::class(), descriptor.class);
}

#[test]
fn framework_effect_markers_have_stable_descriptors() {
    assert_effect::<Pure>(
        EffectClass::Pure,
        "pure",
        "effect:mfm.kernel.effect:pure:sha256-jcs-v1:a54d4bec4951dc5e829fa22cf2f0305c45c5151a1b3cfe965477734fb9996559",
    );
    assert_effect::<ReadExternal>(
        EffectClass::ReadExternal,
        "read_external",
        "effect:mfm.kernel.effect:read_external:sha256-jcs-v1:57b1f6b604d4d724bdf593d53f397f88661f07c17cd2ed036e454f66192a6502",
    );
    assert_effect::<ApplySideEffect>(
        EffectClass::ApplySideEffect,
        "apply_side_effect",
        "effect:mfm.kernel.effect:apply_side_effect:sha256-jcs-v1:a007caa404540427f9773d9d3d7b1ff763ed7d3449a53f28cc503403e37d0258",
    );
}

#[test]
fn effect_class_strings_are_stable() {
    assert_eq!(EffectClass::Pure.as_str(), "pure");
    assert_eq!(EffectClass::ReadExternal.as_str(), "read_external");
    assert_eq!(EffectClass::ApplySideEffect.as_str(), "apply_side_effect");
}

struct ReadRpc;
struct SupportKeystore;
struct MutationSubmitter;

impl CapabilitySpec for ReadRpc {
    type Role = ReadExternalRole;

    fn kind() -> Result<CapabilityKind> {
        capability_kind("read_rpc", 0x11)
    }

    fn version() -> Result<CapabilityVersion> {
        capability_version()
    }

    fn name() -> &'static str {
        "read_rpc"
    }
}

impl CapabilitySpec for SupportKeystore {
    type Role = SupportRole;

    fn kind() -> Result<CapabilityKind> {
        capability_kind("support_keystore", 0x22)
    }

    fn version() -> Result<CapabilityVersion> {
        capability_version()
    }

    fn name() -> &'static str {
        "support_keystore"
    }
}

impl CapabilitySpec for MutationSubmitter {
    type Role = ExternalMutationAuthorityRole;

    fn kind() -> Result<CapabilityKind> {
        capability_kind("mutation_submitter", 0x44)
    }

    fn version() -> Result<CapabilityVersion> {
        capability_version()
    }

    fn name() -> &'static str {
        "mutation_submitter"
    }
}

fn capability_kind(name: &str, byte: u8) -> Result<CapabilityKind> {
    CapabilityKind::new(
        "mfm.test",
        name,
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    )
    .map_err(|error| CapabilityError::Identity(error.to_string()))
}

fn capability_version() -> Result<CapabilityVersion> {
    CapabilityVersion::new("mfm.capability.v1")
        .map_err(|error| CapabilityError::Identity(error.to_string()))
}

fn assert_capability_set_for<E, C>()
where
    E: EffectSpec,
    C: CapabilitySetFor<E>,
{
}

#[test]
fn valid_effect_capability_sets_compile_and_describe() {
    assert_capability_set_for::<Pure, NoCaps>();
    assert_capability_set_for::<ReadExternal, (ReadRpc,)>();
    assert_capability_set_for::<ReadExternal, (ReadRpc, SupportKeystore)>();
    assert_capability_set_for::<ApplySideEffect, (MutationSubmitter,)>();
    assert_capability_set_for::<ApplySideEffect, (ReadRpc, MutationSubmitter, SupportKeystore)>();

    let read_descriptor =
        <(ReadRpc, SupportKeystore) as CapabilitySet>::descriptor().expect("read descriptor");
    assert_eq!(read_descriptor.len(), 2);
    assert_eq!(
        read_descriptor.capabilities[0].kind.as_str(),
        "capability:mfm.test:read_rpc:sha256-jcs-v1:1111111111111111111111111111111111111111111111111111111111111111"
    );
    assert_eq!(
        read_descriptor.capabilities[0].version.as_str(),
        "mfm.capability.v1"
    );
    assert_eq!(
        read_descriptor.capabilities[0].role,
        CapabilityRole::ReadExternal
    );
    read_descriptor
        .validate_for_effect::<ReadExternal>()
        .expect("read roles valid");
}

#[test]
fn descriptor_role_validation_matches_v1_effect_rules() {
    let no_caps = NoCaps::descriptor().expect("no caps descriptor");
    assert!(no_caps.is_empty());
    no_caps
        .validate_for_effect::<Pure>()
        .expect("pure accepts no caps");
    assert!(matches!(
        no_caps
            .validate_for_effect::<ApplySideEffect>()
            .expect_err("side effect needs one mutation authority"),
        CapabilityError::InvalidCapabilitySet { .. }
    ));

    let read_with_mutation =
        CapabilitySetDescriptor::new(vec![MutationSubmitter::descriptor().expect("mutation cap")])
            .expect("descriptor builds");
    assert!(read_with_mutation
        .validate_for_effect::<ReadExternal>()
        .is_err());

    let two_mutation_authorities = CapabilitySetDescriptor::new(vec![
        MutationSubmitter::descriptor().expect("mutation cap"),
        CapabilityDescriptor::new(
            capability_kind("mutation_submitter_backup", 0x55).expect("backup kind"),
            capability_version().expect("capability version"),
            CapabilityRole::ExternalMutationAuthority,
            "mutation_submitter_backup",
        )
        .expect("backup descriptor"),
    ])
    .expect("descriptor builds");
    assert!(two_mutation_authorities
        .validate_for_effect::<ApplySideEffect>()
        .is_err());
}

#[test]
fn duplicate_capability_descriptors_reject() {
    let error = <(ReadRpc, ReadRpc) as CapabilitySet>::descriptor()
        .expect_err("duplicate kind/version must reject");

    assert!(matches!(error, CapabilityError::DuplicateCapability { .. }));
}

#[test]
fn capability_role_strings_are_stable() {
    assert_eq!(CapabilityRole::ReadExternal.as_str(), "read_external");
    assert_eq!(CapabilityRole::Support.as_str(), "support");
    assert_eq!(
        CapabilityRole::ExternalMutationAuthority.as_str(),
        "external_mutation_authority"
    );
}
