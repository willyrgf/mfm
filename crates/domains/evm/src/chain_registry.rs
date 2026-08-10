//! Secret-free qualified chain-registry and route-membership contracts.

use std::collections::{BTreeMap, BTreeSet};

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{ContentRef, LocalPublicId, StableId};
use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

use crate::wallet_authority::{
    canonical_wallet_reference, derive_qualified_chain_instance_id, ChainInstanceDeclaration,
    QualifiedChainInstanceId,
};
use crate::{EvmRoutingGenerationRef, EvmWalletReference, WalletAuthorityContractError};

/// Fixed provider class reviewed by the EVM JSON-RPC adapter.
pub const EVM_JSON_RPC_PROVIDER_CLASS: &str = "mfm.evm-json-rpc";
/// Fixed route policy: one endpoint entry, no retry, redirect, or fallback.
pub const EVM_ROUTE_POLICY_ID: &str = "mfm.evm.single-entry-no-retry";
/// Version of the fixed route policy.
pub const EVM_ROUTE_POLICY_VERSION: &str = "1";
/// Exact routing-generation descriptor version.
pub const EVM_ROUTING_GENERATION_DESCRIPTOR_VERSION: &str = "mfm.evm.routing-generation.v1";
/// Exact aggregate routing-catalog descriptor version.
pub const EVM_ROUTING_CATALOG_DESCRIPTOR_VERSION: &str = "mfm.evm.routing-catalog.v1";

/// Provider-issued public attestation for one never-reused physical chain instance.
///
/// Structural validation proves content addressing only. Authenticity is established by the
/// separately authenticated deployment provider and is deliberately not represented by a
/// persisted signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "chain-instance-registry-attestation",
    version = "1",
    schema = "mfm.evm.chain_instance_registry_attestation"
)]
pub struct ChainInstanceRegistryAttestation {
    declaration_ref: EvmWalletReference,
    registry_issuance_ref: EvmWalletReference,
    registry_head_ref_at_issuance: EvmWalletReference,
    declaration: ChainInstanceDeclaration,
}

impl ChainInstanceRegistryAttestation {
    /// Constructs one structurally exact public registry attestation.
    pub fn new(
        declaration: ChainInstanceDeclaration,
        registry_issuance_ref: EvmWalletReference,
        registry_head_ref_at_issuance: EvmWalletReference,
    ) -> Result<Self, WalletAuthorityContractError> {
        declaration.validate()?;
        validate_reference(&registry_issuance_ref, "chain_registry_issuance_ref")?;
        validate_reference(&registry_head_ref_at_issuance, "chain_registry_head_ref")?;
        let declaration_ref = canonical_wallet_reference(&declaration)?;
        Ok(Self {
            declaration_ref,
            registry_issuance_ref,
            registry_head_ref_at_issuance,
            declaration,
        })
    }

    /// Revalidates canonical references and the complete declaration closure.
    pub fn validate(&self) -> Result<(), WalletAuthorityContractError> {
        self.declaration.validate()?;
        validate_reference(&self.registry_issuance_ref, "chain_registry_issuance_ref")?;
        validate_reference(
            &self.registry_head_ref_at_issuance,
            "chain_registry_head_ref",
        )?;
        if self.declaration_ref != canonical_wallet_reference(&self.declaration)? {
            return Err(WalletAuthorityContractError::Invalid(
                "chain_declaration_ref",
            ));
        }
        Ok(())
    }

    /// Returns the exact declaration content reference.
    pub const fn declaration_ref(&self) -> &EvmWalletReference {
        &self.declaration_ref
    }

    /// Returns the provider-assigned permanent issuance reference.
    pub const fn registry_issuance_ref(&self) -> &EvmWalletReference {
        &self.registry_issuance_ref
    }

    /// Returns the append-only chain-registry head at issuance.
    pub const fn registry_head_ref_at_issuance(&self) -> &EvmWalletReference {
        &self.registry_head_ref_at_issuance
    }

    /// Returns the full canonical chain declaration.
    pub const fn declaration(&self) -> &ChainInstanceDeclaration {
        &self.declaration
    }

    /// Derives the compact exact chain binding carried by runtime values.
    pub fn binding(&self) -> Result<EvmChainInstanceBinding, WalletAuthorityContractError> {
        EvmChainInstanceBinding::from_attestation(self)
    }

    /// Returns this complete attestation's content reference.
    pub fn content_ref(&self) -> Result<EvmWalletReference, WalletAuthorityContractError> {
        self.validate()?;
        canonical_wallet_reference(self)
    }
}

/// Compact exact identity shared by intents, routes, activation, and live bindings.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "chain-instance-binding",
    version = "1",
    schema = "mfm.evm.chain_instance_binding"
)]
pub struct EvmChainInstanceBinding {
    chain_instance_attestation_ref: EvmWalletReference,
    qualified_chain_instance_id: QualifiedChainInstanceId,
    chain_id: u64,
}

impl EvmChainInstanceBinding {
    /// Derives one compact binding from its complete attestation.
    pub fn from_attestation(
        attestation: &ChainInstanceRegistryAttestation,
    ) -> Result<Self, WalletAuthorityContractError> {
        attestation.validate()?;
        Ok(Self {
            chain_instance_attestation_ref: attestation.content_ref()?,
            qualified_chain_instance_id: derive_qualified_chain_instance_id(
                attestation.declaration(),
            )?,
            chain_id: attestation.declaration().chain_id(),
        })
    }

