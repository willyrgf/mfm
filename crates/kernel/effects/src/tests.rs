use super::*;

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
    assert_effect::<ManagedPlatformWrite>(
        EffectClass::ManagedPlatformWrite,
        "managed_platform_write",
        "effect:mfm.kernel.effect:managed_platform_write:sha256-jcs-v1:40d491129f1350c3b54cf50db658bd6a9bb0cadaeb8098252c7e4bcc4897192f",
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
    assert_eq!(
        EffectClass::ManagedPlatformWrite.as_str(),
        "managed_platform_write"
    );
    assert_eq!(EffectClass::ApplySideEffect.as_str(), "apply_side_effect");
}
