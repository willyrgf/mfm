//! Shared metadata builders for reusable states.

use mfm_machine::meta::{DependencyStrategy, Idempotency, SideEffectKind, StateMeta, Tag};

/// Stable tags used by reusable shared states.
pub mod tags {
    /// Tag for states that apply an external side effect.
    pub const APPLY_SIDE_EFFECT: &str = "apply_side_effect";
    /// Tag for configuration-loading or validation states.
    pub const CONFIG: &str = "config";
    /// Tag for execution states.
    pub const EXECUTE: &str = "execute";
    /// Tag for data-fetching states.
    pub const FETCH_DATA: &str = "fetch_data";
    /// Tag for read-only IO states that do not mutate external systems.
    pub const READ_ONLY_IO: &str = "read_only_io";
    /// Tag for validation-only states.
    pub const VALIDATE: &str = "validate";
}

fn mk_meta(tags: Vec<Tag>, side_effects: SideEffectKind, idempotency: Idempotency) -> StateMeta {
    StateMeta {
        tags,
        depends_on: Vec::new(),
        depends_on_strategy: DependencyStrategy::Latest,
        side_effects,
        idempotency,
    }
}

/// Returns metadata for a read-only data-fetching state.
pub fn fetch_data() -> StateMeta {
    read_only_io_with_tag(tags::FETCH_DATA)
}

/// Returns metadata for a read-only validation state.
pub fn validate() -> StateMeta {
    read_only_io_with_tag(tags::VALIDATE)
}

/// Returns metadata for a pure state with no default tags.
pub fn pure() -> StateMeta {
    mk_meta(Vec::new(), SideEffectKind::Pure, Idempotency::None)
}

/// Returns metadata for a read-only IO state with no default tags.
pub fn read_only_io() -> StateMeta {
    mk_meta(Vec::new(), SideEffectKind::ReadOnlyIo, Idempotency::None)
}

/// Returns metadata for a pure configuration state.
pub fn config() -> StateMeta {
    pure_with_tag(tags::CONFIG)
}

/// Returns metadata for a pure state with one classification tag.
pub fn pure_with_tag(tag: impl Into<String>) -> StateMeta {
    mk_meta(
        vec![Tag(tag.into())],
        SideEffectKind::Pure,
        Idempotency::None,
    )
}

/// Returns metadata for a read-only IO state with one classification tag.
pub fn read_only_io_with_tag(tag: impl Into<String>) -> StateMeta {
    mk_meta(
        vec![Tag(tag.into())],
        SideEffectKind::ReadOnlyIo,
        Idempotency::None,
    )
}

/// Returns metadata for a side-effecting state tagged as `apply_side_effect`.
pub fn apply_side_effect(idempotency_key: impl Into<String>) -> StateMeta {
    apply_side_effect_with_tag(tags::APPLY_SIDE_EFFECT, idempotency_key)
}

/// Returns metadata for a side-effecting execution state.
pub fn execute(idempotency_key: impl Into<String>) -> StateMeta {
    apply_side_effect_with_tag(tags::EXECUTE, idempotency_key)
}

/// Returns metadata for a side-effecting state with a custom tag and idempotency key.
pub fn apply_side_effect_with_tag(
    tag: impl Into<String>,
    idempotency_key: impl Into<String>,
) -> StateMeta {
    mk_meta(
        vec![Tag(tag.into())],
        SideEffectKind::ApplySideEffect,
        Idempotency::Key(idempotency_key.into()),
    )
}

#[cfg(test)]
mod tests {
    use mfm_machine::meta::{Idempotency, SideEffectKind};

    use super::*;

    #[test]
    fn fetch_data_meta_is_stable() {
        let m = fetch_data();
        assert_eq!(m.tags, vec![Tag(tags::FETCH_DATA.to_string())]);
        assert_eq!(m.side_effects, SideEffectKind::ReadOnlyIo);
        assert_eq!(m.idempotency, Idempotency::None);
    }

    #[test]
    fn validate_meta_is_stable() {
        let m = validate();
        assert_eq!(m.tags, vec![Tag(tags::VALIDATE.to_string())]);
        assert_eq!(m.side_effects, SideEffectKind::ReadOnlyIo);
        assert_eq!(m.idempotency, Idempotency::None);
    }

    #[test]
    fn pure_meta_is_stable() {
        let m = pure();
        assert!(m.tags.is_empty());
        assert_eq!(m.side_effects, SideEffectKind::Pure);
        assert_eq!(m.idempotency, Idempotency::None);
    }

    #[test]
    fn read_only_io_meta_is_stable() {
        let m = read_only_io();
        assert!(m.tags.is_empty());
        assert_eq!(m.side_effects, SideEffectKind::ReadOnlyIo);
        assert_eq!(m.idempotency, Idempotency::None);
    }

    #[test]
    fn apply_side_effect_meta_is_stable() {
        let m = apply_side_effect("idem-key");
        assert_eq!(m.tags, vec![Tag(tags::APPLY_SIDE_EFFECT.to_string())]);
        assert_eq!(m.side_effects, SideEffectKind::ApplySideEffect);
        assert_eq!(m.idempotency, Idempotency::Key("idem-key".to_string()));
    }

    #[test]
    fn execute_meta_is_stable() {
        let m = execute("idem-key");
        assert_eq!(m.tags, vec![Tag(tags::EXECUTE.to_string())]);
        assert_eq!(m.side_effects, SideEffectKind::ApplySideEffect);
        assert_eq!(m.idempotency, Idempotency::Key("idem-key".to_string()));
    }

    #[test]
    fn pure_with_tag_meta_is_stable() {
        let m = pure_with_tag(tags::CONFIG);
        assert_eq!(m.tags, vec![Tag(tags::CONFIG.to_string())]);
        assert_eq!(m.side_effects, SideEffectKind::Pure);
        assert_eq!(m.idempotency, Idempotency::None);
    }

    #[test]
    fn config_meta_is_stable() {
        let m = config();
        assert_eq!(m.tags, vec![Tag(tags::CONFIG.to_string())]);
        assert_eq!(m.side_effects, SideEffectKind::Pure);
        assert_eq!(m.idempotency, Idempotency::None);
    }

    #[test]
    fn read_only_io_with_tag_meta_is_stable() {
        let m = read_only_io_with_tag("validate");
        assert_eq!(m.tags, vec![Tag("validate".to_string())]);
        assert_eq!(m.side_effects, SideEffectKind::ReadOnlyIo);
        assert_eq!(m.idempotency, Idempotency::None);
    }

    #[test]
    fn apply_side_effect_with_tag_meta_is_stable() {
        let m = apply_side_effect_with_tag("custom_apply", "idem-key");
        assert_eq!(m.tags, vec![Tag("custom_apply".to_string())]);
        assert_eq!(m.side_effects, SideEffectKind::ApplySideEffect);
        assert_eq!(m.idempotency, Idempotency::Key("idem-key".to_string()));
    }
}
