#![warn(missing_docs)]
//! Secret-free EVM State values and capability contracts.
//!
//! The domain owns complete cumulative contexts for Portfolio's observational EVM reads. Provider
//! clients and response ingress belong to the live adapter crate; no domain value contains a
//! client, credential, raw response, or generic context map.

use std::collections::BTreeSet;
use std::marker::PhantomData;

use mfm_capabilities::ReadCapabilityContract;
use mfm_ids::{ContentRef, StableId};
use mfm_program::{
    CapabilityInjection, Operation, OperationExpansion, ProgramError, ProposedStateOutcome,
    PureState, ReadPreparationError, ReadState, State,
};
use mfm_program_derive::MfmValue;
use mfm_values::{string_contains_secret_marker, MfmValue as MfmValueTrait};
use serde::de;
use serde::{Deserialize, Serialize};

macro_rules! impl_checked_deserialize {
    ($type:ident { $($field:ident: $field_type:ty),+ $(,)? }) => {
        impl<'de> Deserialize<'de> for $type {
            fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Wire {
                    $($field: $field_type,)+
                }

                let wire = Wire::deserialize(deserializer)?;
                let value = Self {
                    $($field: wire.$field,)+
                };
                value.validate().map(|_| value).map_err(de::Error::custom)
            }
        }
    };
}

/// Secret-free public identity of one live EVM route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "physical-target",
    version = "1",
    schema = "mfm.evm-physical-target"
)]
pub struct EvmPhysicalTarget {
    chain_id: u64,
    endpoint_ref: ContentRef,
}

impl_checked_deserialize!(EvmPhysicalTarget {
    chain_id: u64,
    endpoint_ref: ContentRef,
});

impl EvmPhysicalTarget {
    /// Constructs one public route identity.
    pub fn new(chain_id: u64, endpoint_ref: ContentRef) -> Result<Self, EvmDomainError> {
        if chain_id == 0 {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(Self {
            chain_id,
            endpoint_ref,
        })
    }

    /// Returns the public EVM chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the public endpoint identity.
    pub const fn endpoint_ref(&self) -> &ContentRef {
        &self.endpoint_ref
    }

    /// Derives the exact canonical adapter binding identity.
    pub fn binding_ref(&self) -> Result<ContentRef, EvmDomainError> {
        mfm_values::canonicalize_mfm_value(self)
            .map(|(_, reference)| reference)
            .map_err(|_| EvmDomainError::Program)
    }

    fn validate(&self) -> Result<(), EvmDomainError> {
        Self::new(self.chain_id, self.endpoint_ref.clone()).map(|_| ())
    }
}

/// Secret-free named identity of one physical EVM endpoint.
///
/// The name alone identifies the route: an RPC URL, credential, or client handle is never
/// endpoint material, so replacing one under the same name leaves the derived reference,
/// the `EvmPhysicalTarget`, and the planned Program byte-identical.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "endpoint",
    version = "1",
    schema = "mfm.evm-endpoint"
)]
pub struct EvmEndpoint {
    endpoint_id: String,
}

impl_checked_deserialize!(EvmEndpoint {
    endpoint_id: String,
});

impl EvmEndpoint {
    /// Constructs one checked public endpoint identity.
    pub fn new(endpoint_id: impl Into<String>) -> Result<Self, EvmDomainError> {
        let endpoint = Self {
            endpoint_id: endpoint_id.into(),
        };
        endpoint.validate().map(|_| endpoint)
    }

    /// Derives the exact canonical endpoint reference one `EvmPhysicalTarget` binds.
    pub fn endpoint_ref(&self) -> Result<ContentRef, EvmDomainError> {
        mfm_values::canonicalize_mfm_value(self)
            .map(|(_, reference)| reference)
            .map_err(|_| EvmDomainError::Program)
    }

    fn validate(&self) -> Result<(), EvmDomainError> {
        valid_public_text(&self.endpoint_id, 256)
            .then_some(())
            .ok_or(EvmDomainError::InvalidValue)
    }
}

/// Maximum admitted EVM balance sources.
pub const EVM_BALANCE_SOURCE_LIMIT: usize = 64;

/// One EVM balance source admitted into a cumulative collection context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EvmBalanceSource {
    /// Public source identity.
    pub source_id: String,
    /// Public chain id.
    pub chain_id: u64,
    /// Public account address.
    pub address: String,
    /// Optional token contract address.
    pub token: Option<String>,
}

impl_checked_deserialize!(EvmBalanceSource {
    source_id: String,
    chain_id: u64,
    address: String,
    token: Option<String>,
});

impl EvmBalanceSource {
    /// Validates one public source identity without changing ownership.
    fn validate(&self) -> Result<(), EvmDomainError> {
        if !valid_public_text(&self.source_id, 256)
            || !valid_public_text(&self.address, 128)
            || self.address != self.address.to_ascii_lowercase()
            || self.chain_id == 0
            || self.token.as_ref().is_some_and(|token| {
                !valid_public_text(token, 128) || token != &token.to_ascii_lowercase()
            })
        {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(())
    }
}

/// Singular domain-planned EVM balance fragment admission value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EvmBalanceRequest {
    /// Ordered sources; the sequential Program expands one State per source.
    pub sources: Vec<EvmBalanceSource>,
    /// Integer decimal scale for public amounts.
    pub decimals: u8,
}

impl_checked_deserialize!(EvmBalanceRequest {
    sources: Vec<EvmBalanceSource>,
    decimals: u8,
});

