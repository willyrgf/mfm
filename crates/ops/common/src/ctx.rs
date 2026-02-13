use mfm_machine::context::DynContext;
use mfm_machine::errors::StateError;
use mfm_machine::ids::ContextKey;

use crate::errors::state_unknown;

pub fn read_json(
    ctx: &dyn DynContext,
    key: &ContextKey,
) -> Result<Option<serde_json::Value>, StateError> {
    ctx.read(key)
        .map_err(|_| state_unknown("ctx_read_failed", "context read failed"))
}

pub fn read_u64_required(
    ctx: &dyn DynContext,
    key: &ContextKey,
    missing_code: &'static str,
    missing_message: &'static str,
) -> Result<u64, StateError> {
    read_json(ctx, key)?
        .and_then(|v| v.as_u64())
        .ok_or_else(|| state_unknown(missing_code, missing_message))
}

pub fn write_json(
    ctx: &mut dyn DynContext,
    key: ContextKey,
    value: serde_json::Value,
) -> Result<(), StateError> {
    ctx.write(key, value)
        .map_err(|_| state_unknown("ctx_write_failed", "context write failed"))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use mfm_machine::context::DynContext;
    use mfm_machine::errors::{ContextError, ErrorCategory, ErrorInfo};
    use mfm_machine::ids::{ContextKey, ErrorCode};

    use super::*;

    fn context_error() -> ContextError {
        ContextError::Other(ErrorInfo {
            code: ErrorCode("context_error".to_string()),
            category: ErrorCategory::Context,
            retryable: false,
            message: "ctx failure".to_string(),
            details: None,
        })
    }

    #[derive(Default)]
    struct MapContext {
        inner: HashMap<String, serde_json::Value>,
        fail_reads: bool,
        fail_writes: bool,
    }

    impl DynContext for MapContext {
        fn read(&self, key: &ContextKey) -> Result<Option<serde_json::Value>, ContextError> {
            if self.fail_reads {
                return Err(context_error());
            }
            Ok(self.inner.get(&key.0).cloned())
        }

        fn write(&mut self, key: ContextKey, value: serde_json::Value) -> Result<(), ContextError> {
            if self.fail_writes {
                return Err(context_error());
            }
            self.inner.insert(key.0, value);
            Ok(())
        }

        fn delete(&mut self, key: &ContextKey) -> Result<(), ContextError> {
            self.inner.remove(&key.0);
            Ok(())
        }

        fn dump(&self) -> Result<serde_json::Value, ContextError> {
            let mut out = serde_json::Map::new();
            for (k, v) in &self.inner {
                out.insert(k.clone(), v.clone());
            }
            Ok(serde_json::Value::Object(out))
        }
    }

    #[test]
    fn write_and_read_u64_roundtrip() {
        let key = ContextKey("n".to_string());
        let mut ctx = MapContext::default();
        write_json(&mut ctx, key.clone(), serde_json::json!(123)).expect("write");
        let n = read_u64_required(&ctx, &key, "missing_n", "missing n").expect("read");
        assert_eq!(n, 123);
    }

    #[test]
    fn read_u64_missing_uses_requested_error() {
        let key = ContextKey("missing".to_string());
        let ctx = MapContext::default();
        let err = read_u64_required(&ctx, &key, "missing_key", "key was missing")
            .expect_err("expected missing");
        assert_eq!(err.info.code.0, "missing_key");
        assert_eq!(err.info.message, "key was missing");
    }

    #[test]
    fn context_failures_map_to_stable_codes() {
        let key = ContextKey("k".to_string());
        let mut ctx = MapContext {
            fail_reads: true,
            ..Default::default()
        };
        let err = read_json(&ctx, &key).expect_err("read should fail");
        assert_eq!(err.info.code.0, "ctx_read_failed");

        ctx.fail_reads = false;
        ctx.fail_writes = true;
        let err = write_json(&mut ctx, key, serde_json::json!(1)).expect_err("write should fail");
        assert_eq!(err.info.code.0, "ctx_write_failed");
    }
}