    /// Revalidates the compact representation.
    pub fn validate(&self) -> Result<(), WalletAuthorityContractError> {
        validate_reference(
            &self.chain_instance_attestation_ref,
            "chain_instance_attestation_ref",
        )?;
        self.qualified_chain_instance_id.validate()?;
        if self.chain_id == 0 {
            return Err(WalletAuthorityContractError::Invalid("chain_id"));
        }
        Ok(())
    }

    /// Requires this binding to be the exact derivation of `attestation`.
    pub fn validate_attestation(
        &self,
        attestation: &ChainInstanceRegistryAttestation,
    ) -> Result<(), WalletAuthorityContractError> {
        if self != &Self::from_attestation(attestation)? {
            return Err(WalletAuthorityContractError::Invalid(
                "chain_instance_binding",
            ));
        }
        Ok(())
    }

    /// Returns the full chain-attestation content reference.
    pub const fn chain_instance_attestation_ref(&self) -> &EvmWalletReference {
        &self.chain_instance_attestation_ref
    }

    /// Returns the qualified physical-chain identity.
    pub const fn qualified_chain_instance_id(&self) -> &QualifiedChainInstanceId {
        &self.qualified_chain_instance_id
    }

    /// Returns the exact non-zero EVM chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }
}

/// Secret-free identity of one provider-qualified route generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "routing-generation-descriptor",
    version = "1",
    schema = "mfm.evm.routing_generation_descriptor"
)]
pub struct EvmRoutingGenerationDescriptor {
    version: String,
    network_id: String,
    source_ref: String,
    generation_id: String,
    provider_class: String,
    route_policy_id: String,
    route_policy_version: String,
    route_membership_issuance_ref: EvmWalletReference,
    chain_instance: EvmChainInstanceBinding,
}

impl EvmRoutingGenerationDescriptor {
    /// Constructs one exact public route membership.
    pub fn new(
        network_id: impl Into<String>,
        source_ref: impl Into<String>,
        generation_id: StableId,
        route_membership_issuance_ref: EvmWalletReference,
        chain_instance: EvmChainInstanceBinding,
    ) -> Result<Self, WalletAuthorityContractError> {
        let value = Self {
            version: EVM_ROUTING_GENERATION_DESCRIPTOR_VERSION.to_owned(),
            network_id: network_id.into(),
            source_ref: source_ref.into(),
            generation_id: generation_id.as_str().to_owned(),
            provider_class: EVM_JSON_RPC_PROVIDER_CLASS.to_owned(),
            route_policy_id: EVM_ROUTE_POLICY_ID.to_owned(),
            route_policy_version: EVM_ROUTE_POLICY_VERSION.to_owned(),
            route_membership_issuance_ref,
            chain_instance,
        };
        value.validate()?;
        Ok(value)
    }

    /// Revalidates the exact fixed route contract.
    pub fn validate(&self) -> Result<(), WalletAuthorityContractError> {
        LocalPublicId::new(&self.network_id)
            .map_err(|_| WalletAuthorityContractError::Invalid("network_id"))?;
        LocalPublicId::new(&self.source_ref)
            .map_err(|_| WalletAuthorityContractError::Invalid("source_ref"))?;
        StableId::new(&self.generation_id)
            .map_err(|_| WalletAuthorityContractError::Invalid("generation_id"))?;
        validate_reference(
            &self.route_membership_issuance_ref,
            "route_membership_issuance_ref",
        )?;
        self.chain_instance.validate()?;
        if self.version != EVM_ROUTING_GENERATION_DESCRIPTOR_VERSION
            || self.provider_class != EVM_JSON_RPC_PROVIDER_CLASS
            || self.route_policy_id != EVM_ROUTE_POLICY_ID
            || self.route_policy_version != EVM_ROUTE_POLICY_VERSION
        {
            return Err(WalletAuthorityContractError::Invalid(
                "routing_generation_descriptor",
            ));
        }
        Ok(())
    }

    /// Returns the exact descriptor version.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns the semantic network id.
    pub fn network_id(&self) -> &str {
        &self.network_id
    }

    /// Returns the reviewed local route source id.
    pub fn source_ref(&self) -> &str {
        &self.source_ref
    }

    /// Returns the immutable route-generation identity.
    pub fn generation_id(&self) -> &str {
        &self.generation_id
    }

    /// Returns the fixed provider class.
    pub fn provider_class(&self) -> &str {
        &self.provider_class
    }

    /// Returns the fixed one-entry route policy.
    pub fn route_policy_id(&self) -> &str {
        &self.route_policy_id
    }

    /// Returns the fixed route-policy version.
    pub fn route_policy_version(&self) -> &str {
        &self.route_policy_version
    }

    /// Returns the provider-issued route-membership reference.
    pub const fn route_membership_issuance_ref(&self) -> &EvmWalletReference {
        &self.route_membership_issuance_ref
    }

    /// Returns the exact qualified chain binding.
    pub const fn chain_instance(&self) -> &EvmChainInstanceBinding {
        &self.chain_instance
    }

    /// Returns the EVM chain id through the exact binding.
    pub const fn chain_id(&self) -> u64 {
        self.chain_instance.chain_id()
    }

