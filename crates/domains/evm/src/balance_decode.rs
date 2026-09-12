use super::*;
use mfm_values::NativeCause;

#[derive(Debug, Serialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
enum BalanceMetadataError {
    #[error("correlation is empty")]
    EmptyCorrelation,
    #[error("correlation exceeds its byte ceiling")]
    CorrelationTooLong { limit: usize, observed_bytes: usize },
    #[error("correlation violates public-text rules; input withheld")]
    InvalidCorrelation { input: &'static str },
}

#[derive(Debug, Serialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
enum BalanceContextDecodeError {
    #[error("balance context metadata construction failed")]
    Metadata {
        location: &'static str,
        #[source]
        source: BalanceMetadataError,
    },
}

impl EvmBalanceResultMetadata {
    fn new(
        collection_ordinal: u32,
        correlation: String,
        route_ref: ContentRef,
    ) -> Result<Self, BalanceMetadataError> {
        if correlation.is_empty() {
            return Err(BalanceMetadataError::EmptyCorrelation);
        }
        if correlation.len() > 256 {
            return Err(BalanceMetadataError::CorrelationTooLong {
                limit: 256,
                observed_bytes: correlation.len(),
            });
        }
        if !valid_public_text(&correlation, 256) {
            return Err(BalanceMetadataError::InvalidCorrelation { input: "withheld" });
        }
        Ok(Self {
            collection_ordinal,
            correlation,
            route_ref,
        })
    }
}

pub(super) fn metadata(
    collection_ordinal: u32,
    correlation: String,
    route_ref: ContentRef,
) -> Result<EvmBalanceResultMetadata, NativeCause> {
    EvmBalanceResultMetadata::new(collection_ordinal, correlation, route_ref).map_err(|source| {
        NativeCause::from_error(BalanceContextDecodeError::Metadata {
            location: "metadata.correlation",
            source,
        })
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MetadataWire {
    collection_ordinal: u32,
    correlation: String,
    route_ref: ContentRef,
}
impl<'de> Deserialize<'de> for EvmBalanceResultMetadata {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = MetadataWire::deserialize(deserializer)?;
        Self::new(wire.collection_ordinal, wire.correlation, wire.route_ref)
            .map_err(de::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(
    deny_unknown_fields,
    bound(deserialize = "K: serde::de::DeserializeOwned")
)]
struct ContextWire<K> {
    request: EvmBalanceRequest,
    caller_continuation: K,
    metadata: MetadataWire,
    completed: Vec<EvmBalanceResult>,
    work: EvmBalanceWork,
}
impl<K: MfmValueTrait> ContextWire<K> {
    fn checked(self) -> Result<EvmBalanceContext<K>, NativeCause> {
        let value = EvmBalanceContext {
            request: self.request,
            caller_continuation: self.caller_continuation,
            metadata: metadata(
                self.metadata.collection_ordinal,
                self.metadata.correlation,
                self.metadata.route_ref,
            )?,
            completed: self.completed,
            work: self.work,
        };
        value.validate().map_err(NativeCause::from_error)?;
        Ok(value)
    }
}
impl<'de, K: MfmValueTrait> Deserialize<'de> for EvmBalanceContext<K> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ContextWire::<K>::deserialize(deserializer)?
            .checked()
            .map_err(de::Error::custom)
    }
}
impl<K: MfmValueTrait> EvmBalanceContext<K> {
    pub(super) fn decode_checked(bytes: &[u8]) -> Result<Self, NativeCause> {
        serde_json::from_slice::<ContextWire<K>>(bytes)
            .map_err(|source| NativeCause::from_error(mfm_canonical::JsonError::new(source)))?
            .checked()
    }
}
