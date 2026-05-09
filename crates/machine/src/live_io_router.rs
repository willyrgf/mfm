//! Live IO transport router.
//!
//! This module is NOT part of the stable API contract (Appendix C.1) and may change.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::errors::{ErrorCategory, ErrorInfo, IoError};
use crate::ids::ErrorCode;
use crate::io::IoCall;
use crate::live_io::{LiveIoEnv, LiveIoTransport, LiveIoTransportFactory};
use crate::live_io_registry::{RegistryError, TransportRegistry};

const CODE_IO_UNKNOWN_NAMESPACE: &str = "io_unknown_namespace";

fn info(code: &'static str, category: ErrorCategory, message: &'static str) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable: false,
        message: message.to_string(),
        details: None,
    }
}

fn matches_group(namespace: &str, group: &str) -> bool {
    namespace == group || namespace.starts_with(&format!("{group}."))
}

fn longest_matching_group<'a>(
    namespace: &str,
    groups: impl Iterator<Item = &'a str>,
) -> Option<&'a str> {
    groups
        .filter(|group| matches_group(namespace, group))
        .max_by_key(|group| group.len())
}

/// A `LiveIoTransportFactory` that routes calls by namespace group.
///
/// Routing rule:
/// - candidates are registered groups `g` where `namespace == g` or `namespace` starts with `g + "."`
/// - selected route is the longest matching group
#[derive(Clone, Default)]
pub struct RouterLiveIoTransportFactory {
    routes: HashMap<String, Arc<dyn LiveIoTransportFactory>>,
}

impl RouterLiveIoTransportFactory {
    /// Creates a router from an already-validated namespace-group map.
    pub fn new(routes: HashMap<String, Arc<dyn LiveIoTransportFactory>>) -> Self {
        Self { routes }
    }

    /// Builds a router from factories, rejecting empty or duplicate namespace groups.
    pub fn from_factories(
        factories: Vec<Arc<dyn LiveIoTransportFactory>>,
    ) -> Result<Self, RegistryError> {
        let mut routes = HashMap::new();
        for factory in factories {
            let group = factory.namespace_group().trim();
            if group.is_empty() {
                return Err(RegistryError::new(
                    "transport namespace group must not be empty",
                ));
            }
            if routes.contains_key(group) {
                return Err(RegistryError::new(format!(
                    "duplicate transport namespace group: {group}",
                )));
            }
            routes.insert(group.to_string(), factory);
        }
        Ok(Self { routes })
    }

    /// Creates a router from all factories currently registered in `registry`.
    pub fn from_registry(registry: &dyn TransportRegistry) -> Self {
        let mut routes = HashMap::new();
        for factory in registry.all() {
            routes.insert(factory.namespace_group().to_string(), factory);
        }
        Self { routes }
    }

    /// Adds or replaces a route for the supplied namespace group.
    pub fn with_route(
        mut self,
        group: impl Into<String>,
        factory: Arc<dyn LiveIoTransportFactory>,
    ) -> Self {
        self.routes.insert(group.into(), factory);
        self
    }
}

impl LiveIoTransportFactory for RouterLiveIoTransportFactory {
    fn namespace_group(&self) -> &str {
        "router"
    }

    fn make(&self, env: LiveIoEnv) -> Box<dyn LiveIoTransport> {
        let mut routes = HashMap::new();
        for (group, factory) in &self.routes {
            routes.insert(group.clone(), factory.make(env.clone()));
        }
        Box::new(RouterLiveIoTransport { routes })
    }
}

struct RouterLiveIoTransport {
    routes: HashMap<String, Box<dyn LiveIoTransport>>,
}

#[async_trait]
impl LiveIoTransport for RouterLiveIoTransport {
    async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
        let Some(group) =
            longest_matching_group(&call.namespace, self.routes.keys().map(String::as_str))
                .map(str::to_string)
        else {
            // Do not echo request payloads in errors (avoid accidental secret leakage).
            return Err(IoError::Other(info(
                CODE_IO_UNKNOWN_NAMESPACE,
                ErrorCategory::Unknown,
                "unknown io namespace",
            )));
        };

        let t = self
            .routes
            .get_mut(&group)
            .expect("matched route must exist in route map");
        t.call(call).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::engine::Stores;
    use crate::errors::StorageError;
    use crate::ids::{ArtifactId, RunId, StateId};
    use crate::stores::{
        AppendBatchResult, ArtifactKind, ArtifactStore, StreamAppend, StreamId, StreamRecord,
        StreamStore,
    };
    use async_trait::async_trait;
    use std::sync::Arc;

    #[derive(Clone)]
    struct NoopStreamStore;

    #[async_trait]
    impl StreamStore for NoopStreamStore {
        async fn head_seq(&self, _stream_id: &StreamId) -> Result<u64, StorageError> {
            Ok(0)
        }

