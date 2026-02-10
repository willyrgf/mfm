//! Internal helpers for event-profile filtering.
//!
//! Not part of the stable API contract (Appendix C.1).

#![allow(dead_code)]

use crate::config::EventProfile;
use crate::errors::RunError;
use crate::events::{DomainEvent, DOMAIN_EVENT_FACT_RECORDED};
use crate::recorder::EventRecorder;
use async_trait::async_trait;

pub(crate) fn should_persist_domain_event(profile: &EventProfile, event: &DomainEvent) -> bool {
    // Design contract: fact bindings must be durable regardless of profile.
    if event.name == DOMAIN_EVENT_FACT_RECORDED {
        return true;
    }

    match profile {
        EventProfile::Minimal => false,
        EventProfile::Normal | EventProfile::Verbose | EventProfile::Custom(_) => true,
    }
}

pub(crate) fn filter_domain_events(
    profile: &EventProfile,
    events: Vec<DomainEvent>,
) -> Vec<DomainEvent> {
    events
        .into_iter()
        .filter(|e| should_persist_domain_event(profile, e))
        .collect()
}

pub(crate) struct FilteringEventRecorder<'a> {
    profile: EventProfile,
    inner: &'a mut dyn EventRecorder,
}

impl<'a> FilteringEventRecorder<'a> {
    pub(crate) fn new(profile: EventProfile, inner: &'a mut dyn EventRecorder) -> Self {
        Self { profile, inner }
    }
}

#[async_trait]
impl EventRecorder for FilteringEventRecorder<'_> {
    async fn emit(&mut self, event: DomainEvent) -> Result<(), RunError> {
        if should_persist_domain_event(&self.profile, &event) {
            self.inner.emit(event).await
        } else {
            Ok(())
        }
    }

    async fn emit_many(&mut self, events: Vec<DomainEvent>) -> Result<(), RunError> {
        let filtered = filter_domain_events(&self.profile, events);
        if filtered.is_empty() {
            Ok(())
        } else {
            self.inner.emit_many(filtered).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::DOMAIN_EVENT_ARTIFACT_WRITTEN;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn ev(name: &str) -> DomainEvent {
        DomainEvent {
            name: name.to_string(),
            payload: serde_json::json!({"ok":true}),
            payload_ref: None,
        }
    }

    #[test]
    fn minimal_filters_non_critical_domain_events() {
        let events = vec![
            ev(DOMAIN_EVENT_FACT_RECORDED),
            ev(DOMAIN_EVENT_ARTIFACT_WRITTEN),
            ev("custom"),
        ];

        let got = filter_domain_events(&EventProfile::Minimal, events);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, DOMAIN_EVENT_FACT_RECORDED);
    }

    #[test]
    fn normal_allows_custom_domain_events() {
        let events = vec![ev("custom")];
        let got = filter_domain_events(&EventProfile::Normal, events);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "custom");
    }

    struct CollectingRecorder {
        events: Arc<Mutex<Vec<DomainEvent>>>,
    }

    impl CollectingRecorder {
        fn new(events: Arc<Mutex<Vec<DomainEvent>>>) -> Self {
            Self { events }
        }
    }

    #[async_trait]
    impl EventRecorder for CollectingRecorder {
        async fn emit(&mut self, event: DomainEvent) -> Result<(), RunError> {
            self.events.lock().await.push(event);
            Ok(())
        }

        async fn emit_many(&mut self, events: Vec<DomainEvent>) -> Result<(), RunError> {
            self.events.lock().await.extend(events);
            Ok(())
        }
    }

    #[tokio::test]
    async fn filtering_event_recorder_enforces_profile() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut inner = CollectingRecorder::new(Arc::clone(&events));

        let mut rec = FilteringEventRecorder::new(EventProfile::Minimal, &mut inner);
        rec.emit(ev("custom")).await.expect("emit");
        rec.emit(ev(DOMAIN_EVENT_FACT_RECORDED))
            .await
            .expect("emit");

        let got = events.lock().await.clone();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, DOMAIN_EVENT_FACT_RECORDED);
    }
}
