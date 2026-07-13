use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use mfm_evm_capabilities::{EvmNetworkId, EvmSourcePolicyId, EvmSourceRef};

use super::raw::{RawEvmConfig, RawEvmPolicy, RawEvmRoute, RawEvmSource};
use super::resolve::{
    parse_policy_id, parse_source_ref, resolve_optional_value, resolve_required_value,
    validate_rpc_url,
};
use super::{
    reject_extra_fields, Result, RuntimeConfigError, RuntimeConfigErrorKind,
    RuntimeConfigIdentifierKind, RuntimeConfigLocation, RuntimeSecretValue,
};

/// Runtime-local EVM descriptors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmRuntimeConfig {
    sources: BTreeMap<EvmSourceRef, EvmRpcSource>,
    policies: BTreeMap<EvmSourcePolicyId, EvmSourcePolicy>,
    routes: BTreeMap<EvmNetworkId, EvmRoute>,
}

impl EvmRuntimeConfig {
    /// Returns configured EVM JSON-RPC sources.
    pub const fn sources(&self) -> &BTreeMap<EvmSourceRef, EvmRpcSource> {
        &self.sources
    }

    /// Returns configured and synthesized EVM source policies.
    pub const fn policies(&self) -> &BTreeMap<EvmSourcePolicyId, EvmSourcePolicy> {
        &self.policies
    }

    /// Returns semantic network routes.
    pub const fn routes(&self) -> &BTreeMap<EvmNetworkId, EvmRoute> {
        &self.routes
    }

    pub(super) fn from_raw(raw: RawEvmConfig) -> Result<Self> {
        reject_extra_fields(&raw.extra, RuntimeConfigLocation::Evm)?;

        let mut sources = BTreeMap::new();
        for (raw_id, raw_source) in raw.sources {
            let source_ref = parse_source_ref(
                &raw_id,
                RuntimeConfigLocation::EvmSource { source_ref: None },
            )?;
            let location = RuntimeConfigLocation::EvmSource {
                source_ref: Some(source_ref.to_string()),
            };
            let source = EvmRpcSource::from_raw(raw_source, location)?;
            sources.insert(source_ref, source);
        }

        let mut policies = BTreeMap::new();
        for (raw_id, raw_policy) in raw.policies {
            let policy_id = parse_policy_id(
                &raw_id,
                RuntimeConfigLocation::EvmPolicy { policy_id: None },
            )?;
            let location = RuntimeConfigLocation::EvmPolicy {
                policy_id: Some(policy_id.to_string()),
            };
            let policy = EvmSourcePolicy::from_raw(raw_policy, location)?;
            for source_ref in policy.ordered_sources() {
                if !sources.contains_key(source_ref) {
                    return Err(RuntimeConfigError::new(
                        RuntimeConfigLocation::EvmPolicy {
                            policy_id: Some(policy_id.to_string()),
                        },
                        RuntimeConfigErrorKind::MissingSource,
                    ));
                }
            }
            policies.insert(policy_id, policy);
        }

        let explicit_policy_ids = policies.keys().cloned().collect::<BTreeSet<_>>();
        let mut routes = BTreeMap::new();
        for (raw_network_id, raw_route) in raw.routes {
            let network_id = EvmNetworkId::new(&raw_network_id).map_err(|_| {
                RuntimeConfigError::new(
                    RuntimeConfigLocation::EvmRoute { network_id: None },
                    RuntimeConfigErrorKind::InvalidIdentifier {
                        kind: RuntimeConfigIdentifierKind::NetworkId,
                    },
                )
            })?;
            let location = RuntimeConfigLocation::EvmRoute {
                network_id: Some(network_id.to_string()),
            };
            let route = EvmRoute::from_raw(
                raw_route,
                location.clone(),
                &sources,
                &mut policies,
                &explicit_policy_ids,
            )?;
            routes.insert(network_id, route);
        }

        Ok(Self {
            sources,
            policies,
            routes,
        })
    }
}

/// Runtime EVM JSON-RPC source descriptor.
#[derive(Clone, PartialEq, Eq)]
pub struct EvmRpcSource {
    rpc_url: RuntimeSecretValue,
    auth_header: Option<RuntimeSecretValue>,
}

impl EvmRpcSource {
    /// Returns the resolved RPC URL.
    pub const fn rpc_url(&self) -> &RuntimeSecretValue {
        &self.rpc_url
    }

    /// Returns the resolved authorization header, when configured.
    pub const fn auth_header(&self) -> Option<&RuntimeSecretValue> {
        self.auth_header.as_ref()
    }

    fn from_raw(raw: RawEvmSource, location: RuntimeConfigLocation) -> Result<Self> {
        reject_extra_fields(&raw.extra, location.clone())?;

        let rpc_url = resolve_required_value(
            location.clone(),
            "rpc_url",
            &raw.rpc_url,
            &raw.rpc_url_env,
            &raw.rpc_url_file,
            &raw.rpc_url_file_env,
        )?;
        validate_rpc_url(&rpc_url, location.clone().with_field("rpc_url"))?;

        let auth_header = resolve_optional_value(
            location,
            "auth_header",
            &raw.auth_header,
            &raw.auth_header_env,
            &raw.auth_header_file,
            &raw.auth_header_file_env,
        )?;

        Ok(Self {
            rpc_url,
            auth_header,
        })
    }
}