    /// Returns this generation descriptor's exact route reference.
    pub fn generation_ref(&self) -> Result<EvmRoutingGenerationRef, WalletAuthorityContractError> {
        self.validate()?;
        EvmRoutingGenerationRef::from_content_ref(
            canonical_wallet_reference(self)?
                .to_content_ref()
                .map_err(|_| WalletAuthorityContractError::Invalid("routing_generation_ref"))?,
        )
        .map_err(|_| WalletAuthorityContractError::Invalid("routing_generation_ref"))
    }

    /// Returns exact canonical descriptor bytes.
    pub fn canonical(&self) -> Result<PlainCanonicalJsonBytes, WalletAuthorityContractError> {
        self.validate()?;
        canonical_json(self)
    }
}

/// Complete secret-free routing closure authenticated by the deployment provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "routing-catalog-descriptor",
    version = "1",
    schema = "mfm.evm.routing_catalog_descriptor"
)]
pub struct EvmRoutingCatalogDescriptor {
    version: String,
    chain_registry_head_ref: EvmWalletReference,
    chain_instances: Vec<ChainInstanceRegistryAttestation>,
    generations: Vec<EvmRoutingGenerationDescriptor>,
}

impl EvmRoutingCatalogDescriptor {
    /// Constructs a canonical complete catalog closure.
    pub fn new(
        chain_registry_head_ref: EvmWalletReference,
        chain_instances: Vec<ChainInstanceRegistryAttestation>,
        generations: Vec<EvmRoutingGenerationDescriptor>,
    ) -> Result<Self, WalletAuthorityContractError> {
        validate_reference(&chain_registry_head_ref, "chain_registry_head_ref")?;
        let mut chain_instances = chain_instances
            .into_iter()
            .map(|attestation| {
                let reference = attestation.content_ref()?;
                Ok((reference.content_digest().to_owned(), attestation))
            })
            .collect::<Result<Vec<_>, WalletAuthorityContractError>>()?;
        chain_instances.sort_by(|left, right| left.0.cmp(&right.0));
        let chain_instances = chain_instances
            .into_iter()
            .map(|(_, attestation)| attestation)
            .collect();
        let mut generations = generations
            .into_iter()
            .map(|generation| {
                let reference = generation.generation_ref()?;
                Ok((reference.content_digest().to_owned(), generation))
            })
            .collect::<Result<Vec<_>, WalletAuthorityContractError>>()?;
        generations.sort_by(|left, right| left.0.cmp(&right.0));
        let generations = generations
            .into_iter()
            .map(|(_, generation)| generation)
            .collect();
        let value = Self {
            version: EVM_ROUTING_CATALOG_DESCRIPTOR_VERSION.to_owned(),
            chain_registry_head_ref,
            chain_instances,
            generations,
        };
        value.validate()?;
        Ok(value)
    }

    /// Revalidates ordering, uniqueness, and exact route-to-chain closure.
    pub fn validate(&self) -> Result<(), WalletAuthorityContractError> {
        validate_reference(&self.chain_registry_head_ref, "chain_registry_head_ref")?;
        if self.version != EVM_ROUTING_CATALOG_DESCRIPTOR_VERSION
            || self.chain_instances.is_empty()
            || self.generations.is_empty()
        {
            return Err(WalletAuthorityContractError::Invalid(
                "routing_catalog_descriptor",
            ));
        }
        let mut attestations = BTreeMap::new();
        let mut namespaces = BTreeMap::new();
        let mut registry_issuances = BTreeSet::new();
        let mut authority_references = BTreeMap::new();
        retain_authority_reference(
            &mut authority_references,
            &self.chain_registry_head_ref,
            RoutingAuthorityReferenceKind::RegistryHead,
        )?;
        let mut prior_attestation_ref: Option<EvmWalletReference> = None;
        for attestation in &self.chain_instances {
            attestation.validate()?;
            let reference = attestation.content_ref()?;
            retain_authority_reference(
                &mut authority_references,
                attestation.declaration().chain_registry_lineage_ref(),
                RoutingAuthorityReferenceKind::RegistryLineage,
            )?;
            retain_authority_reference(
                &mut authority_references,
                attestation.registry_head_ref_at_issuance(),
                RoutingAuthorityReferenceKind::RegistryHead,
            )?;
            retain_authority_reference(
                &mut authority_references,
                attestation.registry_issuance_ref(),
                RoutingAuthorityReferenceKind::ChainIssuance,
            )?;
            let namespace_key = (
                attestation
                    .declaration()
                    .chain_registry_lineage_ref()
                    .clone(),
                attestation.declaration().instance_namespace_id().to_owned(),
            );
            if prior_attestation_ref
                .as_ref()
                .is_some_and(|prior| prior >= &reference)
                || attestations
                    .insert(reference.clone(), attestation.binding()?)
                    .is_some()
            {
                return Err(WalletAuthorityContractError::Invalid(
                    "chain_instance_order",
                ));
            }
            if namespaces
                .insert(namespace_key, reference.clone())
                .is_some()
            {
                return Err(WalletAuthorityContractError::Invalid(
                    "chain_instance_namespace_reuse",
                ));
            }
            if !registry_issuances.insert(attestation.registry_issuance_ref().clone()) {
                return Err(WalletAuthorityContractError::Invalid(
                    "chain_registry_issuance_reuse",
                ));
            }
            prior_attestation_ref = Some(reference);
        }
        let mut prior_generation_ref: Option<EvmRoutingGenerationRef> = None;
        let mut memberships = BTreeSet::new();
        let mut route_generation_ids = BTreeSet::new();
        let mut used_attestations = BTreeSet::new();
        for generation in &self.generations {
            generation.validate()?;
            let generation_ref = generation.generation_ref()?;
            retain_authority_reference(
                &mut authority_references,
                generation.route_membership_issuance_ref(),
                RoutingAuthorityReferenceKind::RouteMembership,
            )?;
            if prior_generation_ref
                .as_ref()
                .is_some_and(|prior| prior >= &generation_ref)
                || !memberships.insert(generation.route_membership_issuance_ref.clone())
                || !route_generation_ids.insert(generation.generation_id.clone())
            {
                return Err(WalletAuthorityContractError::Invalid(
                    "route_generation_order",
                ));
            }
            let attestation_ref = generation
                .chain_instance
                .chain_instance_attestation_ref()
                .clone();
            if attestations.get(&attestation_ref) != Some(&generation.chain_instance) {
                return Err(WalletAuthorityContractError::Invalid(
                    "route_chain_membership",
                ));
            }
            used_attestations.insert(attestation_ref);
            prior_generation_ref = Some(generation_ref);
        }
        if used_attestations.len() != attestations.len() {
            return Err(WalletAuthorityContractError::Invalid(
                "unused_chain_attestation",
            ));
        }
        Ok(())
    }