impl EvmBalanceRequest {
    /// Creates one bounded declaration-ordered source list.
    pub fn new(sources: Vec<EvmBalanceSource>, decimals: u8) -> Result<Self, EvmDomainError> {
        if sources.is_empty()
            || sources.len() > EVM_BALANCE_SOURCE_LIMIT
            || decimals > 30
            || sources.iter().any(|source| source.validate().is_err())
            || duplicate_source_ids(&sources)
            || sources.first().is_some_and(|first| {
                sources
                    .iter()
                    .any(|source| source.chain_id != first.chain_id)
            })
        {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(Self { sources, decimals })
    }

    /// Validates a decoded balance request and every admitted source.
    pub fn validate(&self) -> Result<(), EvmDomainError> {
        if self.sources.is_empty()
            || self.sources.len() > EVM_BALANCE_SOURCE_LIMIT
            || self.decimals > 30
            || self.sources.iter().any(|source| source.validate().is_err())
            || duplicate_source_ids(&self.sources)
            || self.sources.first().is_some_and(|first| {
                self.sources
                    .iter()
                    .any(|source| source.chain_id != first.chain_id)
            })
        {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(())
    }

    /// Scales one exact observed raw amount to this request's declared decimal scale.
    pub fn scale_units(&self, raw_units: &str, source_decimals: u8) -> Option<String> {
        scale_units(raw_units, source_decimals, self.decimals)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct EvmBalanceResultMetadata {
    collection_ordinal: u32,
    correlation: String,
    route_ref: ContentRef,
}

impl_checked_deserialize!(EvmBalanceResultMetadata {
    collection_ordinal: u32,
    correlation: String,
    route_ref: ContentRef,
});

impl EvmBalanceResultMetadata {
    fn new(
        collection_ordinal: u32,
        correlation: String,
        route_ref: ContentRef,
    ) -> Result<Self, EvmDomainError> {
        if !valid_public_text(&correlation, 256) {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(Self {
            collection_ordinal,
            correlation,
            route_ref,
        })
    }

    fn validate(&self) -> Result<(), EvmDomainError> {
        Self::new(
            self.collection_ordinal,
            self.correlation.clone(),
            self.route_ref.clone(),
        )
        .map(|_| ())
    }
}

/// Committed public block anchor of one bounded EVM observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EvmBlockAnchor {
    number: String,
    hash: String,
}

impl_checked_deserialize!(EvmBlockAnchor {
    number: String,
    hash: String,
});

impl EvmBlockAnchor {
    fn new(number: String, hash: String) -> Result<Self, EvmDomainError> {
        if !is_decimal_integer(&number) || !valid_public_text(&hash, 256) {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(Self { number, hash })
    }

    /// Returns the canonical decimal block number.
    pub fn number(&self) -> &str {
        &self.number
    }

    /// Returns the exact public block hash.
    pub fn hash(&self) -> &str {
        &self.hash
    }

    fn validate(&self) -> Result<(), EvmDomainError> {
        Self::new(self.number.clone(), self.hash.clone()).map(|_| ())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum EvmBalanceWork {
    CheckChainIdentity {
        source: EvmBalanceSource,
    },
    ReadInitialAnchor {
        source: EvmBalanceSource,
        checked_chain_id: u64,
    },
    SelectAsset {
        source: EvmBalanceSource,
        checked_chain_id: u64,
        initial_anchor: EvmBlockAnchor,
    },
    ReadNativeBalance {
        source: EvmBalanceSource,
        checked_chain_id: u64,
        initial_anchor: EvmBlockAnchor,
    },
    ReadTokenDecimals {
        source: EvmBalanceSource,
        checked_chain_id: u64,
        initial_anchor: EvmBlockAnchor,
    },
    ReadTokenBalance {
        source: EvmBalanceSource,
        checked_chain_id: u64,
        initial_anchor: EvmBlockAnchor,
        token_decimals: u8,
    },
    ConfirmAnchor {
        source: EvmBalanceSource,
        checked_chain_id: u64,
        initial_anchor: EvmBlockAnchor,
        source_decimals: u8,
        raw_balance: String,
    },
    Complete,
}

/// Complete cumulative balance context consumed by each source State and consolidation State.
///
/// `K` is the caller-owned continuation.  The EVM domain stores and returns it without
/// interpreting its fields; the consuming handoff is what keeps Portfolio semantics outside the
/// reusable EVM fragment.
#[derive(Debug, Serialize, MfmValue)]
#[serde(
    deny_unknown_fields,
    bound(
        serialize = "K: Serialize",
        deserialize = "K: serde::de::DeserializeOwned"
    )
)]
pub struct EvmBalanceContext<K: MfmValueTrait> {
    request: EvmBalanceRequest,
    caller_continuation: K,
    metadata: EvmBalanceResultMetadata,
    completed: Vec<EvmBalanceResult>,
    work: EvmBalanceWork,
}

impl<'de, K: MfmValueTrait> Deserialize<'de> for EvmBalanceContext<K> {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(
            deny_unknown_fields,
            bound(deserialize = "K: serde::de::DeserializeOwned")
        )]
        struct Wire<K> {
            request: EvmBalanceRequest,
            caller_continuation: K,
            metadata: EvmBalanceResultMetadata,
            completed: Vec<EvmBalanceResult>,
            work: EvmBalanceWork,
        }

        let wire = Wire::deserialize(deserializer)?;
        let context = Self {
            request: wire.request,
            caller_continuation: wire.caller_continuation,
            metadata: wire.metadata,
            completed: wire.completed,
            work: wire.work,
        };
        context
            .validate()
            .map(|_| context)
            .map_err(de::Error::custom)
    }
}

impl<K: MfmValueTrait> EvmBalanceContext<K> {
    /// Creates the first work item for one caller-owned collection continuation.
    pub fn new(
        request: EvmBalanceRequest,
        caller_continuation: K,
        collection_ordinal: u32,
        correlation: String,
        route_ref: ContentRef,
    ) -> Result<Self, EvmDomainError> {
        let first = request
            .sources
            .first()
            .cloned()
            .ok_or(EvmDomainError::InvalidValue)?;
        let metadata = EvmBalanceResultMetadata::new(collection_ordinal, correlation, route_ref)?;
        let context = Self {
            request,
            caller_continuation,
            metadata,
            completed: Vec::new(),
            work: EvmBalanceWork::CheckChainIdentity { source: first },
        };
        context.validate()?;
        Ok(context)
    }

    fn active_source(&self) -> Option<&EvmBalanceSource> {
        match &self.work {
            EvmBalanceWork::CheckChainIdentity { source }
            | EvmBalanceWork::ReadInitialAnchor { source, .. }
            | EvmBalanceWork::SelectAsset { source, .. }
            | EvmBalanceWork::ReadNativeBalance { source, .. }
            | EvmBalanceWork::ReadTokenDecimals { source, .. }
            | EvmBalanceWork::ReadTokenBalance { source, .. }
            | EvmBalanceWork::ConfirmAnchor { source, .. } => Some(source),
            EvmBalanceWork::Complete => None,
        }
    }

    fn validate(&self) -> Result<(), EvmDomainError> {
        self.request.validate()?;
        self.metadata.validate()?;
        if self.completed.len() > self.request.sources.len()
            || self
                .completed
                .iter()
                .zip(&self.request.sources)
                .any(|(result, source)| {
                    result.validate().is_err()
                        || result.source != *source
                        || (result.source.token.is_none()
                            && result.decimals != self.request.decimals)
                        || self
                            .request
                            .scale_units(&result.raw_units, result.decimals)
                            .is_none()
                })
            || self
                .completed
                .windows(2)
                .any(|pair| pair[0].anchor != pair[1].anchor)
        {
            return Err(EvmDomainError::InvalidValue);
        }
        if let Some(anchor) = work_initial_anchor(&self.work) {
            if anchor.validate().is_err()
                || self
                    .completed
                    .first()
                    .is_some_and(|result| result.anchor != *anchor)
            {
                return Err(EvmDomainError::InvalidValue);
            }
        }
        let expected = self.request.sources.get(self.completed.len());
        match (&self.work, expected) {
            (EvmBalanceWork::Complete, None) => {}
            (EvmBalanceWork::Complete, Some(_)) => return Err(EvmDomainError::InvalidValue),
            (work, Some(expected_source)) => {
                let source = self.active_source().ok_or(EvmDomainError::InvalidValue)?;
                if source != expected_source || source.validate().is_err() {
                    return Err(EvmDomainError::InvalidValue);
                }
                match work {
                    EvmBalanceWork::ReadNativeBalance { source, .. } if source.token.is_some() => {
                        return Err(EvmDomainError::InvalidValue)
                    }
                    EvmBalanceWork::ReadTokenDecimals { source, .. }
                    | EvmBalanceWork::ReadTokenBalance { source, .. }
                        if source.token.is_none() =>
                    {
                        return Err(EvmDomainError::InvalidValue)
                    }
                    EvmBalanceWork::ReadTokenBalance { token_decimals, .. }
                        if *token_decimals > 30 =>
                    {
                        return Err(EvmDomainError::InvalidValue)
                    }
                    EvmBalanceWork::ConfirmAnchor {
                        source_decimals,
                        raw_balance,
                        initial_anchor,
                        checked_chain_id,
                        ..
                    } if *source_decimals > 30
                        || (source.token.is_none()
                            && *source_decimals != self.request.decimals)
                        || !is_decimal_integer(raw_balance)
                        || initial_anchor.validate().is_err()
                        || *checked_chain_id != source.chain_id =>
                    {
                        return Err(EvmDomainError::InvalidValue)
                    }
                    EvmBalanceWork::ReadInitialAnchor {
                        checked_chain_id, ..
                    }
                    | EvmBalanceWork::SelectAsset {
                        checked_chain_id, ..
                    }
                    | EvmBalanceWork::ReadNativeBalance {
                        checked_chain_id, ..
                    }
                    | EvmBalanceWork::ReadTokenDecimals {
                        checked_chain_id, ..
                    }
                    | EvmBalanceWork::ReadTokenBalance {
                        checked_chain_id, ..
                    } if *checked_chain_id != source.chain_id => {
                        return Err(EvmDomainError::InvalidValue)
                    }
                    _ => {}
                }
            }
            (_, None) => return Err(EvmDomainError::InvalidValue),
        }
        Ok(())
    }
}

/// Closed Match selector retaining the entire EVM context through either asset arm.
#[derive(Debug, Serialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields,
    bound(
        serialize = "K: Serialize",
        deserialize = "K: serde::de::DeserializeOwned"
    )
)]
pub enum EvmBalanceAsset<K: MfmValueTrait> {
    /// Native arm with the complete cumulative context.
    Native(EvmBalanceContext<K>),
    /// Token arm with the complete cumulative context.
    Token(EvmBalanceContext<K>),
}

impl<'de, K: MfmValueTrait> Deserialize<'de> for EvmBalanceAsset<K> {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(
            tag = "kind",
            content = "value",
            rename_all = "snake_case",
            deny_unknown_fields,
            bound(deserialize = "K: serde::de::DeserializeOwned")
        )]
        enum Wire<K: MfmValueTrait> {
            Native(EvmBalanceContext<K>),
            Token(EvmBalanceContext<K>),
        }

        let value = match Wire::deserialize(deserializer)? {
            Wire::Native(context) => Self::Native(context),
            Wire::Token(context) => Self::Token(context),
        };
        value.validate().map(|_| value).map_err(de::Error::custom)
    }
}