        async fn append(&self, _append: StreamAppend) -> Result<u64, StorageError> {
            Ok(0)
        }

        async fn append_batch(
            &self,
            _appends: Vec<StreamAppend>,
        ) -> Result<AppendBatchResult, StorageError> {
            Ok(AppendBatchResult {
                stream_heads: Vec::new(),
            })
        }

        async fn read_range(
            &self,
            _stream_id: &StreamId,
            _from_seq: u64,
            _to_seq: Option<u64>,
        ) -> Result<Vec<StreamRecord>, StorageError> {
            Ok(Vec::new())
        }
    }

    #[derive(Clone)]
    struct NoopArtifactStore;

    #[async_trait]
    impl ArtifactStore for NoopArtifactStore {
        async fn put(
            &self,
            _kind: ArtifactKind,
            _bytes: Vec<u8>,
        ) -> Result<ArtifactId, StorageError> {
            Ok(ArtifactId::must_new("0".repeat(64)))
        }

        async fn get(&self, _id: &ArtifactId) -> Result<Vec<u8>, StorageError> {
            Ok(Vec::new())
        }

        async fn exists(&self, _id: &ArtifactId) -> Result<bool, StorageError> {
            Ok(false)
        }
    }

    fn env() -> LiveIoEnv {
        LiveIoEnv {
            stores: Stores {
                streams: Arc::new(NoopStreamStore),
                artifacts: Arc::new(NoopArtifactStore),
            },
            run_id: RunId(uuid::Uuid::new_v4()),
            state_id: StateId::must_new("machine.main.s1".to_string()),
            attempt: 0,
        }
    }

    struct FixedFactory {
        group: &'static str,
        response: serde_json::Value,
    }

    impl LiveIoTransportFactory for FixedFactory {
        fn namespace_group(&self) -> &str {
            self.group
        }

        fn make(&self, _env: LiveIoEnv) -> Box<dyn LiveIoTransport> {
            Box::new(FixedTransport {
                response: self.response.clone(),
            })
        }
    }

    struct FixedTransport {
        response: serde_json::Value,
    }

    #[async_trait]
    impl LiveIoTransport for FixedTransport {
        async fn call(&mut self, _call: IoCall) -> Result<serde_json::Value, IoError> {
            Ok(self.response.clone())
        }
    }

    #[tokio::test]
    async fn routes_by_namespace_group_prefix() {
        let factory = RouterLiveIoTransportFactory::from_factories(vec![
            Arc::new(FixedFactory {
                group: "proof",
                response: serde_json::json!({"ok": "proof"}),
            }),
            Arc::new(FixedFactory {
                group: "evm",
                response: serde_json::json!({"ok": "evm"}),
            }),
        ])
        .expect("factory");
        let mut t = factory.make(env());

        let got = t
            .call(IoCall {
                namespace: "proof.read".to_string(),
                request: serde_json::json!({}),
                fact_key: None,
            })
            .await
            .expect("call");
        assert_eq!(got, serde_json::json!({"ok": "proof"}));

        let got = t
            .call(IoCall {
                namespace: "evm".to_string(),
                request: serde_json::json!({}),
                fact_key: None,
            })
            .await
            .expect("call");
        assert_eq!(got, serde_json::json!({"ok": "evm"}));
    }

    #[tokio::test]
    async fn routes_to_longest_matching_prefix() {
        let factory = RouterLiveIoTransportFactory::from_factories(vec![
            Arc::new(FixedFactory {
                group: "local",
                response: serde_json::json!({"ok": "local"}),
            }),
            Arc::new(FixedFactory {
                group: "local.fs",
                response: serde_json::json!({"ok": "local.fs"}),
            }),
        ])
        .expect("factory");

        let mut t = factory.make(env());
        let got = t
            .call(IoCall {
                namespace: "local.fs.read_text".to_string(),
                request: serde_json::json!({}),
                fact_key: None,
            })
            .await
            .expect("call");

        assert_eq!(got, serde_json::json!({"ok": "local.fs"}));
    }

    #[tokio::test]
    async fn unknown_namespace_is_stable_error() {
        let routes: HashMap<String, Arc<dyn LiveIoTransportFactory>> = HashMap::new();
        let factory = RouterLiveIoTransportFactory::new(routes);
        let mut t = factory.make(env());

        let err = t
            .call(IoCall {
                namespace: "unknown.ns".to_string(),
                request: serde_json::json!({"authorization": "Bearer x"}),
                fact_key: None,
            })
            .await
            .expect_err("expected error");

        match err {
            IoError::Other(info) => assert_eq!(info.code.0, CODE_IO_UNKNOWN_NAMESPACE),
            other => panic!("expected IoError::Other, got: {other:?}"),
        }
    }
}