impl fmt::Debug for EvmRpcSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvmRpcSource")
            .field("rpc_url", &self.rpc_url)
            .field("auth_header", &self.auth_header)
            .finish()
    }
}

/// Ordered EVM source fallback policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmSourcePolicy {
    ordered_sources: Vec<EvmSourceRef>,
    synthesized: bool,
}

impl EvmSourcePolicy {
    /// Creates an explicit runtime EVM source policy.
    pub fn explicit(ordered_sources: Vec<EvmSourceRef>) -> Self {
        Self {
            ordered_sources,
            synthesized: false,
        }
    }

    /// Returns ordered source references for this policy.
    pub fn ordered_sources(&self) -> &[EvmSourceRef] {
        &self.ordered_sources
    }

    /// Returns true when this policy was synthesized from a same-id route.
    pub const fn is_synthesized(&self) -> bool {
        self.synthesized
    }

    fn synthesized(source_ref: EvmSourceRef) -> Self {
        Self {
            ordered_sources: vec![source_ref],
            synthesized: true,
        }
    }

    fn from_raw(raw: RawEvmPolicy, location: RuntimeConfigLocation) -> Result<Self> {
        reject_extra_fields(&raw.extra, location.clone())?;
        let Some(raw_sources) = raw.ordered_sources else {
            return Err(RuntimeConfigError::new(
                location.with_field("ordered_sources"),
                RuntimeConfigErrorKind::MissingRequiredField,
            ));
        };
        if raw_sources.is_empty() {
            return Err(RuntimeConfigError::new(
                location.with_field("ordered_sources"),
                RuntimeConfigErrorKind::EmptyPolicy,
            ));
        }

        let mut seen = BTreeSet::new();
        let mut ordered_sources = Vec::new();
        for raw_source_ref in raw_sources {
            let source_ref = parse_source_ref(&raw_source_ref, location.clone())?;
            if !seen.insert(source_ref.clone()) {
                return Err(RuntimeConfigError::new(
                    location.with_field("ordered_sources"),
                    RuntimeConfigErrorKind::DuplicatePolicySource,
                ));
            }
            ordered_sources.push(source_ref);
        }
        Ok(Self::explicit(ordered_sources))
    }
}

/// Runtime EVM route from a semantic network id to local source policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmRoute {
    source_ref: EvmSourceRef,
    policy_id: EvmSourcePolicyId,
}

impl EvmRoute {
    /// Returns the preferred local source reference.
    pub const fn source_ref(&self) -> &EvmSourceRef {
        &self.source_ref
    }

    /// Returns the source policy id.
    pub const fn policy_id(&self) -> &EvmSourcePolicyId {
        &self.policy_id
    }

    fn from_raw(
        raw: RawEvmRoute,
        location: RuntimeConfigLocation,
        sources: &BTreeMap<EvmSourceRef, EvmRpcSource>,
        policies: &mut BTreeMap<EvmSourcePolicyId, EvmSourcePolicy>,
        explicit_policy_ids: &BTreeSet<EvmSourcePolicyId>,
    ) -> Result<Self> {
        reject_extra_fields(&raw.extra, location.clone())?;

        let Some(raw_source_ref) = raw.source_ref else {
            return Err(RuntimeConfigError::new(
                location.with_field("source_ref"),
                RuntimeConfigErrorKind::MissingRequiredField,
            ));
        };
        let source_ref = parse_source_ref(&raw_source_ref, location.clone())?;
        if !sources.contains_key(&source_ref) {
            return Err(RuntimeConfigError::new(
                location.with_field("source_ref"),
                RuntimeConfigErrorKind::MissingSource,
            ));
        }

        let policy_id = match raw.policy_id {
            Some(raw_policy_id) => parse_policy_id(&raw_policy_id, location.clone())?,
            None => {
                let same_id = EvmSourcePolicyId::new(source_ref.as_str()).map_err(|_| {
                    RuntimeConfigError::new(
                        location.clone().with_field("policy_id"),
                        RuntimeConfigErrorKind::InvalidIdentifier {
                            kind: RuntimeConfigIdentifierKind::PolicyId,
                        },
                    )
                })?;
                if explicit_policy_ids.contains(&same_id) {
                    return Err(RuntimeConfigError::new(
                        location.clone().with_field("policy_id"),
                        RuntimeConfigErrorKind::SameIdPolicyRequiresExplicitPolicyId,
                    ));
                }
                policies
                    .entry(same_id.clone())
                    .or_insert_with(|| EvmSourcePolicy::synthesized(source_ref.clone()));
                same_id
            }
        };

        let policy = policies.get(&policy_id).ok_or_else(|| {
            RuntimeConfigError::new(
                location.clone().with_field("policy_id"),
                RuntimeConfigErrorKind::MissingPolicy,
            )
        })?;
        if !policy.ordered_sources().contains(&source_ref) {
            return Err(RuntimeConfigError::new(
                location.with_field("source_ref"),
                RuntimeConfigErrorKind::SourceNotInPolicy,
            ));
        }

        Ok(Self {
            source_ref,
            policy_id,
        })
    }
}