impl<K: MfmValueTrait> EvmBalanceAsset<K> {
    fn validate(&self) -> Result<(), EvmDomainError> {
        let context = match self {
            Self::Native(context) | Self::Token(context) => context,
        };
        context.validate()?;
        match (self, context.active_source()) {
            (Self::Native(_), Some(source)) if source.token.is_none() => Ok(()),
            (Self::Token(_), Some(source)) if source.token.is_some() => Ok(()),
            _ => Err(EvmDomainError::InvalidValue),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct EvmBalanceResult {
    source: EvmBalanceSource,
    decimals: u8,
    raw_units: String,
    anchor: EvmBlockAnchor,
}

impl_checked_deserialize!(EvmBalanceResult {
    source: EvmBalanceSource,
    decimals: u8,
    raw_units: String,
    anchor: EvmBlockAnchor,
});

impl EvmBalanceResult {
    fn validate(&self) -> Result<(), EvmDomainError> {
        if self.source.validate().is_err()
            || self.decimals > 30
            || !is_decimal_integer(&self.raw_units)
            || self.anchor.validate().is_err()
        {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum EvmCollectedAsset {
    Native,
    Token { contract: String },
}

impl EvmCollectedAsset {
    fn validate(&self) -> Result<(), EvmDomainError> {
        match self {
            Self::Native => Ok(()),
            Self::Token { contract }
                if valid_public_text(contract, 128)
                    && contract == &contract.to_ascii_lowercase() =>
            {
                Ok(())
            }
            Self::Token { .. } => Err(EvmDomainError::InvalidValue),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct EvmCollectedBalanceSource {
    source_id: String,
    chain_id: u64,
    address: String,
    asset: EvmCollectedAsset,
}

impl_checked_deserialize!(EvmCollectedBalanceSource {
    source_id: String,
    chain_id: u64,
    address: String,
    asset: EvmCollectedAsset,
});

impl EvmCollectedBalanceSource {
    fn from_source(source: &EvmBalanceSource) -> Self {
        Self {
            source_id: source.source_id.clone(),
            chain_id: source.chain_id,
            address: source.address.clone(),
            asset: match &source.token {
                Some(contract) => EvmCollectedAsset::Token {
                    contract: contract.clone(),
                },
                None => EvmCollectedAsset::Native,
            },
        }
    }

    fn validate(&self) -> Result<(), EvmDomainError> {
        if !valid_public_text(&self.source_id, 256)
            || self.chain_id == 0
            || !valid_public_text(&self.address, 128)
            || self.address != self.address.to_ascii_lowercase()
        {
            return Err(EvmDomainError::InvalidValue);
        }
        self.asset.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct EvmCollectedBalance {
    source: EvmCollectedBalanceSource,
    decimals: u8,
    raw_units: String,
}

impl_checked_deserialize!(EvmCollectedBalance {
    source: EvmCollectedBalanceSource,
    decimals: u8,
    raw_units: String,
});

impl EvmCollectedBalance {
    fn from_result(result: EvmBalanceResult) -> Self {
        Self {
            source: EvmCollectedBalanceSource::from_source(&result.source),
            decimals: result.decimals,
            raw_units: result.raw_units,
        }
    }

    fn validate(&self) -> Result<(), EvmDomainError> {
        if self.source.validate().is_err()
            || self.decimals > 30
            || !is_decimal_integer(&self.raw_units)
        {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(())
    }
}

type EvmCollectedBalanceParts = (String, String, Option<String>, u8, String);
type EvmBalanceCollectionParts = (u64, EvmBlockAnchor, Vec<EvmCollectedBalanceParts>, String);
type EvmBalanceCollectionCompletionParts<K> = (
    K,
    u32,
    u64,
    String,
    String,
    Vec<EvmCollectedBalanceParts>,
    String,
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct EvmBalanceCollectionResult {
    chain_id: u64,
    anchor: EvmBlockAnchor,
    balances: Vec<EvmCollectedBalance>,
    total_scaled: String,
}

impl_checked_deserialize!(EvmBalanceCollectionResult {
    chain_id: u64,
    anchor: EvmBlockAnchor,
    balances: Vec<EvmCollectedBalance>,
    total_scaled: String,
});

impl EvmBalanceCollectionResult {
    fn validate(&self) -> Result<(), EvmDomainError> {
        if self.chain_id == 0
            || self.anchor.validate().is_err()
            || self.balances.is_empty()
            || self.balances.len() > EVM_BALANCE_SOURCE_LIMIT
            || self
                .balances
                .iter()
                .any(|balance| balance.validate().is_err())
            || duplicate_collected_source_ids(&self.balances)
            || self
                .balances
                .iter()
                .any(|balance| balance.source.chain_id != self.chain_id)
            || !is_decimal_integer(&self.total_scaled)
        {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(())
    }

    fn into_parts(self) -> EvmBalanceCollectionParts {
        let balances = self
            .balances
            .into_iter()
            .map(|balance| {
                let token = match balance.source.asset {
                    EvmCollectedAsset::Native => None,
                    EvmCollectedAsset::Token { contract } => Some(contract),
                };
                (
                    balance.source.source_id,
                    balance.source.address,
                    token,
                    balance.decimals,
                    balance.raw_units,
                )
            })
            .collect();
        (self.chain_id, self.anchor, balances, self.total_scaled)
    }
}

/// One consuming frozen EVM collection handoff returned to the caller context.
#[derive(Debug, Serialize, MfmValue)]
#[serde(
    deny_unknown_fields,
    bound(
        serialize = "K: Serialize",
        deserialize = "K: serde::de::DeserializeOwned"
    )
)]
pub struct EvmBalanceCollectionCompletion<K: MfmValueTrait> {
    caller_context: K,
    collection_ordinal: u32,
    collection: EvmBalanceCollectionResult,
}

impl<'de, K: MfmValueTrait> Deserialize<'de> for EvmBalanceCollectionCompletion<K> {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(
            deny_unknown_fields,
            bound(deserialize = "K: serde::de::DeserializeOwned")
        )]
        struct Wire<K> {
            caller_context: K,
            collection_ordinal: u32,
            collection: EvmBalanceCollectionResult,
        }
        let wire = Wire::deserialize(deserializer)?;
        let completion = Self {
            caller_context: wire.caller_context,
            collection_ordinal: wire.collection_ordinal,
            collection: wire.collection,
        };
        completion
            .validate()
            .map(|_| completion)
            .map_err(de::Error::custom)
    }
}

impl<K: MfmValueTrait> EvmBalanceCollectionCompletion<K> {
    fn new(
        caller_context: K,
        collection_ordinal: u32,
        collection: EvmBalanceCollectionResult,
    ) -> Result<Self, EvmDomainError> {
        collection.validate()?;
        Ok(Self {
            caller_context,
            collection_ordinal,
            collection,
        })
    }

    fn validate(&self) -> Result<(), EvmDomainError> {
        self.collection.validate()
    }

    /// Consumes the EVM completion into its unchanged caller context and checked observations.
    pub fn into_parts(self) -> EvmBalanceCollectionCompletionParts<K> {
        let (chain_id, anchor, balances, total_scaled) = self.collection.into_parts();
        (
            self.caller_context,
            self.collection_ordinal,
            chain_id,
            anchor.number,
            anchor.hash,
            balances,
            total_scaled,
        )
    }
}

/// Redaction-safe EVM domain construction failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EvmDomainError {
    /// A bounded public value is invalid.
    #[error("EVM domain value is invalid")]
    InvalidValue,
    /// A capability intent/evidence pair is not bound.
    #[error("EVM capability evidence is not bound")]
    EvidenceBinding,
    /// Program authoring or binding identity is invalid.
    #[error("EVM Program contract is invalid")]
    Program,
}

/// Closed typed subject of one bounded EVM read.
///
/// The adapter serializes the whole intent as every provider's request bytes, so this
/// sum is the provider's own request contract. A provider decodes it with the checked
/// [`EvmReadIntent`] deserializer and matches these variants; it never mirrors the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum EvmReadSubject {
    /// The public chain identity of the target route.
    ChainIdentity,
    /// The current chain head that anchors one collection.
    InitialAnchor,
    /// The native balance of one source at its committed anchor.
    NativeBalance {
        /// Public balance source.
        source: EvmBalanceSource,
        /// Committed observation anchor.
        anchor: EvmBlockAnchor,
    },
    /// The decimal scale of one source's token contract at its committed anchor.
    TokenDecimals {
        /// Public balance source.
        source: EvmBalanceSource,
        /// Committed observation anchor.
        anchor: EvmBlockAnchor,
    },
    /// The token balance of one source at its committed anchor.
    TokenBalance {
        /// Public balance source.
        source: EvmBalanceSource,
        /// Committed observation anchor.
        anchor: EvmBlockAnchor,
    },
    /// The committed anchor re-read by number for confirmation.
    ConfirmAnchor {
        /// Public balance source.
        source: EvmBalanceSource,
        /// Committed observation anchor.
        anchor: EvmBlockAnchor,
    },
}

/// One strict read intent shared by the bounded EVM read capabilities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub struct EvmReadIntent {
    operation: String,
    chain_id: u64,
    subject: EvmReadSubject,
    route_ref: ContentRef,
}

impl_checked_deserialize!(EvmReadIntent {
    operation: String,
    chain_id: u64,
    subject: EvmReadSubject,
    route_ref: ContentRef,
});

impl EvmReadIntent {
    fn new(
        operation: String,
        chain_id: u64,
        subject: EvmReadSubject,
        route_ref: ContentRef,
    ) -> Result<Self, EvmDomainError> {
        let intent = Self {
            operation,
            chain_id,
            subject,
            route_ref,
        };
        intent.validate()?;
        Ok(intent)
    }

    /// Returns the exact operation and public chain target fixed by this intent.
    pub fn operation_and_chain_id(&self) -> (&str, u64) {
        (&self.operation, self.chain_id)
    }

    /// Returns the exact planned physical route identity.
    pub const fn route_ref(&self) -> &ContentRef {
        &self.route_ref
    }

    /// Returns the exact typed subject this intent fixes.
    pub const fn subject(&self) -> &EvmReadSubject {
        &self.subject
    }

    fn validate(&self) -> Result<(), EvmDomainError> {
        if StableId::new(&self.operation).is_err() || self.chain_id == 0 {
            return Err(EvmDomainError::InvalidValue);
        }
        match &self.subject {
            EvmReadSubject::ChainIdentity | EvmReadSubject::InitialAnchor => {}
            EvmReadSubject::NativeBalance { source, anchor }
            | EvmReadSubject::TokenDecimals { source, anchor }
            | EvmReadSubject::TokenBalance { source, anchor }
            | EvmReadSubject::ConfirmAnchor { source, anchor } => {
                source.validate()?;
                anchor.validate()?;
                if source.chain_id != self.chain_id {
                    return Err(EvmDomainError::InvalidValue);
                }
            }
        }
        Ok(())
    }
}

/// Typed provider values admitted after raw EVM ingress is discarded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum EvmReadValue {
    /// Authenticated chain id.
    ChainId(u64),
    /// Authenticated block anchor components.
    Anchor {
        /// Canonical decimal block number.
        number: String,
        /// Canonical public block hash.
        hash: String,
    },
    /// Canonical unsigned raw units.
    RawUnits(String),
    /// Token decimal scale.
    TokenDecimals(u8),
}

impl<'de> Deserialize<'de> for EvmReadValue {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(
            tag = "kind",
            content = "value",
            rename_all = "snake_case",
            deny_unknown_fields
        )]
        enum Wire {
            ChainId(u64),
            Anchor { number: String, hash: String },
            RawUnits(String),
            TokenDecimals(u8),
        }

        let value = match Wire::deserialize(deserializer)? {
            Wire::ChainId(chain_id) => Self::ChainId(chain_id),
            Wire::Anchor { number, hash } => Self::Anchor { number, hash },
            Wire::RawUnits(units) => Self::RawUnits(units),
            Wire::TokenDecimals(decimals) => Self::TokenDecimals(decimals),
        };
        read_value_valid(&value)
            .then_some(value)
            .ok_or_else(|| de::Error::custom(EvmDomainError::InvalidValue))
    }
}

/// Closed EVM read evidence sum; raw provider material is discarded before construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum EvmReadEvidence {
    /// Provider returned an exact structured value for the committed intent.
    Returned {
        /// Interpreted provider value.
        value: EvmReadValue,
    },
    /// Provider returned a reviewed rejection.
    Rejected,
    /// Adapter accepted a safe failure.
    SafeFailure,
    /// Authentication/integrity failed and grants no retry authority.
    IntegrityBlocked,
}

impl EvmReadEvidence {
    /// Validates the exact operation-bound evidence envelope.
    fn validate_for(&self, intent: &EvmReadIntent) -> Result<(), EvmDomainError> {
        let valid = match self {
            Self::Returned { value } => read_value_valid_for_intent(intent, value),
            Self::Rejected | Self::SafeFailure | Self::IntegrityBlocked => true,
        };
        valid.then_some(()).ok_or(EvmDomainError::EvidenceBinding)
    }
}

#[derive(Debug, Clone, Copy)]
enum ReadCapabilityFamily {
    ChainIdentity,
    LatestAnchor,
    Balance,
}

fn validate_read_capability_intent(
    intent: &EvmReadIntent,
    family: ReadCapabilityFamily,
) -> Result<(), EvmDomainError> {
    intent.validate()?;
    let valid = match family {
        ReadCapabilityFamily::ChainIdentity => matches!(
            (&intent.operation[..], &intent.subject),
            (
                "mfm.evm.read-chain-identity@1",
                EvmReadSubject::ChainIdentity
            )
        ),
        ReadCapabilityFamily::LatestAnchor => matches!(
            (&intent.operation[..], &intent.subject),
            (
                "mfm.evm.read-initial-anchor@1",
                EvmReadSubject::InitialAnchor
            ) | (
                "mfm.evm.confirm-balance-anchor@1",
                EvmReadSubject::ConfirmAnchor { .. }
            )
        ),
        ReadCapabilityFamily::Balance => matches!(
            (&intent.operation[..], &intent.subject),
            (
                "mfm.evm.read-native-balance@1",
                EvmReadSubject::NativeBalance { .. }
            ) | (
                "mfm.evm.read-token-decimals@1",
                EvmReadSubject::TokenDecimals { .. }
            ) | (
                "mfm.evm.read-token-balance@1",
                EvmReadSubject::TokenBalance { .. }
            )
        ),
    };
    valid.then_some(()).ok_or(EvmDomainError::EvidenceBinding)
}

/// Observational EVM Read capability for the target chain identity.
pub struct EvmChainIdentityRead;

/// Observational EVM Read capability for chain anchors.
pub struct EvmAnchorRead;

/// Observational EVM Read capability for native and token balances.
pub struct EvmBalanceRead;

macro_rules! impl_read_capability {
    ($ty:ty, $name:literal, $family:ident) => {
        impl ReadCapabilityContract for $ty {
            type Intent = EvmReadIntent;
            type Evidence = EvmReadEvidence;
            fn contract_id() -> mfm_capabilities::Result<StableId> {
                StableId::new($name).map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
            }

            fn bind_evidence(
                intent: &Self::Intent,
                evidence: &Self::Evidence,
            ) -> mfm_capabilities::Result<()> {
                validate_read_capability_intent(intent, ReadCapabilityFamily::$family)
                    .and_then(|_| evidence.validate_for(intent))
                    .map_err(|_| mfm_capabilities::CapabilityError::EvidenceBinding)
            }
        }
    };
}

impl_read_capability!(
    EvmChainIdentityRead,
    "mfm.evm.capability.read-chain-identity@1",
    ChainIdentity
);
impl_read_capability!(
    EvmAnchorRead,
    "mfm.evm.capability.read-anchor@1",
    LatestAnchor
);
impl_read_capability!(EvmBalanceRead, "mfm.evm.capability.read-balance@1", Balance);

/// Verifies that one balance collection targets the expected EVM chain.
pub struct CheckChainIdentity<K: MfmValueTrait>(PhantomData<fn() -> K>);

/// Reads the initial anchor for one balance source.
pub struct ReadInitialAnchor<K: MfmValueTrait>(PhantomData<fn() -> K>);

/// Selects native or token balance observation topology.
pub struct SelectBalanceAsset<K: MfmValueTrait>(PhantomData<fn() -> K>);

/// Reads one native-asset balance.
pub struct ReadNativeBalance<K: MfmValueTrait>(PhantomData<fn() -> K>);

/// Reads the decimal scale for one token asset.
pub struct ReadTokenDecimals<K: MfmValueTrait>(PhantomData<fn() -> K>);

/// Reads one token-asset balance.
pub struct ReadTokenBalance<K: MfmValueTrait>(PhantomData<fn() -> K>);

/// Confirms that the initial balance anchor remains current.
pub struct ConfirmBalanceAnchor<K: MfmValueTrait>(PhantomData<fn() -> K>);

/// Consolidates one completed EVM balance collection.
pub struct ConsolidateBalanceCollection<K: MfmValueTrait>(PhantomData<fn() -> K>);

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EvmBalanceFailureStage {
    CheckChainIdentity,
    ReadInitialAnchor,
    SelectAsset,
    ReadNativeBalance,
    ReadTokenDecimals,
    ReadTokenBalance,
    ConfirmAnchor,
    Consolidate,
}

impl EvmBalanceFailureStage {
    const fn as_str(self) -> &'static str {
        match self {
            Self::CheckChainIdentity => "check_chain_identity",
            Self::ReadInitialAnchor => "read_initial_anchor",
            Self::SelectAsset => "select_asset",
            Self::ReadNativeBalance => "read_native_balance",
            Self::ReadTokenDecimals => "read_token_decimals",
            Self::ReadTokenBalance => "read_token_balance",
            Self::ConfirmAnchor => "confirm_anchor",
            Self::Consolidate => "consolidate",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EvmBalanceFailureCode {
    ChainIdentityUnavailable,
    ObservationUnavailable,
    CollectionInvalid,
    IntegrityBlocked,
}

impl EvmBalanceFailureCode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ChainIdentityUnavailable => "chain_identity_unavailable",
            Self::ObservationUnavailable => "observation_unavailable",
            Self::CollectionInvalid => "collection_invalid",
            Self::IntegrityBlocked => "integrity_blocked",
        }
    }

    const fn is_source_failure_for(self, stage: EvmBalanceFailureStage) -> bool {
        matches!(
            (self, stage),
            (
                Self::ChainIdentityUnavailable,
                EvmBalanceFailureStage::CheckChainIdentity
            ) | (
                Self::ObservationUnavailable,
                EvmBalanceFailureStage::ReadInitialAnchor
                    | EvmBalanceFailureStage::SelectAsset
                    | EvmBalanceFailureStage::ReadNativeBalance
                    | EvmBalanceFailureStage::ReadTokenDecimals
                    | EvmBalanceFailureStage::ReadTokenBalance
                    | EvmBalanceFailureStage::ConfirmAnchor
                    | EvmBalanceFailureStage::Consolidate,
            ) | (Self::CollectionInvalid, EvmBalanceFailureStage::Consolidate)
        )
    }
}

/// EVM-owned typed failure that leaves Portfolio semantics to its mapper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum EvmBalanceFailure {
    /// A bounded source-stage failure.
    SourceUnavailable {
        /// Stage that could not be completed.
        stage: String,
        /// Planner-owned collection ordinal retained for the caller's failure mapper.
        collection_ordinal: u32,
        /// Stable redacted failure code.
        code: String,
    },
    /// The adapter accepted a capability integrity block.
    IntegrityBlocked {
        /// Stage whose capability accepted the integrity block.
        stage: String,
        /// Planner-owned collection ordinal retained for the caller's failure mapper.
        collection_ordinal: u32,
        /// Stable redacted integrity code.
        code: String,
    },
}

impl<'de> Deserialize<'de> for EvmBalanceFailure {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(
            tag = "kind",
            content = "value",
            rename_all = "snake_case",
            deny_unknown_fields
        )]
        enum Wire {
            SourceUnavailable {
                stage: EvmBalanceFailureStage,
                collection_ordinal: u32,
                code: EvmBalanceFailureCode,
            },
            IntegrityBlocked {
                stage: EvmBalanceFailureStage,
                collection_ordinal: u32,
                code: EvmBalanceFailureCode,
            },
        }