    /// Revalidates a complete ordered append-only registry history.
    ///
    /// Catalogs are ordered from oldest to current. Every later catalog must retain every prior
    /// attestation and route generation exactly, while permanent namespaces, issuance references,
    /// bare generation ids, and route-to-chain assignments may only repeat their original binding.
    pub fn validate_append_only_history(
        chain_registry_lineage_ref: &EvmWalletReference,
        current_chain_registry_head_ref: &EvmWalletReference,
        catalogs: &[Self],
    ) -> Result<(), WalletAuthorityContractError> {
        validate_reference(chain_registry_lineage_ref, "chain_registry_lineage_ref")?;
        validate_reference(current_chain_registry_head_ref, "chain_registry_head_ref")?;
        if catalogs.last().map(Self::chain_registry_head_ref)
            != Some(current_chain_registry_head_ref)
        {
            return Err(WalletAuthorityContractError::Invalid(
                "current_chain_registry_head",
            ));
        }

        let mut authority_references = BTreeMap::new();
        retain_authority_reference(
            &mut authority_references,
            chain_registry_lineage_ref,
            RoutingAuthorityReferenceKind::RegistryLineage,
        )?;
        let mut catalog_heads = BTreeSet::new();
        let mut retained_attestations = BTreeSet::new();
        let mut retained_generations = BTreeSet::new();
        let mut namespaces = BTreeMap::new();
        let mut chain_issuances = BTreeMap::new();
        let mut route_memberships = BTreeMap::new();
        let mut generation_ids = BTreeMap::new();
        let mut route_chains = BTreeMap::new();

        for catalog in catalogs {
            catalog.validate()?;
            if !catalog_heads.insert(catalog.chain_registry_head_ref.clone()) {
                return Err(WalletAuthorityContractError::Invalid(
                    "chain_registry_head_reuse",
                ));
            }
            retain_authority_reference(
                &mut authority_references,
                catalog.chain_registry_head_ref(),
                RoutingAuthorityReferenceKind::RegistryHead,
            )?;

            let mut catalog_attestations = BTreeSet::new();
            for attestation in catalog.chain_instances() {
                if attestation.declaration().chain_registry_lineage_ref()
                    != chain_registry_lineage_ref
                    || !catalog_heads.contains(attestation.registry_head_ref_at_issuance())
                {
                    return Err(WalletAuthorityContractError::Invalid(
                        "chain_registry_history",
                    ));
                }
                let attestation_ref = attestation.content_ref()?;
                catalog_attestations.insert(attestation_ref.clone());
                retain_permanent_binding(
                    &mut namespaces,
                    attestation.declaration().instance_namespace_id().to_owned(),
                    attestation_ref.clone(),
                    "chain_instance_namespace_reuse",
                )?;
                retain_permanent_binding(
                    &mut chain_issuances,
                    attestation.registry_issuance_ref().clone(),
                    attestation_ref,
                    "chain_registry_issuance_reuse",
                )?;
                retain_authority_reference(
                    &mut authority_references,
                    attestation.registry_head_ref_at_issuance(),
                    RoutingAuthorityReferenceKind::RegistryHead,
                )?;
                retain_authority_reference(
                    &mut authority_references,
                    attestation.registry_issuance_ref(),
                    RoutingAuthorityReferenceKind::ChainIssuance,
                )?;
            }
            if !retained_attestations.is_subset(&catalog_attestations) {
                return Err(WalletAuthorityContractError::Invalid(
                    "chain_registry_history_omission",
                ));
            }
            retained_attestations = catalog_attestations;

            let mut catalog_generations = BTreeSet::new();
            for generation in catalog.generations() {
                let generation_ref = generation.generation_ref()?;
                catalog_generations.insert(generation_ref.clone());
                retain_permanent_binding(
                    &mut route_memberships,
                    generation.route_membership_issuance_ref().clone(),
                    generation_ref.clone(),
                    "route_membership_issuance_reuse",
                )?;
                retain_permanent_binding(
                    &mut generation_ids,
                    generation.generation_id().to_owned(),
                    generation_ref,
                    "route_generation_id_reuse",
                )?;
                retain_permanent_binding(
                    &mut route_chains,
                    (
                        generation.network_id().to_owned(),
                        generation.source_ref().to_owned(),
                    ),
                    generation.chain_instance().clone(),
                    "route_chain_reassignment",
                )?;
                retain_authority_reference(
                    &mut authority_references,
                    generation.route_membership_issuance_ref(),
                    RoutingAuthorityReferenceKind::RouteMembership,
                )?;
            }
            if !retained_generations.is_subset(&catalog_generations) {
                return Err(WalletAuthorityContractError::Invalid(
                    "routing_catalog_history_omission",
                ));
            }
            retained_generations = catalog_generations;
        }
        Ok(())
    }

