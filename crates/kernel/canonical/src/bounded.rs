use crate::CanonicalError;
use serde::Serialize;
use std::io::{self, Write};

/// Serializes JSON while bounding encoded-output accumulation before each allocation.
///
/// The caller supplies its existing representation ceiling. This does not bound allocations
/// performed inside a custom serializer. A rejected write reports a lower bound, not the size of
/// a suffix that was never serialized.
pub fn to_json_bounded<T: Serialize + ?Sized>(value: &T, limit: usize) -> crate::Result<String> {
    let mut writer = Writer {
        bytes: Vec::new(),
        limit,
        rejected: None,
    };
    if let Err(source) = serde_json::to_writer(&mut writer, value) {
        return Err(match writer.rejected {
            Some(observed) => CanonicalError::serialization_limit(limit, observed, source),
            None => CanonicalError::json(source),
        });
    }
    String::from_utf8(writer.bytes).map_err(|source| CanonicalError::utf8(source.utf8_error()))
}
struct Writer {
    bytes: Vec<u8>,
    limit: usize,
    rejected: Option<usize>,
}
impl Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let required = self.bytes.len().saturating_add(bytes.len());
        if required > self.limit {
            self.rejected = Some(required);
            return Err(io::Error::other("JSON output exceeds its byte ceiling"));
        }
        if required > self.bytes.capacity() {
            self.bytes
                .try_reserve_exact(required - self.bytes.len())
                .map_err(io::Error::other)?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::ser::SerializeSeq;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Counted<'a>(&'a AtomicUsize);
    impl Serialize for Counted<'_> {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let mut sequence = serializer.serialize_seq(None)?;
            for _ in 0..100 {
                self.0.fetch_add(1, Ordering::SeqCst);
                sequence.serialize_element(&"item")?;
            }
            sequence.end()
        }
    }
    #[test]
    fn exact_bound_and_first_rejected_write_preserve_sources_without_traversing_suffix() {
        assert_eq!(to_json_bounded(&"abc", 5).unwrap(), "\"abc\"");
        let failure = to_json_bounded(&"abc", 4).unwrap_err();
        assert_eq!(failure.serialization_bound(), Some((4, 5)));
        assert!(std::error::Error::source(&failure)
            .unwrap()
            .source()
            .is_some());
        let calls = AtomicUsize::new(0);
        let failure = to_json_bounded(&Counted(&calls), 15).unwrap_err();
        assert!(failure.serialization_bound().is_some());
        assert!(calls.load(Ordering::SeqCst) < 100);
    }
    #[test]
    fn a_serializer_failure_retains_its_kind_and_supplied_message() {
        struct Reject;
        impl Serialize for Reject {
            fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
                Err(serde::ser::Error::custom(
                    "credential-shaped rejected input",
                ))
            }
        }
        let failure = to_json_bounded(&Reject, 1024).unwrap_err();
        assert!(failure.serialization_bound().is_none());
        let projection = serde_json::to_string(&failure).unwrap();
        assert!(projection.contains("credential-shaped rejected input"));
        assert!(!format!("{failure:?}").contains("credential-shaped"));
    }
}