        let value = match Wire::deserialize(deserializer)? {
            Wire::SourceUnavailable {
                stage,
                collection_ordinal,
                code,
            } if code.is_source_failure_for(stage) => Self::SourceUnavailable {
                stage: stage.as_str().to_owned(),
                collection_ordinal,
                code: code.as_str().to_owned(),
            },
            Wire::IntegrityBlocked {
                stage,
                collection_ordinal,
                code: EvmBalanceFailureCode::IntegrityBlocked,
            } => Self::IntegrityBlocked {
                stage: stage.as_str().to_owned(),
                collection_ordinal,
                code: EvmBalanceFailureCode::IntegrityBlocked.as_str().to_owned(),
            },
            _ => return Err(de::Error::custom(EvmDomainError::InvalidValue)),
        };
        Ok(value)
    }
}

fn balance_integrity_failure<K: MfmValueTrait>(
    context: &EvmBalanceContext<K>,
    stage: EvmBalanceFailureStage,
) -> EvmBalanceFailure {
    EvmBalanceFailure::IntegrityBlocked {
        stage: stage.as_str().to_owned(),
        collection_ordinal: context.metadata.collection_ordinal,
        code: EvmBalanceFailureCode::IntegrityBlocked.as_str().to_owned(),
    }
}

macro_rules! impl_balance_state {
    ($state:ident, $input:ty, $output:ty, $id:literal) => {
        impl<K: MfmValueTrait> State for $state<K> {
            type Input = $input;
            type Output = $output;
            type Failure = EvmBalanceFailure;

            fn state_id() -> mfm_program::Result<StableId> {
                StableId::new($id).map_err(|_| mfm_program::ProgramError::InvalidContract)
            }
        }
    };
}