    /// Returns the exact catalog version.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns the exact current chain-registry head.
    pub const fn chain_registry_head_ref(&self) -> &EvmWalletReference {
        &self.chain_registry_head_ref
    }

    /// Returns full attestation closure in canonical reference order.
    pub fn chain_instances(&self) -> &[ChainInstanceRegistryAttestation] {
        &self.chain_instances
    }

    /// Returns route descriptors in canonical reference order.
    pub fn generations(&self) -> &[EvmRoutingGenerationDescriptor] {
        &self.generations
    }

    /// Resolves one exact route generation.
    pub fn resolve_generation(
        &self,
        reference: &EvmRoutingGenerationRef,
    ) -> Option<&EvmRoutingGenerationDescriptor> {
        self.generations.iter().find(|generation| {
            generation
                .generation_ref()
                .is_ok_and(|candidate| &candidate == reference)
        })
    }

    /// Returns the catalog's exact canonical content reference.
    pub fn content_ref(&self) -> Result<ContentRef, WalletAuthorityContractError> {
        self.validate()?;
        canonical_wallet_reference(self)?
            .to_content_ref()
            .map_err(|_| WalletAuthorityContractError::Invalid("routing_catalog_ref"))
    }

    /// Returns exact canonical catalog bytes.
    pub fn canonical(&self) -> Result<PlainCanonicalJsonBytes, WalletAuthorityContractError> {
        self.validate()?;
        canonical_json(self)
    }
}

impl mfm_values::PersistedObjectPayload for EvmRoutingCatalogDescriptor {
    fn object_type() -> mfm_values::Result<mfm_ids::StableId> {
        mfm_ids::StableId::new("structured.admission_routing_policy")
            .map_err(|error| mfm_values::ValueError::Identity(error.to_string()))
    }
}

impl mfm_values::PersistedSchema for EvmRoutingCatalogDescriptor {
    fn schema_identity() -> mfm_values::Result<mfm_values::SchemaIdentity> {
        <Self as mfm_values::MfmValue>::schema_descriptor().map(|descriptor| descriptor.identity)
    }

    fn validate(&self) -> mfm_values::Result<()> {
        EvmRoutingCatalogDescriptor::validate(self)
            .map_err(|_| mfm_values::ValueError::SchemaShapeMismatch)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RoutingAuthorityReferenceKind {
    RegistryLineage,
    RegistryHead,
    ChainIssuance,
    RouteMembership,
}

fn retain_authority_reference(
    retained: &mut BTreeMap<EvmWalletReference, RoutingAuthorityReferenceKind>,
    reference: &EvmWalletReference,
    kind: RoutingAuthorityReferenceKind,
) -> Result<(), WalletAuthorityContractError> {
    if retained
        .get(reference)
        .is_some_and(|retained_kind| *retained_kind != kind)
    {
        return Err(WalletAuthorityContractError::Invalid(
            "routing_authority_reference_reuse",
        ));
    }
    retained.insert(reference.clone(), kind);
    Ok(())
}

fn retain_permanent_binding<K, V>(
    retained: &mut BTreeMap<K, V>,
    key: K,
    value: V,
    field: &'static str,
) -> Result<(), WalletAuthorityContractError>
where
    K: Ord,
    V: PartialEq,
{
    if retained
        .get(&key)
        .is_some_and(|retained_value| retained_value != &value)
    {
        return Err(WalletAuthorityContractError::Invalid(field));
    }
    retained.insert(key, value);
    Ok(())
}

fn canonical_json<T: Serialize>(
    value: &T,
) -> Result<PlainCanonicalJsonBytes, WalletAuthorityContractError> {
    let json = serde_json::to_string(value).map_err(|_| WalletAuthorityContractError::Canonical)?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|_| WalletAuthorityContractError::Canonical)
}

fn validate_reference(
    value: &EvmWalletReference,
    field: &'static str,
) -> Result<(), WalletAuthorityContractError> {
    value
        .to_content_ref()
        .map(|_| ())
        .map_err(|_| WalletAuthorityContractError::Invalid(field))
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, B256, U256};
    use mfm_canonical::sha256_digest_bytes;
    use mfm_ids::{ContentDigest, DigestAlgorithm, SchemaId, TenantScopeId};

