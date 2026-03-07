/// Registry primitives for Live IO transport factories.
use std::collections::HashMap;
use std::sync::Arc;

use crate::live_io::LiveIoTransportFactory;

/// Error returned when a transport registry operation cannot be completed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryError {
    /// Human-readable explanation of the registration failure.
    pub message: String,
}

impl RegistryError {
    /// Creates a new registry error with the provided message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for RegistryError {}

/// Registry abstraction used to store and resolve Live IO transport factories by namespace group.
pub trait TransportRegistry: Send + Sync {
    /// Registers a factory under its namespace group.
    ///
    /// Implementations should reject empty groups and duplicate registrations.
    fn register(&mut self, factory: Arc<dyn LiveIoTransportFactory>) -> Result<(), RegistryError>;

    /// Resolves the factory registered for an exact namespace group.
    fn resolve(&self, namespace_group: &str) -> Option<Arc<dyn LiveIoTransportFactory>>;

    /// Returns all registered factories in implementation-defined order.
    fn all(&self) -> Vec<Arc<dyn LiveIoTransportFactory>>;
}

/// Hash map-backed [`TransportRegistry`] implementation for runtime wiring and tests.
#[derive(Clone, Default)]
pub struct HashMapTransportRegistry {
    routes: HashMap<String, Arc<dyn LiveIoTransportFactory>>,
}

impl HashMapTransportRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }
}

impl TransportRegistry for HashMapTransportRegistry {
    fn register(&mut self, factory: Arc<dyn LiveIoTransportFactory>) -> Result<(), RegistryError> {
        let group = factory.namespace_group().trim();
        if group.is_empty() {
            return Err(RegistryError::new(
                "transport namespace group must not be empty",
            ));
        }
        if self.routes.contains_key(group) {
            return Err(RegistryError::new(format!(
                "duplicate transport namespace group: {group}",
            )));
        }
        self.routes.insert(group.to_string(), factory);
        Ok(())
    }

    fn resolve(&self, namespace_group: &str) -> Option<Arc<dyn LiveIoTransportFactory>> {
        self.routes.get(namespace_group).cloned()
    }

    fn all(&self) -> Vec<Arc<dyn LiveIoTransportFactory>> {
        self.routes.values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::live_io::{LiveIoEnv, LiveIoTransport};

    struct DummyFactory {
        group: &'static str,
    }

    impl LiveIoTransportFactory for DummyFactory {
        fn namespace_group(&self) -> &str {
            self.group
        }

        fn make(&self, _env: LiveIoEnv) -> Box<dyn LiveIoTransport> {
            panic!("not used")
        }
    }

    #[test]
    fn duplicate_namespace_group_is_rejected() {
        let mut reg = HashMapTransportRegistry::new();
        reg.register(Arc::new(DummyFactory { group: "local.fs" }))
            .expect("first registration");
        let err = reg
            .register(Arc::new(DummyFactory { group: "local.fs" }))
            .expect_err("expected duplicate error");
        assert!(err.message.contains("duplicate"));
    }
}