impl_balance_state!(
    CheckChainIdentity,
    EvmBalanceContext<K>,
    EvmBalanceContext<K>,
    "mfm.evm.state.check-chain-identity@1"
);
impl_balance_state!(
    ReadInitialAnchor,
    EvmBalanceContext<K>,
    EvmBalanceContext<K>,
    "mfm.evm.state.read-initial-anchor@1"
);
impl_balance_state!(
    SelectBalanceAsset,
    EvmBalanceContext<K>,
    EvmBalanceAsset<K>,
    "mfm.evm.state.select-asset@1"
);
impl_balance_state!(
    ReadNativeBalance,
    EvmBalanceContext<K>,
    EvmBalanceContext<K>,
    "mfm.evm.state.read-native-balance@1"
);
impl_balance_state!(
    ReadTokenDecimals,
    EvmBalanceContext<K>,
    EvmBalanceContext<K>,
    "mfm.evm.state.read-token-decimals@1"
);
impl_balance_state!(
    ReadTokenBalance,
    EvmBalanceContext<K>,
    EvmBalanceContext<K>,
    "mfm.evm.state.read-token-balance@1"
);
impl_balance_state!(
    ConfirmBalanceAnchor,
    EvmBalanceContext<K>,
    EvmBalanceContext<K>,
    "mfm.evm.state.confirm-balance-anchor@1"
);
impl_balance_state!(
    ConsolidateBalanceCollection,
    EvmBalanceContext<K>,
    EvmBalanceCollectionCompletion<K>,
    "mfm.evm.state.consolidate-balance-collection@1"
);

fn balance_read_intent<K: MfmValueTrait>(
    input: &EvmBalanceContext<K>,
    operation: &str,
    chain_id: u64,
    subject: EvmReadSubject,
) -> Result<EvmReadIntent, EvmDomainError> {
    EvmReadIntent::new(
        operation.to_owned(),
        chain_id,
        subject,
        input.metadata.route_ref.clone(),
    )
}

fn prepare_check_chain_identity<K: MfmValueTrait>(
    input: &EvmBalanceContext<K>,
) -> Result<EvmReadIntent, EvmDomainError> {
    let EvmBalanceWork::CheckChainIdentity { source } = &input.work else {
        return Err(EvmDomainError::InvalidValue);
    };
    balance_read_intent(
        input,
        "mfm.evm.read-chain-identity@1",
        source.chain_id,
        EvmReadSubject::ChainIdentity,
    )
}

fn interpret_check_chain_identity<K: MfmValueTrait>(
    input: EvmBalanceContext<K>,
    evidence: &EvmReadEvidence,
) -> ProposedStateOutcome<EvmBalanceContext<K>, EvmBalanceFailure> {
    if matches!(evidence, EvmReadEvidence::IntegrityBlocked) {
        return failure(balance_integrity_failure(
            &input,
            EvmBalanceFailureStage::CheckChainIdentity,
        ));
    }
    let intent = match prepare_check_chain_identity(&input) {
        Ok(intent) => intent,
        Err(_) => return balance_failure(&input, EvmBalanceFailureStage::CheckChainIdentity),
    };
    let EvmBalanceWork::CheckChainIdentity { source } = &input.work else {
        return balance_failure(&input, EvmBalanceFailureStage::CheckChainIdentity);
    };
    match read_returned(evidence, &intent) {
        Some(EvmReadValue::ChainId(chain_id)) if *chain_id == source.chain_id => {
            let work = EvmBalanceWork::ReadInitialAnchor {
                source: source.clone(),
                checked_chain_id: *chain_id,
            };
            advance_balance_context(input, work, EvmBalanceFailureStage::CheckChainIdentity)
        }
        _ => balance_failure(&input, EvmBalanceFailureStage::CheckChainIdentity),
    }
}

fn prepare_read_initial_anchor<K: MfmValueTrait>(
    input: &EvmBalanceContext<K>,
) -> Result<EvmReadIntent, EvmDomainError> {
    let EvmBalanceWork::ReadInitialAnchor {
        source,
        checked_chain_id,
    } = &input.work
    else {
        return Err(EvmDomainError::InvalidValue);
    };
    balance_read_intent(
        input,
        "mfm.evm.read-initial-anchor@1",
        *checked_chain_id,
        EvmReadSubject::InitialAnchor,
    )
    .and_then(|intent| {
        (source.chain_id == *checked_chain_id)
            .then_some(intent)
            .ok_or(EvmDomainError::InvalidValue)
    })
}

fn interpret_read_initial_anchor<K: MfmValueTrait>(
    input: EvmBalanceContext<K>,
    evidence: &EvmReadEvidence,
) -> ProposedStateOutcome<EvmBalanceContext<K>, EvmBalanceFailure> {
    if matches!(evidence, EvmReadEvidence::IntegrityBlocked) {
        return failure(balance_integrity_failure(
            &input,
            EvmBalanceFailureStage::ReadInitialAnchor,
        ));
    }
    let intent = match prepare_read_initial_anchor(&input) {
        Ok(intent) => intent,
        Err(_) => return balance_failure(&input, EvmBalanceFailureStage::ReadInitialAnchor),
    };
    let EvmBalanceWork::ReadInitialAnchor {
        source,
        checked_chain_id,
    } = &input.work
    else {
        return balance_failure(&input, EvmBalanceFailureStage::ReadInitialAnchor);
    };
    match read_returned(evidence, &intent) {
        Some(EvmReadValue::Anchor { number, hash }) => match read_anchor(number, hash) {
            Some(initial_anchor) => {
                let work = EvmBalanceWork::SelectAsset {
                    source: source.clone(),
                    checked_chain_id: *checked_chain_id,
                    initial_anchor,
                };
                advance_balance_context(input, work, EvmBalanceFailureStage::ReadInitialAnchor)
            }
            None => balance_failure(&input, EvmBalanceFailureStage::ReadInitialAnchor),
        },
        _ => balance_failure(&input, EvmBalanceFailureStage::ReadInitialAnchor),
    }
}

fn select_balance_asset<K: MfmValueTrait>(
    input: EvmBalanceContext<K>,
) -> ProposedStateOutcome<EvmBalanceAsset<K>, EvmBalanceFailure> {
    let native = matches!(input.work, EvmBalanceWork::SelectAsset { ref source, .. } if source.token.is_none());
    let token = matches!(input.work, EvmBalanceWork::SelectAsset { ref source, .. } if source.token.is_some());
    if native {
        success(EvmBalanceAsset::Native(input))
    } else if token {
        success(EvmBalanceAsset::Token(input))
    } else {
        balance_failure(&input, EvmBalanceFailureStage::SelectAsset)
    }
}

fn prepare_read_native_balance<K: MfmValueTrait>(
    input: &EvmBalanceContext<K>,
) -> Result<EvmReadIntent, EvmDomainError> {
    let (EvmBalanceWork::SelectAsset {
        source,
        checked_chain_id,
        initial_anchor,
    }
    | EvmBalanceWork::ReadNativeBalance {
        source,
        checked_chain_id,
        initial_anchor,
    }) = &input.work
    else {
        return Err(EvmDomainError::InvalidValue);
    };
    if source.token.is_some() {
        return Err(EvmDomainError::InvalidValue);
    }
    balance_read_intent(
        input,
        "mfm.evm.read-native-balance@1",
        *checked_chain_id,
        EvmReadSubject::NativeBalance {
            source: source.clone(),
            anchor: initial_anchor.clone(),
        },
    )
}

fn interpret_read_native_balance<K: MfmValueTrait>(
    input: EvmBalanceContext<K>,
    evidence: &EvmReadEvidence,
) -> ProposedStateOutcome<EvmBalanceContext<K>, EvmBalanceFailure> {
    if matches!(evidence, EvmReadEvidence::IntegrityBlocked) {
        return failure(balance_integrity_failure(
            &input,
            EvmBalanceFailureStage::ReadNativeBalance,
        ));
    }
    let intent = match prepare_read_native_balance(&input) {
        Ok(intent) => intent,
        Err(_) => return balance_failure(&input, EvmBalanceFailureStage::ReadNativeBalance),
    };
    let (EvmBalanceWork::SelectAsset {
        source,
        checked_chain_id,
        initial_anchor,
    }
    | EvmBalanceWork::ReadNativeBalance {
        source,
        checked_chain_id,
        initial_anchor,
    }) = &input.work
    else {
        return balance_failure(&input, EvmBalanceFailureStage::ReadNativeBalance);
    };
    match read_returned(evidence, &intent) {
        Some(EvmReadValue::RawUnits(raw_balance)) => {
            let work = EvmBalanceWork::ConfirmAnchor {
                source: source.clone(),
                checked_chain_id: *checked_chain_id,
                initial_anchor: initial_anchor.clone(),
                source_decimals: input.request.decimals,
                raw_balance: raw_balance.clone(),
            };
            advance_balance_context(input, work, EvmBalanceFailureStage::ReadNativeBalance)
        }
        _ => balance_failure(&input, EvmBalanceFailureStage::ReadNativeBalance),
    }
}

fn prepare_read_token_decimals<K: MfmValueTrait>(
    input: &EvmBalanceContext<K>,
) -> Result<EvmReadIntent, EvmDomainError> {
    let (EvmBalanceWork::SelectAsset {
        source,
        checked_chain_id,
        initial_anchor,
    }
    | EvmBalanceWork::ReadTokenDecimals {
        source,
        checked_chain_id,
        initial_anchor,
    }) = &input.work
    else {
        return Err(EvmDomainError::InvalidValue);
    };
    if source.token.is_none() {
        return Err(EvmDomainError::InvalidValue);
    }
    balance_read_intent(
        input,
        "mfm.evm.read-token-decimals@1",
        *checked_chain_id,
        EvmReadSubject::TokenDecimals {
            source: source.clone(),
            anchor: initial_anchor.clone(),
        },
    )
}