    use super::*;
    use crate::wallet::evm_wallet_nonce_policy_ref;
    use crate::wallet_authority::{
        derive_authenticated_intent_issuer_id, derive_evm_chain_lineage_id,
        derive_evm_nonce_reservation_key, derive_submission_intent_id, derive_wallet_nonce_domain,
    };

    #[test]
    fn physical_chain_identity_distinguishes_forks_anchors_and_independent_clones() {
        let lineage = evm_wallet_nonce_policy_ref().expect("registry lineage");
        let base = declaration(&lineage, "mfm.test/instance-a", 0x11, 0x21);
        let different_genesis = declaration(&lineage, "mfm.test/instance-b", 0x12, 0x21);
        let different_anchor = declaration(&lineage, "mfm.test/instance-c", 0x11, 0x22);
        let independent_clone = declaration(&lineage, "mfm.test/instance-d", 0x11, 0x21);

        let ids = [
            derive_qualified_chain_instance_id(&base).expect("base identity"),
            derive_qualified_chain_instance_id(&different_genesis)
                .expect("different-genesis identity"),
            derive_qualified_chain_instance_id(&different_anchor)
                .expect("different-anchor identity"),
            derive_qualified_chain_instance_id(&independent_clone)
                .expect("independent-clone identity"),
        ];
        for left in 0..ids.len() {
            for right in (left + 1)..ids.len() {
                assert_ne!(ids[left], ids[right]);
            }
        }
    }

    #[test]
    fn true_fork_changes_wallet_domain_intent_and_reservation_keys() {
        let lineage = evm_wallet_nonce_policy_ref().expect("registry lineage");
        let base = attestation(
            declaration(&lineage, "mfm.test/domain-instance-a", 0x23, 0x24),
            21,
        );
        let fork = attestation(
            declaration(&lineage, "mfm.test/domain-instance-b", 0x23, 0x25),
            22,
        );
        let base_chain_lineage = derive_evm_chain_lineage_id(
            base.binding()
                .expect("base binding")
                .qualified_chain_instance_id(),
        )
        .expect("base chain lineage");
        let fork_chain_lineage = derive_evm_chain_lineage_id(
            fork.binding()
                .expect("fork binding")
                .qualified_chain_instance_id(),
        )
        .expect("fork chain lineage");
        let sender = Address::repeat_byte(0x26);
        let base_domain =
            derive_wallet_nonce_domain(base_chain_lineage, sender).expect("base wallet domain");
        let fork_domain =
            derive_wallet_nonce_domain(fork_chain_lineage, sender).expect("fork wallet domain");
        assert_ne!(base_domain, fork_domain);

        let tenant = TenantScopeId::new("mfm.tenant_scope.v1:00000000000000000000000000000027")
            .expect("tenant");
        let issuer = derive_authenticated_intent_issuer_id(
            &tenant,
            &StableId::new("mfm.test/chain-fork-principal").expect("principal"),
            &reference(23),
        )
        .expect("authenticated issuer");
        let base_intent =
            derive_submission_intent_id(&base_domain, &issuer, "same-token").expect("base intent");
        let fork_intent =
            derive_submission_intent_id(&fork_domain, &issuer, "same-token").expect("fork intent");
        assert_ne!(base_intent, fork_intent);
        assert_ne!(
            derive_evm_nonce_reservation_key(&base_domain, &base_intent)
                .expect("base reservation key"),
            derive_evm_nonce_reservation_key(&fork_domain, &fork_intent)
                .expect("fork reservation key")
        );
    }

    #[test]
    fn redundant_routes_converge_on_one_exact_chain_binding() {
        let lineage = evm_wallet_nonce_policy_ref().expect("registry lineage");
        let attestation = attestation(
            declaration(&lineage, "mfm.test/redundant-instance", 0x31, 0x32),
            1,
        );
        let binding = attestation.binding().expect("chain binding");
        let first = route(
            "redundant-route-a",
            "mfm.test/redundant-generation-a",
            reference(2),
            binding.clone(),
        );
        let second = route(
            "redundant-route-b",
            "mfm.test/redundant-generation-b",
            reference(3),
            binding.clone(),
        );
        let catalog =
            EvmRoutingCatalogDescriptor::new(reference(4), vec![attestation], vec![second, first])
                .expect("redundant-route catalog");
        assert_eq!(catalog.generations().len(), 2);
        assert!(catalog
            .generations()
            .iter()
            .all(|generation| generation.chain_instance() == &binding));
    }

    #[test]
    fn catalog_rejects_one_route_generation_assigned_to_two_instances() {
        let lineage = evm_wallet_nonce_policy_ref().expect("registry lineage");
        let first = attestation(
            declaration(&lineage, "mfm.test/route-owner-a", 0x41, 0x42),
            5,
        );
        let second = attestation(
            declaration(&lineage, "mfm.test/route-owner-b", 0x41, 0x42),
            6,
        );
        let first_route = route(
            "shared-route",
            "mfm.test/shared-generation",
            reference(7),
            first.binding().expect("first binding"),
        );
        let second_route = route(
            "shared-route",
            "mfm.test/shared-generation",
            reference(8),
            second.binding().expect("second binding"),
        );
        assert!(EvmRoutingCatalogDescriptor::new(
            reference(9),
            vec![first, second],
            vec![first_route, second_route],
        )
        .is_err());
    }

