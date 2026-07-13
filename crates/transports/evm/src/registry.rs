use super::*;

/// Runtime EVM JSON-RPC source.
#[derive(Clone, PartialEq, Eq)]
pub struct EvmRuntimeSource {
    pub(super) id: EvmSourceRef,
    pub(super) rpc_url: String,
    pub(super) authorization: Option<String>,
}

impl EvmRuntimeSource {
    /// Creates a runtime EVM source.
    pub fn new(
        id: EvmSourceRef,
        rpc_url: impl Into<String>,
        authorization: Option<String>,
    ) -> TransportResult<Self> {
        let rpc_url = rpc_url.into();
        if rpc_url.trim().is_empty() || reqwest::Url::parse(&rpc_url).is_err() {
            return Err(EvmTransportError::InvalidRegistry);
        }
        Ok(Self {
            id,
            rpc_url,
            authorization: authorization.filter(|value| !value.trim().is_empty()),
        })
    }

    /// Returns the source id.
    pub fn id(&self) -> &EvmSourceRef {
        &self.id
    }
}

impl fmt::Debug for EvmRuntimeSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvmRuntimeSource")
            .field("id", &self.id)
            .field("rpc_url", &"<redacted>")
            .field(
                "authorization",
                &self.authorization.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

/// Ordered EVM source fallback policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmSourcePolicy {
    id: EvmSourcePolicyId,
    ordered_sources: Vec<EvmSourceRef>,
}

impl EvmSourcePolicy {
    /// Creates an ordered source policy.
    pub fn new(id: EvmSourcePolicyId, ordered_sources: Vec<EvmSourceRef>) -> TransportResult<Self> {
        if ordered_sources.is_empty() {
            return Err(EvmTransportError::InvalidRegistry);
        }
        Ok(Self {
            id,
            ordered_sources,
        })
    }

    /// Returns the policy id.
    pub fn id(&self) -> &EvmSourcePolicyId {
        &self.id
    }

    /// Returns the ordered source ids.
    pub fn ordered_sources(&self) -> &[EvmSourceRef] {
        &self.ordered_sources
    }
}

/// Runtime EVM source registry.
#[derive(Clone, PartialEq, Eq)]
pub struct EvmSourceRegistry {
    sources: BTreeMap<EvmSourceRef, EvmRuntimeSource>,
    policies: BTreeMap<EvmSourcePolicyId, EvmSourcePolicy>,
}

impl EvmSourceRegistry {
    /// Creates a registry from runtime sources and policies.
    pub fn new(
        sources: impl IntoIterator<Item = EvmRuntimeSource>,
        policies: impl IntoIterator<Item = EvmSourcePolicy>,
    ) -> TransportResult<Self> {
        let mut source_map = BTreeMap::new();
        for source in sources {
            if source_map.insert(source.id.clone(), source).is_some() {
                return Err(EvmTransportError::InvalidRegistry);
            }
        }
        let mut policy_map = BTreeMap::new();
        for policy in policies {
            for source_id in policy.ordered_sources() {
                if !source_map.contains_key(source_id) {
                    return Err(EvmTransportError::InvalidRegistry);
                }
            }
            if policy_map.insert(policy.id.clone(), policy).is_some() {
                return Err(EvmTransportError::InvalidRegistry);
            }
        }
        Ok(Self {
            sources: source_map,
            policies: policy_map,
        })
    }

    /// Creates a registry with one source and one policy.
    pub fn single_source(
        source: EvmRuntimeSource,
        policy_id: EvmSourcePolicyId,
    ) -> TransportResult<Self> {
        let source_id = source.id.clone();
        let policy = EvmSourcePolicy::new(policy_id, vec![source_id])?;
        Self::new([source], [policy])
    }

    pub(super) fn candidates<'a>(
        &'a self,
        policy_id: &EvmSourcePolicyId,
        source_ref: &EvmSourceRef,
    ) -> TransportResult<Vec<&'a EvmRuntimeSource>> {
        let policy = self
            .policies
            .get(policy_id)
            .ok_or(EvmTransportError::PolicyUnavailable)?;
        let start = policy
            .ordered_sources
            .iter()
            .position(|candidate| candidate == source_ref)
            .ok_or(EvmTransportError::SourceNotAllowed)?;
        let mut seen = BTreeSet::new();
        let mut candidates = Vec::new();
        for source_id in policy.ordered_sources[start..]
            .iter()
            .chain(policy.ordered_sources[..start].iter())
        {
            if seen.insert(source_id.clone()) {
                let source = self
                    .sources
                    .get(source_id)
                    .ok_or(EvmTransportError::SourceUnavailable)?;
                candidates.push(source);
            }
        }
        Ok(candidates)
    }
}

impl fmt::Debug for EvmSourceRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvmSourceRegistry")
            .field("sources", &self.sources)
            .field("policies", &self.policies)
            .finish()
    }
}

/// Runtime route from a semantic EVM network id to a local source policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmRoute {
    network_id: EvmNetworkId,
    source_ref: EvmSourceRef,
    policy_id: EvmSourcePolicyId,
}

impl EvmRoute {
    /// Creates a runtime route.
    pub fn new(
        network_id: EvmNetworkId,
        source_ref: EvmSourceRef,
        policy_id: EvmSourcePolicyId,
    ) -> Self {
        Self {
            network_id,
            source_ref,
            policy_id,
        }
    }

    /// Returns the semantic network id.
    pub const fn network_id(&self) -> &EvmNetworkId {
        &self.network_id
    }

    /// Returns the preferred source reference.
    pub const fn source_ref(&self) -> &EvmSourceRef {
        &self.source_ref
    }

    /// Returns the source policy id.
    pub const fn policy_id(&self) -> &EvmSourcePolicyId {
        &self.policy_id
    }
}

/// Runtime EVM route registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmRouteRegistry {
    routes: BTreeMap<EvmNetworkId, EvmRoute>,
}

impl EvmRouteRegistry {
    /// Creates a runtime route registry, rejecting duplicate semantic network ids.
    pub fn new(routes: impl IntoIterator<Item = EvmRoute>) -> TransportResult<Self> {
        let mut by_network = BTreeMap::new();
        for route in routes {
            if by_network
                .insert(route.network_id().clone(), route)
                .is_some()
            {
                return Err(EvmTransportError::InvalidRegistry);
            }
        }
        Ok(Self { routes: by_network })
    }

    /// Returns the route for a semantic network id.
    pub fn route(&self, network_id: &EvmNetworkId) -> TransportResult<&EvmRoute> {
        self.routes
            .get(network_id)
            .ok_or(EvmTransportError::RouteUnavailable)
    }
}