fn interpret_read_token_decimals<K: MfmValueTrait>(
    input: EvmBalanceContext<K>,
    evidence: &EvmReadEvidence,
) -> ProposedStateOutcome<EvmBalanceContext<K>, EvmBalanceFailure> {
    if matches!(evidence, EvmReadEvidence::IntegrityBlocked) {
        return failure(balance_integrity_failure(
            &input,
            EvmBalanceFailureStage::ReadTokenDecimals,
        ));
    }
    let intent = match prepare_read_token_decimals(&input) {
        Ok(intent) => intent,
        Err(_) => return balance_failure(&input, EvmBalanceFailureStage::ReadTokenDecimals),
    };
    let (EvmBalanceWork::SelectAsset {
        source,
        checked_chain_id,
        initial_anchor,
    }
    | EvmBalanceWork::ReadTokenDecimals {
        source,
        checked_chain_id,
        initial_anchor,
    }) = &input.work
    else {
        return balance_failure(&input, EvmBalanceFailureStage::ReadTokenDecimals);
    };
    match read_returned(evidence, &intent) {
        Some(EvmReadValue::TokenDecimals(token_decimals)) if *token_decimals <= 30 => {
            let work = EvmBalanceWork::ReadTokenBalance {
                source: source.clone(),
                checked_chain_id: *checked_chain_id,
                initial_anchor: initial_anchor.clone(),
                token_decimals: *token_decimals,
            };
            advance_balance_context(input, work, EvmBalanceFailureStage::ReadTokenDecimals)
        }
        _ => balance_failure(&input, EvmBalanceFailureStage::ReadTokenDecimals),
    }
}

fn prepare_read_token_balance<K: MfmValueTrait>(
    input: &EvmBalanceContext<K>,
) -> Result<EvmReadIntent, EvmDomainError> {
    let EvmBalanceWork::ReadTokenBalance {
        source,
        checked_chain_id,
        initial_anchor,
        ..
    } = &input.work
    else {
        return Err(EvmDomainError::InvalidValue);
    };
    balance_read_intent(
        input,
        "mfm.evm.read-token-balance@1",
        *checked_chain_id,
        EvmReadSubject::TokenBalance {
            source: source.clone(),
            anchor: initial_anchor.clone(),
        },
    )
}

fn interpret_read_token_balance<K: MfmValueTrait>(
    input: EvmBalanceContext<K>,
    evidence: &EvmReadEvidence,
) -> ProposedStateOutcome<EvmBalanceContext<K>, EvmBalanceFailure> {
    if matches!(evidence, EvmReadEvidence::IntegrityBlocked) {
        return failure(balance_integrity_failure(
            &input,
            EvmBalanceFailureStage::ReadTokenBalance,
        ));
    }
    let intent = match prepare_read_token_balance(&input) {
        Ok(intent) => intent,
        Err(_) => return balance_failure(&input, EvmBalanceFailureStage::ReadTokenBalance),
    };
    let EvmBalanceWork::ReadTokenBalance {
        source,
        checked_chain_id,
        initial_anchor,
        token_decimals,
    } = &input.work
    else {
        return balance_failure(&input, EvmBalanceFailureStage::ReadTokenBalance);
    };
    match read_returned(evidence, &intent) {
        Some(EvmReadValue::RawUnits(raw_balance)) => {
            let work = EvmBalanceWork::ConfirmAnchor {
                source: source.clone(),
                checked_chain_id: *checked_chain_id,
                initial_anchor: initial_anchor.clone(),
                source_decimals: *token_decimals,
                raw_balance: raw_balance.clone(),
            };
            advance_balance_context(input, work, EvmBalanceFailureStage::ReadTokenBalance)
        }
        _ => balance_failure(&input, EvmBalanceFailureStage::ReadTokenBalance),
    }
}

fn prepare_confirm_balance_anchor<K: MfmValueTrait>(
    input: &EvmBalanceContext<K>,
) -> Result<EvmReadIntent, EvmDomainError> {
    let EvmBalanceWork::ConfirmAnchor {
        source,
        checked_chain_id,
        initial_anchor,
        ..
    } = &input.work
    else {
        return Err(EvmDomainError::InvalidValue);
    };
    balance_read_intent(
        input,
        "mfm.evm.confirm-balance-anchor@1",
        *checked_chain_id,
        EvmReadSubject::ConfirmAnchor {
            source: source.clone(),
            anchor: initial_anchor.clone(),
        },
    )
}

fn interpret_confirm_balance_anchor<K: MfmValueTrait>(
    mut input: EvmBalanceContext<K>,
    evidence: &EvmReadEvidence,
) -> ProposedStateOutcome<EvmBalanceContext<K>, EvmBalanceFailure> {
    if matches!(evidence, EvmReadEvidence::IntegrityBlocked) {
        return failure(balance_integrity_failure(
            &input,
            EvmBalanceFailureStage::ConfirmAnchor,
        ));
    }
    let intent = match prepare_confirm_balance_anchor(&input) {
        Ok(intent) => intent,
        Err(_) => return balance_failure(&input, EvmBalanceFailureStage::ConfirmAnchor),
    };
    let EvmBalanceWork::ConfirmAnchor {
        source,
        initial_anchor,
        source_decimals,
        raw_balance,
        ..
    } = &input.work
    else {
        return balance_failure(&input, EvmBalanceFailureStage::ConfirmAnchor);
    };
    let Some(EvmReadValue::Anchor { number, hash }) = read_returned(evidence, &intent) else {
        return balance_failure(&input, EvmBalanceFailureStage::ConfirmAnchor);
    };
    let Some(anchor) = read_anchor(number, hash) else {
        return balance_failure(&input, EvmBalanceFailureStage::ConfirmAnchor);
    };
    if &anchor != initial_anchor {
        return balance_failure(&input, EvmBalanceFailureStage::ConfirmAnchor);
    }
    if input
        .request
        .scale_units(raw_balance, *source_decimals)
        .is_none()
    {
        return balance_failure(&input, EvmBalanceFailureStage::ConfirmAnchor);
    }
    input.completed.push(EvmBalanceResult {
        source: source.clone(),
        decimals: *source_decimals,
        raw_units: raw_balance.clone(),
        anchor: initial_anchor.clone(),
    });
    let work = match input.request.sources.get(input.completed.len()).cloned() {
        Some(source) => EvmBalanceWork::CheckChainIdentity { source },
        None => EvmBalanceWork::Complete,
    };
    advance_balance_context(input, work, EvmBalanceFailureStage::ConfirmAnchor)
}

fn consolidate_balance_collection<K: MfmValueTrait>(
    input: EvmBalanceContext<K>,
) -> ProposedStateOutcome<EvmBalanceCollectionCompletion<K>, EvmBalanceFailure> {
    if !matches!(input.work, EvmBalanceWork::Complete) || input.validate().is_err() {
        return balance_failure(&input, EvmBalanceFailureStage::Consolidate);
    }
    let total_scaled = match completed_total_scaled(&input.request, &input.completed) {
        Some(total) => total,
        None => return balance_failure(&input, EvmBalanceFailureStage::Consolidate),
    };
    let collection_ordinal = input.metadata.collection_ordinal;
    let chain_id = input.request.sources[0].chain_id;
    let anchor = input.completed[0].anchor.clone();
    let result = EvmBalanceCollectionResult {
        chain_id,
        anchor,
        balances: input
            .completed
            .into_iter()
            .map(EvmCollectedBalance::from_result)
            .collect(),
        total_scaled,
    };
    match EvmBalanceCollectionCompletion::new(input.caller_continuation, collection_ordinal, result)
    {
        Ok(output) => success(output),
        Err(_) => failure(EvmBalanceFailure::SourceUnavailable {
            stage: EvmBalanceFailureStage::Consolidate.as_str().to_owned(),
            collection_ordinal,
            code: EvmBalanceFailureCode::CollectionInvalid.as_str().to_owned(),
        }),
    }
}

macro_rules! impl_balance_access {
    ($state:ident, $capability:ty, $prepare:path, $interpret:path) => {
        impl<K: MfmValueTrait> ReadState<$capability> for $state<K> {
            fn prepare(
                input: &Self::Input,
            ) -> std::result::Result<
                <$capability as ReadCapabilityContract>::Intent,
                ReadPreparationError,
            > {
                $prepare(input).map_err(|_| ReadPreparationError)
            }

            fn interpret(
                input: Self::Input,
                evidence: &<$capability as ReadCapabilityContract>::Evidence,
            ) -> ProposedStateOutcome<Self::Output, Self::Failure> {
                $interpret(input, evidence)
            }
        }
    };
}

impl_balance_access!(
    CheckChainIdentity,
    EvmChainIdentityRead,
    prepare_check_chain_identity,
    interpret_check_chain_identity
);
impl_balance_access!(
    ReadInitialAnchor,
    EvmAnchorRead,
    prepare_read_initial_anchor,
    interpret_read_initial_anchor
);
impl_balance_access!(
    ReadNativeBalance,
    EvmBalanceRead,
    prepare_read_native_balance,
    interpret_read_native_balance
);
impl_balance_access!(
    ReadTokenDecimals,
    EvmBalanceRead,
    prepare_read_token_decimals,
    interpret_read_token_decimals
);
impl_balance_access!(
    ReadTokenBalance,
    EvmBalanceRead,
    prepare_read_token_balance,
    interpret_read_token_balance
);
impl_balance_access!(
    ConfirmBalanceAnchor,
    EvmAnchorRead,
    prepare_confirm_balance_anchor,
    interpret_confirm_balance_anchor
);
impl<K: MfmValueTrait> PureState for SelectBalanceAsset<K> {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        select_balance_asset(input)
    }
}
impl<K: MfmValueTrait> PureState for ConsolidateBalanceCollection<K> {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        consolidate_balance_collection(input)
    }
}

fn balance_failure<K: MfmValueTrait, O>(
    context: &EvmBalanceContext<K>,
    stage: EvmBalanceFailureStage,
) -> ProposedStateOutcome<O, EvmBalanceFailure> {
    let code = match stage {
        EvmBalanceFailureStage::CheckChainIdentity => {
            EvmBalanceFailureCode::ChainIdentityUnavailable
        }
        EvmBalanceFailureStage::ReadInitialAnchor
        | EvmBalanceFailureStage::SelectAsset
        | EvmBalanceFailureStage::ReadNativeBalance
        | EvmBalanceFailureStage::ReadTokenDecimals
        | EvmBalanceFailureStage::ReadTokenBalance
        | EvmBalanceFailureStage::ConfirmAnchor
        | EvmBalanceFailureStage::Consolidate => EvmBalanceFailureCode::ObservationUnavailable,
    };
    failure(EvmBalanceFailure::SourceUnavailable {
        stage: stage.as_str().to_owned(),
        collection_ordinal: context.metadata.collection_ordinal,
        code: code.as_str().to_owned(),
    })
}