    #[test]
    fn catalog_rejects_namespace_reuse_and_foreign_route_attestations() {
        let lineage = evm_wallet_nonce_policy_ref().expect("registry lineage");
        let first = attestation(
            declaration(&lineage, "mfm.test/reused-namespace", 0x51, 0x52),
            10,
        );
        let reused = attestation(
            declaration(&lineage, "mfm.test/reused-namespace", 0x53, 0x52),
            11,
        );
        assert!(EvmRoutingCatalogDescriptor::new(
            reference(12),
            vec![first.clone(), reused.clone()],
            vec![
                route(
                    "namespace-route-a",
                    "mfm.test/namespace-generation-a",
                    reference(13),
                    first.binding().expect("first binding"),
                ),
                route(
                    "namespace-route-b",
                    "mfm.test/namespace-generation-b",
                    reference(14),
                    reused.binding().expect("reused binding"),
                ),
            ],
        )
        .is_err());

        assert!(EvmRoutingCatalogDescriptor::new(
            reference(15),
            vec![first],
            vec![route(
                "foreign-route",
                "mfm.test/foreign-generation",
                reference(16),
                reused.binding().expect("foreign binding"),
            )],
        )
        .is_err());
    }

    #[test]
    fn catalog_rejects_chain_issuance_reuse_and_cross_category_authority_refs() {
        let lineage = evm_wallet_nonce_policy_ref().expect("registry lineage");
        let head = reference(0x61);
        let shared_issuance = reference(0x62);
        let first = ChainInstanceRegistryAttestation::new(
            declaration(&lineage, "mfm.test/issuance-owner-a", 0x63, 0x64),
            shared_issuance.clone(),
            head.clone(),
        )
        .expect("first chain issuance");
        let second = ChainInstanceRegistryAttestation::new(
            declaration(&lineage, "mfm.test/issuance-owner-b", 0x65, 0x66),
            shared_issuance.clone(),
            head.clone(),
        )
        .expect("second chain issuance");
        assert!(EvmRoutingCatalogDescriptor::new(
            head.clone(),
            vec![first.clone(), second.clone()],
            vec![
                route(
                    "issuance-route-a",
                    "mfm.test/issuance-generation-a",
                    reference(0x67),
                    first.binding().expect("first binding"),
                ),
                route(
                    "issuance-route-b",
                    "mfm.test/issuance-generation-b",
                    reference(0x68),
                    second.binding().expect("second binding"),
                ),
            ],
        )
        .is_err());

        let head_as_issuance = ChainInstanceRegistryAttestation::new(
            declaration(&lineage, "mfm.test/head-as-issuance", 0x69, 0x6a),
            head.clone(),
            head.clone(),
        )
        .expect("structural head-as-issuance attestation");
        assert!(EvmRoutingCatalogDescriptor::new(
            head.clone(),
            vec![head_as_issuance.clone()],
            vec![route(
                "head-as-issuance-route",
                "mfm.test/head-as-issuance-generation",
                reference(0x6b),
                head_as_issuance
                    .binding()
                    .expect("head-as-issuance binding"),
            )],
        )
        .is_err());

        assert!(EvmRoutingCatalogDescriptor::new(
            head,
            vec![first.clone()],
            vec![route(
                "issuance-as-membership-route",
                "mfm.test/issuance-as-membership-generation",
                shared_issuance,
                first.binding().expect("issuance-as-membership binding"),
            )],
        )
        .is_err());
    }

