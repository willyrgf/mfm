use mfm_machine::meta::{DependencyStrategy, Idempotency, SideEffectKind, StateMeta, Tag};

pub fn fetch_data() -> StateMeta {
    StateMeta {
        tags: vec![Tag("fetch_data".to_string())],
        depends_on: Vec::new(),
        depends_on_strategy: DependencyStrategy::Latest,
        side_effects: SideEffectKind::ReadOnlyIo,
        idempotency: Idempotency::None,
    }
}

pub fn pure() -> StateMeta {
    StateMeta {
        tags: Vec::new(),
        depends_on: Vec::new(),
        depends_on_strategy: DependencyStrategy::Latest,
        side_effects: SideEffectKind::Pure,
        idempotency: Idempotency::None,
    }
}

pub fn apply_side_effect(idempotency_key: impl Into<String>) -> StateMeta {
    StateMeta {
        tags: vec![Tag("apply_side_effect".to_string())],
        depends_on: Vec::new(),
        depends_on_strategy: DependencyStrategy::Latest,
        side_effects: SideEffectKind::ApplySideEffect,
        idempotency: Idempotency::Key(idempotency_key.into()),
    }
}

#[cfg(test)]
mod tests {
    use mfm_machine::meta::{Idempotency, SideEffectKind};

    use super::*;

    #[test]
    fn fetch_data_meta_is_stable() {
        let m = fetch_data();
        assert_eq!(m.tags, vec![Tag("fetch_data".to_string())]);
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
    fn apply_side_effect_meta_is_stable() {
        let m = apply_side_effect("idem-key");
        assert_eq!(m.tags, vec![Tag("apply_side_effect".to_string())]);
        assert_eq!(m.side_effects, SideEffectKind::ApplySideEffect);
        assert_eq!(m.idempotency, Idempotency::Key("idem-key".to_string()));
    }
}