fn advance_balance_context<K: MfmValueTrait>(
    mut context: EvmBalanceContext<K>,
    work: EvmBalanceWork,
    stage: EvmBalanceFailureStage,
) -> ProposedStateOutcome<EvmBalanceContext<K>, EvmBalanceFailure> {
    context.work = work;
    if context.validate().is_ok() {
        success(context)
    } else {
        balance_failure(&context, stage)
    }
}

/// Deterministically expands one checked EVM balance collection.
pub struct CollectEvmBalances<K: MfmValueTrait> {
    binding_ref: ContentRef,
    source_count: usize,
    marker: PhantomData<fn() -> K>,
}

impl<K: MfmValueTrait> CollectEvmBalances<K> {
    /// Constructs one collection expansion with a checked source count.
    pub fn new(binding_ref: ContentRef, source_count: usize) -> Result<Self, EvmDomainError> {
        if !(1..=EVM_BALANCE_SOURCE_LIMIT).contains(&source_count) {
            return Err(EvmDomainError::Program);
        }
        Ok(Self {
            binding_ref,
            source_count,
            marker: PhantomData,
        })
    }
}

impl<K: MfmValueTrait> Operation for CollectEvmBalances<K> {
    type Input = EvmBalanceContext<K>;
    type Output = EvmBalanceCollectionCompletion<K>;
    type Failure = EvmBalanceFailure;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        let native = StableId::new("native").map_err(|_| ProgramError::InvalidContract)?;
        let token = StableId::new("token").map_err(|_| ProgramError::InvalidContract)?;
        for _ in 0..self.source_count {
            body.read::<CheckChainIdentity<K>, EvmChainIdentityRead>(&self.binding_ref)?;
            body.read::<ReadInitialAnchor<K>, EvmAnchorRead>(&self.binding_ref)?;
            body.pure::<SelectBalanceAsset<K>>()?;
            body.match_join::<EvmBalanceAsset<K>, EvmBalanceContext<K>>(|arms| {
                arms.arm::<EvmBalanceContext<K>>(native.clone(), |branch| {
                    branch.read::<ReadNativeBalance<K>, EvmBalanceRead>(&self.binding_ref)
                })?;
                arms.arm::<EvmBalanceContext<K>>(token.clone(), |branch| {
                    branch.read::<ReadTokenDecimals<K>, EvmBalanceRead>(&self.binding_ref)?;
                    branch.read::<ReadTokenBalance<K>, EvmBalanceRead>(&self.binding_ref)
                })
            })?;
            body.read::<ConfirmBalanceAnchor<K>, EvmAnchorRead>(&self.binding_ref)?;
        }
        body.pure::<ConsolidateBalanceCollection<K>>()
    }
}

macro_rules! impl_identity_injection {
    ($capability:ty, $state:ident) => {
        impl<K: MfmValueTrait> CapabilityInjection<$state<K>> for $capability {
            type Setup = ContentRef;
            type ExpandedInput = EvmBalanceContext<K>;
            type ExpandedOutput = EvmBalanceContext<K>;

            fn original_binding_ref(setup: &Self::Setup) -> mfm_program::Result<ContentRef> {
                Ok(setup.clone())
            }
        }
    };
}

impl_identity_injection!(EvmChainIdentityRead, CheckChainIdentity);
impl_identity_injection!(EvmAnchorRead, ReadInitialAnchor);
impl_identity_injection!(EvmBalanceRead, ReadNativeBalance);
impl_identity_injection!(EvmBalanceRead, ReadTokenDecimals);
impl_identity_injection!(EvmBalanceRead, ReadTokenBalance);
impl_identity_injection!(EvmAnchorRead, ConfirmBalanceAnchor);

fn success<O, F>(output: O) -> ProposedStateOutcome<O, F> {
    ProposedStateOutcome::Success { output }
}

fn failure<O, F>(failure: F) -> ProposedStateOutcome<O, F> {
    ProposedStateOutcome::Failure { failure }
}

fn read_returned<'a>(
    evidence: &'a EvmReadEvidence,
    intent: &EvmReadIntent,
) -> Option<&'a EvmReadValue> {
    if evidence.validate_for(intent).is_err() {
        return None;
    }
    match evidence {
        EvmReadEvidence::Returned { value } => Some(value),
        EvmReadEvidence::Rejected
        | EvmReadEvidence::SafeFailure
        | EvmReadEvidence::IntegrityBlocked => None,
    }
}

fn work_initial_anchor(work: &EvmBalanceWork) -> Option<&EvmBlockAnchor> {
    match work {
        EvmBalanceWork::SelectAsset { initial_anchor, .. }
        | EvmBalanceWork::ReadNativeBalance { initial_anchor, .. }
        | EvmBalanceWork::ReadTokenDecimals { initial_anchor, .. }
        | EvmBalanceWork::ReadTokenBalance { initial_anchor, .. }
        | EvmBalanceWork::ConfirmAnchor { initial_anchor, .. } => Some(initial_anchor),
        EvmBalanceWork::CheckChainIdentity { .. }
        | EvmBalanceWork::ReadInitialAnchor { .. }
        | EvmBalanceWork::Complete => None,
    }
}

fn scale_units(raw: &str, source_decimals: u8, target_decimals: u8) -> Option<String> {
    if !is_decimal_integer(raw) || source_decimals > 30 || target_decimals > 30 {
        return None;
    }
    if source_decimals == target_decimals {
        return Some(raw.to_owned());
    }
    if source_decimals < target_decimals {
        let zeros = usize::from(target_decimals - source_decimals);
        let mut scaled = raw.to_owned();
        scaled.reserve(zeros);
        scaled.extend(std::iter::repeat_n('0', zeros));
        return (scaled.len() <= 80).then_some(scaled);
    }
    let shift = usize::from(source_decimals - target_decimals);
    if raw.len() <= shift {
        return Some("0".to_owned());
    }
    let (whole, discarded) = raw.split_at(raw.len() - shift);
    discarded
        .bytes()
        .all(|byte| byte == b'0')
        .then(|| whole.trim_start_matches('0'))
        .map(|value| {
            if value.is_empty() {
                "0".to_owned()
            } else {
                value.to_owned()
            }
        })
}

fn completed_total_scaled(
    request: &EvmBalanceRequest,
    completed: &[EvmBalanceResult],
) -> Option<String> {
    let amounts = completed
        .iter()
        .map(|result| request.scale_units(&result.raw_units, result.decimals))
        .collect::<Option<Vec<_>>>()?;
    sum_decimal(amounts.iter().map(String::as_str))
}

fn sum_decimal<'a>(values: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut digits = vec![0u8];
    for value in values {
        if !is_decimal_integer(value) {
            return None;
        }
        let mut carry = 0u16;
        let width = digits.len().max(value.len());
        digits.resize(width, 0);
        for offset in 0..width {
            let right = value
                .as_bytes()
                .get(value.len().wrapping_sub(offset + 1))
                .copied()
                .unwrap_or(b'0')
                .saturating_sub(b'0') as u16;
            let index = digits.len() - 1 - offset;
            let sum = u16::from(digits[index]) + right + carry;
            digits[index] = (sum % 10) as u8;
            carry = sum / 10;
        }
        if carry != 0 {
            digits.insert(0, carry as u8);
        }
        if digits.len() > 80 {
            return None;
        }
    }
    Some(
        digits
            .into_iter()
            .map(|digit| char::from(b'0' + digit))
            .collect(),
    )
}

fn is_decimal_integer(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value.as_bytes().iter().all(u8::is_ascii_digit)
        && (value == "0" || !value.starts_with('0'))
}

fn read_value_valid(value: &EvmReadValue) -> bool {
    match value {
        EvmReadValue::ChainId(chain_id) => *chain_id != 0,
        EvmReadValue::Anchor { number, hash } => read_anchor(number, hash).is_some(),
        EvmReadValue::RawUnits(units) => is_decimal_integer(units),
        EvmReadValue::TokenDecimals(decimals) => *decimals <= 30,
    }
}

fn read_value_valid_for_intent(intent: &EvmReadIntent, value: &EvmReadValue) -> bool {
    read_value_valid(value)
        && match (&intent.subject, value) {
            (EvmReadSubject::ChainIdentity, EvmReadValue::ChainId(chain_id))
                if *chain_id == intent.chain_id =>
            {
                true
            }
            (EvmReadSubject::ChainIdentity, EvmReadValue::ChainId(_)) => false,
            (EvmReadSubject::InitialAnchor, EvmReadValue::Anchor { .. })
            | (EvmReadSubject::NativeBalance { .. }, EvmReadValue::RawUnits(_))
            | (EvmReadSubject::TokenDecimals { .. }, EvmReadValue::TokenDecimals(_))
            | (EvmReadSubject::TokenBalance { .. }, EvmReadValue::RawUnits(_))
            | (EvmReadSubject::ConfirmAnchor { .. }, EvmReadValue::Anchor { .. }) => true,
            _ => false,
        }
}

fn read_anchor(number: &str, hash: &str) -> Option<EvmBlockAnchor> {
    EvmBlockAnchor::new(number.to_owned(), hash.to_owned()).ok()
}

fn duplicate_source_ids(sources: &[EvmBalanceSource]) -> bool {
    let mut ids = BTreeSet::new();
    sources
        .iter()
        .any(|source| !ids.insert(source.source_id.as_str()))
}