    #[test]
    fn append_only_catalog_history_preserves_every_permanent_binding() {
        let lineage = evm_wallet_nonce_policy_ref().expect("registry lineage");
        let first_head = reference(0x70);
        let current_head = reference(0x71);
        let chain_issuance = reference(0x72);
        let first_membership = reference(0x73);
        let refreshed_membership = reference(0x74);
        let chain = ChainInstanceRegistryAttestation::new(
            declaration(&lineage, "mfm.test/history-instance", 0x75, 0x76),
            chain_issuance.clone(),
            first_head.clone(),
        )
        .expect("history chain attestation");
        let binding = chain.binding().expect("history chain binding");
        let first_route = route(
            "history-route-a",
            "mfm.test/history-generation-a",
            first_membership.clone(),
            binding.clone(),
        );
        let first_catalog = EvmRoutingCatalogDescriptor::new(
            first_head.clone(),
            vec![chain.clone()],
            vec![first_route.clone()],
        )
        .expect("first routing catalog");
        let refreshed_route = route(
            "history-route-b",
            "mfm.test/history-generation-b",
            refreshed_membership,
            binding,
        );
        let refreshed_catalog = EvmRoutingCatalogDescriptor::new(
            current_head.clone(),
            vec![chain.clone()],
            vec![first_route.clone(), refreshed_route],
        )
        .expect("refreshed routing catalog");
        EvmRoutingCatalogDescriptor::validate_append_only_history(
            &lineage,
            &current_head,
            &[first_catalog.clone(), refreshed_catalog.clone()],
        )
        .expect("append-only routing refresh");
        assert!(EvmRoutingCatalogDescriptor::validate_append_only_history(
            &lineage,
            &first_head,
            &[first_catalog.clone(), refreshed_catalog],
        )
        .is_err());

        let hostile_namespace = ChainInstanceRegistryAttestation::new(
            declaration(&lineage, "mfm.test/history-instance", 0x75, 0x77),
            reference(0x78),
            current_head.clone(),
        )
        .expect("hostile namespace attestation");
        let hostile_issuance = ChainInstanceRegistryAttestation::new(
            declaration(&lineage, "mfm.test/history-issuance-reuse", 0x79, 0x7a),
            chain_issuance,
            current_head.clone(),
        )
        .expect("hostile issuance attestation");
        let remapped_chain = ChainInstanceRegistryAttestation::new(
            declaration(&lineage, "mfm.test/history-remapped-instance", 0x7b, 0x7c),
            reference(0x7d),
            current_head.clone(),
        )
        .expect("remapped chain attestation");
        let cross_category_chain = ChainInstanceRegistryAttestation::new(
            declaration(&lineage, "mfm.test/history-cross-category", 0x7e, 0x7f),
            first_membership.clone(),
            current_head.clone(),
        )
        .expect("cross-category chain attestation");

        let hostile_catalogs = [
            EvmRoutingCatalogDescriptor::new(
                current_head.clone(),
                vec![hostile_namespace.clone()],
                vec![route(
                    "hostile-namespace-route",
                    "mfm.test/hostile-namespace-generation",
                    reference(0x80),
                    hostile_namespace
                        .binding()
                        .expect("hostile namespace binding"),
                )],
            )
            .expect("hostile namespace catalog"),
            EvmRoutingCatalogDescriptor::new(
                current_head.clone(),
                vec![hostile_issuance.clone()],
                vec![route(
                    "hostile-issuance-route",
                    "mfm.test/hostile-issuance-generation",
                    reference(0x81),
                    hostile_issuance
                        .binding()
                        .expect("hostile issuance binding"),
                )],
            )
            .expect("hostile issuance catalog"),
            EvmRoutingCatalogDescriptor::new(
                current_head.clone(),
                vec![chain.clone()],
                vec![route(
                    "hostile-membership-route",
                    "mfm.test/hostile-membership-generation",
                    first_membership.clone(),
                    chain.binding().expect("hostile membership binding"),
                )],
            )
            .expect("hostile membership catalog"),
            EvmRoutingCatalogDescriptor::new(
                current_head.clone(),
                vec![chain.clone()],
                vec![route(
                    "hostile-generation-route",
                    "mfm.test/history-generation-a",
                    reference(0x82),
                    chain.binding().expect("hostile generation binding"),
                )],
            )
            .expect("hostile generation catalog"),
            EvmRoutingCatalogDescriptor::new(
                current_head.clone(),
                vec![remapped_chain.clone()],
                vec![route(
                    "history-route-a",
                    "mfm.test/history-remapped-generation",
                    reference(0x83),
                    remapped_chain.binding().expect("remapped route binding"),
                )],
            )
            .expect("hostile route mapping catalog"),
            EvmRoutingCatalogDescriptor::new(
                current_head.clone(),
                vec![cross_category_chain.clone()],
                vec![route(
                    "cross-category-route",
                    "mfm.test/cross-category-generation",
                    reference(0x84),
                    cross_category_chain
                        .binding()
                        .expect("cross-category binding"),
                )],
            )
            .expect("hostile cross-category catalog"),
        ];
        for hostile in hostile_catalogs {
            assert!(EvmRoutingCatalogDescriptor::validate_append_only_history(
                &lineage,
                &current_head,
                &[first_catalog.clone(), hostile],
            )
            .is_err());
        }
    }

    fn declaration(
        lineage: &EvmWalletReference,
        namespace: &str,
        genesis: u8,
        anchor: u8,
    ) -> ChainInstanceDeclaration {
        ChainInstanceDeclaration::new(
            lineage.clone(),
            StableId::new(namespace).expect("instance namespace"),
            1,
            B256::repeat_byte(genesis),
            U256::from(100_u64),
            B256::repeat_byte(anchor),
        )
        .expect("chain declaration")
    }

    fn attestation(
        declaration: ChainInstanceDeclaration,
        discriminator: u8,
    ) -> ChainInstanceRegistryAttestation {
        ChainInstanceRegistryAttestation::new(
            declaration,
            reference(discriminator),
            reference(discriminator.wrapping_add(64)),
        )
        .expect("chain attestation")
    }

    fn route(
        source: &str,
        generation: &str,
        membership: EvmWalletReference,
        binding: EvmChainInstanceBinding,
    ) -> EvmRoutingGenerationDescriptor {
        EvmRoutingGenerationDescriptor::new(
            "test-network",
            source,
            StableId::new(generation).expect("route generation"),
            membership,
            binding,
        )
        .expect("route descriptor")
    }

    fn reference(discriminator: u8) -> EvmWalletReference {
        let schema_id = SchemaId::new(
            "mfm.test.chain-registry-reference",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.test.chain-registry-reference.v1"),
        )
        .expect("reference schema");
        EvmWalletReference::from_content_ref(
            ContentRef::new(
                schema_id,
                ContentDigest::from_digest(
                    DigestAlgorithm::Sha256V1,
                    sha256_digest_bytes(&[discriminator]),
                ),
            )
            .expect("reference content identity"),
        )
    }
}