fn duplicate_collected_source_ids(balances: &[EvmCollectedBalance]) -> bool {
    let mut ids = BTreeSet::new();
    balances
        .iter()
        .any(|balance| !ids.insert(balance.source.source_id.as_str()))
}

fn valid_public_text(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && !string_contains_secret_marker(value)
        && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use mfm_ids::{ContentDigest, DigestAlgorithm, DigestBytes, EntryPointId, SchemaId};
    use mfm_program::{expand_program, Declaration, StateDeclaration};

    use super::*;

    #[derive(Debug, Serialize, Deserialize, MfmValue)]
    #[serde(deny_unknown_fields)]
    struct Continuation {
        value: u8,
    }

    fn route() -> ContentRef {
        ContentRef::new(
            SchemaId::new(
                "mfm.test.route",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([1; 32]),
            )
            .expect("schema"),
            ContentDigest::from_digest(DigestAlgorithm::Sha256V1, DigestBytes::from_array([2; 32])),
        )
        .expect("route")
    }

    fn context() -> EvmBalanceContext<Continuation> {
        let source = EvmBalanceSource {
            source_id: "wallet.native".to_owned(),
            chain_id: 1,
            address: "0x1111111111111111111111111111111111111111".to_owned(),
            token: None,
        };
        let request = EvmBalanceRequest::new(vec![source], 18).expect("request");
        EvmBalanceContext::new(
            request,
            Continuation { value: 1 },
            0,
            "collection".to_owned(),
            route(),
        )
        .expect("context")
    }

    fn state_at(declarations: &[Declaration], index: usize) -> &StateDeclaration {
        match &declarations[index] {
            Declaration::State(state) => state,
            Declaration::Match(_) => panic!("declaration {index} must be a State"),
        }
    }

    fn assert_state_success(declarations: &[Declaration], index: usize, success: Option<u16>) {
        let state = state_at(declarations, index);
        assert_eq!(state.next_index(), success, "success edge at {index}");
    }

    #[test]
    fn semantic_capabilities_and_states_preserve_exact_contracts() {
        fn assert_state<S, I, O, F>()
        where
            S: State<Input = I, Output = O, Failure = F>,
            I: MfmValueTrait,
            O: MfmValueTrait,
            F: MfmValueTrait,
        {
        }

        fn assert_read<S, C>()
        where
            S: ReadState<C>,
            C: ReadCapabilityContract,
        {
        }

        fn assert_pure<S: PureState>() {}

        assert_eq!(
            [
                EvmChainIdentityRead::contract_id().expect("chain identity capability"),
                EvmAnchorRead::contract_id().expect("anchor capability"),
                EvmBalanceRead::contract_id().expect("balance capability"),
            ]
            .map(|id| id.as_str().to_owned()),
            [
                "mfm.evm.capability.read-chain-identity@1",
                "mfm.evm.capability.read-anchor@1",
                "mfm.evm.capability.read-balance@1",
            ]
        );

        assert_state::<
            CheckChainIdentity<Continuation>,
            EvmBalanceContext<Continuation>,
            EvmBalanceContext<Continuation>,
            EvmBalanceFailure,
        >();
        assert_state::<
            ReadInitialAnchor<Continuation>,
            EvmBalanceContext<Continuation>,
            EvmBalanceContext<Continuation>,
            EvmBalanceFailure,
        >();
        assert_state::<
            SelectBalanceAsset<Continuation>,
            EvmBalanceContext<Continuation>,
            EvmBalanceAsset<Continuation>,
            EvmBalanceFailure,
        >();
        assert_state::<
            ReadNativeBalance<Continuation>,
            EvmBalanceContext<Continuation>,
            EvmBalanceContext<Continuation>,
            EvmBalanceFailure,
        >();
        assert_state::<
            ReadTokenDecimals<Continuation>,
            EvmBalanceContext<Continuation>,
            EvmBalanceContext<Continuation>,
            EvmBalanceFailure,
        >();
        assert_state::<
            ReadTokenBalance<Continuation>,
            EvmBalanceContext<Continuation>,
            EvmBalanceContext<Continuation>,
            EvmBalanceFailure,
        >();
        assert_state::<
            ConfirmBalanceAnchor<Continuation>,
            EvmBalanceContext<Continuation>,
            EvmBalanceContext<Continuation>,
            EvmBalanceFailure,
        >();
        assert_state::<
            ConsolidateBalanceCollection<Continuation>,
            EvmBalanceContext<Continuation>,
            EvmBalanceCollectionCompletion<Continuation>,
            EvmBalanceFailure,
        >();

        assert_read::<CheckChainIdentity<Continuation>, EvmChainIdentityRead>();
        assert_read::<ReadInitialAnchor<Continuation>, EvmAnchorRead>();
        assert_pure::<SelectBalanceAsset<Continuation>>();
        assert_read::<ReadNativeBalance<Continuation>, EvmBalanceRead>();
        assert_read::<ReadTokenDecimals<Continuation>, EvmBalanceRead>();
        assert_read::<ReadTokenBalance<Continuation>, EvmBalanceRead>();
        assert_read::<ConfirmBalanceAnchor<Continuation>, EvmAnchorRead>();
        assert_pure::<ConsolidateBalanceCollection<Continuation>>();

        assert_eq!(
            [
                CheckChainIdentity::<Continuation>::state_id().expect("check chain"),
                ReadInitialAnchor::<Continuation>::state_id().expect("initial anchor"),
                SelectBalanceAsset::<Continuation>::state_id().expect("select asset"),
                ReadNativeBalance::<Continuation>::state_id().expect("native balance"),
                ReadTokenDecimals::<Continuation>::state_id().expect("token decimals"),
                ReadTokenBalance::<Continuation>::state_id().expect("token balance"),
                ConfirmBalanceAnchor::<Continuation>::state_id().expect("confirm anchor"),
                ConsolidateBalanceCollection::<Continuation>::state_id().expect("consolidate"),
            ]
            .map(|id| id.as_str().to_owned()),
            [
                "mfm.evm.state.check-chain-identity@1",
                "mfm.evm.state.read-initial-anchor@1",
                "mfm.evm.state.select-asset@1",
                "mfm.evm.state.read-native-balance@1",
                "mfm.evm.state.read-token-decimals@1",
                "mfm.evm.state.read-token-balance@1",
                "mfm.evm.state.confirm-balance-anchor@1",
                "mfm.evm.state.consolidate-balance-collection@1",
            ]
        );
    }

    #[test]
    fn balance_operation_derives_topology_without_external_indices() {
        let target = EvmPhysicalTarget::new(1, route()).expect("target");
        let binding_ref = target.binding_ref().expect("binding");
        let operation =
            CollectEvmBalances::<Continuation>::new(binding_ref.clone(), 2).expect("operation");
        let program = expand_program(
            EntryPointId::new("mfm.test/evm-balance@1").expect("entry"),
            &operation,
        )
        .expect("Program");
        let declarations = program.declarations();
        assert_eq!(declarations.len(), 17);
        for (index, success) in [
            (0, Some(1)),
            (1, Some(2)),
            (2, Some(3)),
            (4, Some(7)),
            (5, Some(6)),
            (6, Some(7)),
            (7, Some(8)),
            (8, Some(9)),
            (9, Some(10)),
            (10, Some(11)),
            (12, Some(15)),
            (13, Some(14)),
            (14, Some(15)),
            (15, Some(16)),
            (16, None),
        ] {
            assert_state_success(declarations, index, success);
        }
        for (index, expected) in [
            (3, [("native", 4), ("token", 5)]),
            (11, [("native", 12), ("token", 13)]),
        ] {
            let Declaration::Match(selector) = &declarations[index] else {
                panic!("declaration {index} must be a Match");
            };
            let actual = selector
                .variants()
                .iter()
                .map(|variant| (variant.tag().as_str(), variant.entry_index()))
                .collect::<Vec<_>>();
            assert_eq!(actual, expected);
        }
        assert!(declarations
            .iter()
            .filter_map(|declaration| match declaration {
                Declaration::State(state) => state.execution().binding_ref(),
                Declaration::Match(_) => None,
            })
            .all(|binding| binding == &binding_ref));
        assert_eq!(
            declarations
                .iter()
                .filter(|declaration| matches!(declaration, Declaration::State(state) if !state.execution().is_pure()))
                .count(),
            12
        );
        assert!(CollectEvmBalances::<Continuation>::new(binding_ref.clone(), 1).is_ok());
        assert!(CollectEvmBalances::<Continuation>::new(binding_ref.clone(), 64).is_ok());
        assert!(CollectEvmBalances::<Continuation>::new(binding_ref.clone(), 0).is_err());
        assert!(CollectEvmBalances::<Continuation>::new(binding_ref, 65).is_err());
    }

    #[test]
    fn ordinary_read_interpreter_maps_every_failure_evidence_variant() {
        for evidence in [EvmReadEvidence::Rejected, EvmReadEvidence::SafeFailure] {
            let ProposedStateOutcome::Failure { failure } =
                <CheckChainIdentity<Continuation> as ReadState<EvmChainIdentityRead>>::interpret(
                    context(),
                    &evidence,
                )
            else {
                panic!("rejection evidence must be a typed failure");
            };
            assert!(matches!(
                failure,
                EvmBalanceFailure::SourceUnavailable {
                    ref stage,
                    collection_ordinal: 0,
                    ref code,
                } if stage == "check_chain_identity" && code == "chain_identity_unavailable"
            ));
        }

        let ProposedStateOutcome::Failure { failure } =
            <CheckChainIdentity<Continuation> as ReadState<EvmChainIdentityRead>>::interpret(
                context(),
                &EvmReadEvidence::IntegrityBlocked,
            )
        else {
            panic!("integrity evidence must be a typed failure");
        };
        assert!(matches!(
            failure,
            EvmBalanceFailure::IntegrityBlocked {
                ref stage,
                collection_ordinal: 0,
                ref code,
            } if stage == "check_chain_identity" && code == "integrity_blocked"
        ));
    }
}
