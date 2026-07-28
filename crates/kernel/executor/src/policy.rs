use std::collections::{BTreeMap, BTreeSet};

use mfm_canonical::{CanonicalValue, ValidatedCanonicalValueV1};
use mfm_ids::{ContentRef, EffectKey};

use crate::contract::{canonical_object, content_ref, content_ref_value, encode};
use crate::{AllocationStateRef, EffectIdentity, FencingRef, ResourceKeyRef, ResourceStreamView};

const ACCOUNT_RESOURCE_KEY_SCHEMA: &str = "mfm.executor-account-resource-key.v1";
const ACCOUNT_ALLOCATION_SCHEMA: &str = "mfm.executor-account-sequence-allocation.v1";
const INVENTORY_RESOURCE_KEY_SCHEMA: &str = "mfm.executor-inventory-resource-key.v1";
const INVENTORY_ALLOCATION_SCHEMA: &str = "mfm.executor-finite-inventory-allocation.v1";

/// Closed failure taxonomy for typed resource allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    /// A fixed policy schema or canonical value was rejected.
    #[error("typed policy contract is invalid")]
    InvalidContract,
    /// Retained resource history is not valid for this policy.
    #[error("typed resource history is invalid")]
    InvalidHistory,
    /// A monotonic sequence cannot advance without overflow.
    #[error("account sequence is exhausted")]
    SequenceExhausted,
    /// The preceding permanent sequence allocation is not terminal.
    #[error("preceding account sequence effect is still pending")]
    PriorSequencePending,
    /// Every admitted unit of an inventory item is permanently allocated.
    #[error("finite inventory is exhausted")]
    InventoryExhausted,
    /// A finite inventory descriptor repeats an item or admits zero units.
    #[error("finite inventory configuration is invalid")]
    InvalidInventory,
    /// A request names an item outside the selected policy configuration.
    #[error("finite inventory item is not configured")]
    UnknownInventoryItem,
}

/// Immutable selected policy and configuration content identities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourcePolicyBinding {
    policy_ref: ContentRef,
    policy_configuration_ref: ContentRef,
}

impl ResourcePolicyBinding {
    /// Constructs an exact policy/configuration pair.
    pub const fn new(policy_ref: ContentRef, policy_configuration_ref: ContentRef) -> Self {
        Self {
            policy_ref,
            policy_configuration_ref,
        }
    }

    /// Returns the selected policy identity.
    pub const fn policy_ref(&self) -> &ContentRef {
        &self.policy_ref
    }

    /// Returns the selected immutable policy configuration.
    pub const fn policy_configuration_ref(&self) -> &ContentRef {
        &self.policy_configuration_ref
    }
}

/// Pure decision returned by one typed resource policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyDecision<Allocation> {
    pub(crate) allocation: Allocation,
    pub(crate) allocation_state: ValidatedCanonicalValueV1,
    pub(crate) fencing_ref: Option<FencingRef>,
}

impl<Allocation> PolicyDecision<Allocation> {
    /// Constructs one pure typed allocation decision and its reviewed state.
    pub fn new(
        allocation: Allocation,
        allocation_state: ValidatedCanonicalValueV1,
        fencing_ref: Option<FencingRef>,
    ) -> Self {
        Self {
            allocation,
            allocation_state,
            fencing_ref,
        }
    }

    /// Returns the typed policy allocation.
    pub const fn allocation(&self) -> &Allocation {
        &self.allocation
    }

    /// Returns the exact annex-validated allocation state.
    pub const fn allocation_state(&self) -> &ValidatedCanonicalValueV1 {
        &self.allocation_state
    }

    /// Returns optional authoritative fencing evidence.
    pub const fn fencing_ref(&self) -> Option<&FencingRef> {
        self.fencing_ref.as_ref()
    }
}

/// Domain-owned allocation semantics over the shared immutable append/CAS substrate.
pub trait TypedResourcePolicy: Send + Sync {
    /// Pure semantic allocation request.
    type Request;
    /// Typed allocation returned to the executor implementation.
    type Allocation;

    /// Returns the exact selected policy and configuration identities.
    fn binding(&self) -> &ResourcePolicyBinding;

    /// Produces the exact annex-validated resource-key object.
    fn resource_key_value(
        &self,
        request: &Self::Request,
    ) -> std::result::Result<ValidatedCanonicalValueV1, PolicyError>;

    /// Derives the exact resource stream key for this request.
    fn resource_key(
        &self,
        request: &Self::Request,
    ) -> std::result::Result<ResourceKeyRef, PolicyError> {
        Ok(ResourceKeyRef::from_reviewed(
            content_ref(&self.resource_key_value(request)?)
                .map_err(|_| PolicyError::InvalidContract)?,
        ))
    }

    /// Proposes the next permanent allocation from immutable resource history.
    fn allocate(
        &self,
        identity: &EffectIdentity,
        request: &Self::Request,
        history: ResourceStreamView<'_>,
    ) -> std::result::Result<PolicyDecision<Self::Allocation>, PolicyError>;

    /// Reconstructs the typed allocation for an already-bound effect.
    fn restore_allocation(
        &self,
        identity: &EffectIdentity,
        request: &Self::Request,
        record: &crate::ResourceLedgerRecord,
    ) -> std::result::Result<Self::Allocation, PolicyError>;

    /// Revalidates complete restored history under this exact policy pair.
    fn validate_history(
        &self,
        history: ResourceStreamView<'_>,
    ) -> std::result::Result<(), PolicyError>;
}

/// Request to allocate the next permanent sequence for one account/domain pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountSequenceRequest {
    account: String,
    resource_domain: String,
}

impl AccountSequenceRequest {
    /// Constructs an account-sequence allocation request.
    pub fn new(
        account: impl Into<String>,
        resource_domain: impl Into<String>,
    ) -> std::result::Result<Self, PolicyError> {
        let request = Self {
            account: account.into(),
            resource_domain: resource_domain.into(),
        };
        account_resource_key_value(&request)?;
        Ok(request)
    }

    /// Returns the account whose sequence is coordinated.
    pub fn account(&self) -> &str {
        &self.account
    }

    /// Returns the external resource domain.
    pub fn resource_domain(&self) -> &str {
        &self.resource_domain
    }
}

/// Permanent sequence allocation for one account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountSequenceAllocation {
    account: String,
    sequence: u64,
}

impl AccountSequenceAllocation {
    /// Returns the coordinated account.
    pub fn account(&self) -> &str {
        &self.account
    }

    /// Returns the permanently allocated sequence.
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }
}

/// Monotonic account-sequence policy.
#[derive(Debug, Clone)]
pub struct AccountSequencePolicy {
    binding: ResourcePolicyBinding,
    initial_sequence: u64,
    fencing_ref: Option<FencingRef>,
}

impl AccountSequencePolicy {
    /// Constructs a monotonic sequence policy.
    pub const fn new(
        binding: ResourcePolicyBinding,
        initial_sequence: u64,
        fencing_ref: Option<FencingRef>,
    ) -> Self {
        Self {
            binding,
            initial_sequence,
            fencing_ref,
        }
    }

    fn allocation_state(
        &self,
        account: &str,
        sequence: u64,
        effect_key: &EffectKey,
    ) -> std::result::Result<ValidatedCanonicalValueV1, PolicyError> {
        let mut fields = vec![
            (
                "account".to_owned(),
                CanonicalValue::String(account.to_owned()),
            ),
            (
                "effect_key".to_owned(),
                CanonicalValue::String(effect_key.as_str().to_owned()),
            ),
            (
                "sequence".to_owned(),
                CanonicalValue::String(sequence.to_string()),
            ),
            (
                "version".to_owned(),
                CanonicalValue::String(ACCOUNT_ALLOCATION_SCHEMA.to_owned()),
            ),
        ];
        if let Some(fencing_ref) = &self.fencing_ref {
            fields.push((
                "fencing_ref".to_owned(),
                content_ref_value(fencing_ref.as_content_ref())
                    .map_err(|_| PolicyError::InvalidContract)?,
            ));
        }
        let value = CanonicalValue::object(fields).map_err(|_| PolicyError::InvalidContract)?;
        encode(ACCOUNT_ALLOCATION_SCHEMA, &value).map_err(|_| PolicyError::InvalidContract)
    }
}

impl TypedResourcePolicy for AccountSequencePolicy {
    type Request = AccountSequenceRequest;
    type Allocation = AccountSequenceAllocation;

    fn binding(&self) -> &ResourcePolicyBinding {
        &self.binding
    }

    fn resource_key_value(
        &self,
        request: &Self::Request,
    ) -> std::result::Result<ValidatedCanonicalValueV1, PolicyError> {
        account_resource_key_value(request)
    }

    fn allocate(
        &self,
        identity: &EffectIdentity,
        request: &Self::Request,
        history: ResourceStreamView<'_>,
    ) -> std::result::Result<PolicyDecision<Self::Allocation>, PolicyError> {
        self.validate_history(history)?;
        if history
            .records()
            .last()
            .is_some_and(|record| !history.effect_is_terminal(record.effect_key()))
        {
            return Err(PolicyError::PriorSequencePending);
        }
        let offset =
            u64::try_from(history.records().len()).map_err(|_| PolicyError::SequenceExhausted)?;
        let sequence = self
            .initial_sequence
            .checked_add(offset)
            .ok_or(PolicyError::SequenceExhausted)?;
        Ok(PolicyDecision {
            allocation: AccountSequenceAllocation {
                account: request.account.clone(),
                sequence,
            },
            allocation_state: self.allocation_state(
                &request.account,
                sequence,
                identity.effect_key(),
            )?,
            fencing_ref: self.fencing_ref.clone(),
        })
    }

    fn validate_history(
        &self,
        history: ResourceStreamView<'_>,
    ) -> std::result::Result<(), PolicyError> {
        for record in history
            .records()
            .iter()
            .take(history.records().len().saturating_sub(1))
        {
            if !history.effect_is_terminal(record.effect_key()) {
                return Err(PolicyError::InvalidHistory);
            }
        }
        for (index, record) in history.records().iter().enumerate() {
            if record.policy_binding() != &self.binding
                || record.resource_key_value().schema_contract() != ACCOUNT_RESOURCE_KEY_SCHEMA
            {
                return Err(PolicyError::InvalidHistory);
            }
            let account = json_string_field(record.resource_key_value(), "account")?;
            let offset = u64::try_from(index).map_err(|_| PolicyError::SequenceExhausted)?;
            let sequence = self
                .initial_sequence
                .checked_add(offset)
                .ok_or(PolicyError::SequenceExhausted)?;
            let expected = self.allocation_state(&account, sequence, record.effect_key())?;
            if record.typed_allocation_state_ref()
                != &AllocationStateRef::from_reviewed(
                    content_ref(&expected).map_err(|_| PolicyError::InvalidContract)?,
                )
                || record.fencing_ref() != self.fencing_ref.as_ref()
            {
                return Err(PolicyError::InvalidHistory);
            }
        }
        Ok(())
    }

    fn restore_allocation(
        &self,
        identity: &EffectIdentity,
        request: &Self::Request,
        record: &crate::ResourceLedgerRecord,
    ) -> std::result::Result<Self::Allocation, PolicyError> {
        if record.effect_key() != identity.effect_key()
            || record.resource_key_ref() != &self.resource_key(request)?
            || record.policy_binding() != &self.binding
            || record.resource_key_value().schema_contract() != ACCOUNT_RESOURCE_KEY_SCHEMA
            || record.typed_allocation_state().schema_contract() != ACCOUNT_ALLOCATION_SCHEMA
        {
            return Err(PolicyError::InvalidHistory);
        }
        let account = json_string_field(record.typed_allocation_state(), "account")?;
        let sequence = json_decimal_u64_field(record.typed_allocation_state(), "sequence")?;
        if account != request.account {
            return Err(PolicyError::InvalidHistory);
        }
        Ok(AccountSequenceAllocation { account, sequence })
    }
}

fn account_resource_key_value(
    request: &AccountSequenceRequest,
) -> std::result::Result<ValidatedCanonicalValueV1, PolicyError> {
    encode(
        ACCOUNT_RESOURCE_KEY_SCHEMA,
        &canonical_object([
            ("account", CanonicalValue::String(request.account.clone())),
            (
                "resource_domain",
                CanonicalValue::String(request.resource_domain.clone()),
            ),
            (
                "version",
                CanonicalValue::String(ACCOUNT_RESOURCE_KEY_SCHEMA.to_owned()),
            ),
        ])
        .map_err(|_| PolicyError::InvalidContract)?,
    )
    .map_err(|_| PolicyError::InvalidContract)
}

/// Request to consume one unit of a configured finite inventory item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FiniteInventoryRequest {
    inventory_namespace: String,
    item: String,
}

impl FiniteInventoryRequest {
    /// Constructs a finite-inventory request.
    pub fn new(
        inventory_namespace: impl Into<String>,
        item: impl Into<String>,
    ) -> std::result::Result<Self, PolicyError> {
        let request = Self {
            inventory_namespace: inventory_namespace.into(),
            item: item.into(),
        };
        inventory_resource_key_value(&request)?;
        Ok(request)
    }

    /// Returns the inventory namespace.
    pub fn inventory_namespace(&self) -> &str {
        &self.inventory_namespace
    }

    /// Returns the requested item.
    pub fn item(&self) -> &str {
        &self.item
    }
}

/// Permanent allocation of one finite inventory unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FiniteInventoryAllocation {
    item: String,
    quantity: u64,
}

impl FiniteInventoryAllocation {
    /// Returns the permanently consumed item.
    pub fn item(&self) -> &str {
        &self.item
    }

    /// Returns the permanently consumed quantity.
    pub const fn quantity(&self) -> u64 {
        self.quantity
    }
}

/// Deterministic finite-inventory allocation policy.
#[derive(Debug, Clone)]
pub struct FiniteInventoryPolicy {
    binding: ResourcePolicyBinding,
    inventory_namespace: String,
    capacities: BTreeMap<String, u64>,
    destination_precondition_ref: ContentRef,
}

impl FiniteInventoryPolicy {
    /// Constructs a duplicate-free finite inventory configuration.
    pub fn new(
        binding: ResourcePolicyBinding,
        inventory_namespace: impl Into<String>,
        capacities: impl IntoIterator<Item = (String, u64)>,
        destination_precondition_ref: ContentRef,
    ) -> std::result::Result<Self, PolicyError> {
        let inventory_namespace = inventory_namespace.into();
        let mut map = BTreeMap::new();
        let mut seen = BTreeSet::new();
        for (item, capacity) in capacities {
            if capacity == 0 || !seen.insert(item.clone()) || map.insert(item, capacity).is_some() {
                return Err(PolicyError::InvalidInventory);
            }
        }
        if map.is_empty() {
            return Err(PolicyError::InvalidInventory);
        }
        for item in map.keys() {
            inventory_resource_key_value(&FiniteInventoryRequest {
                inventory_namespace: inventory_namespace.clone(),
                item: item.clone(),
            })?;
        }
        Ok(Self {
            binding,
            inventory_namespace,
            capacities: map,
            destination_precondition_ref,
        })
    }

    /// Returns the ordered configured item capacities.
    pub const fn capacities(&self) -> &BTreeMap<String, u64> {
        &self.capacities
    }

    fn allocation_state(
        &self,
        item: &str,
        effect_key: &EffectKey,
    ) -> std::result::Result<ValidatedCanonicalValueV1, PolicyError> {
        encode(
            INVENTORY_ALLOCATION_SCHEMA,
            &canonical_object([
                (
                    "destination_precondition_ref",
                    content_ref_value(&self.destination_precondition_ref)
                        .map_err(|_| PolicyError::InvalidContract)?,
                ),
                (
                    "effect_key",
                    CanonicalValue::String(effect_key.as_str().to_owned()),
                ),
                ("item", CanonicalValue::String(item.to_owned())),
                ("quantity", CanonicalValue::String("1".to_owned())),
                (
                    "version",
                    CanonicalValue::String(INVENTORY_ALLOCATION_SCHEMA.to_owned()),
                ),
            ])
            .map_err(|_| PolicyError::InvalidContract)?,
        )
        .map_err(|_| PolicyError::InvalidContract)
    }
}

impl TypedResourcePolicy for FiniteInventoryPolicy {
    type Request = FiniteInventoryRequest;
    type Allocation = FiniteInventoryAllocation;

    fn binding(&self) -> &ResourcePolicyBinding {
        &self.binding
    }

    fn resource_key_value(
        &self,
        request: &Self::Request,
    ) -> std::result::Result<ValidatedCanonicalValueV1, PolicyError> {
        if request.inventory_namespace != self.inventory_namespace
            || !self.capacities.contains_key(&request.item)
        {
            return Err(PolicyError::UnknownInventoryItem);
        }
        inventory_resource_key_value(request)
    }

    fn allocate(
        &self,
        identity: &EffectIdentity,
        request: &Self::Request,
        history: ResourceStreamView<'_>,
    ) -> std::result::Result<PolicyDecision<Self::Allocation>, PolicyError> {
        self.validate_history(history)?;
        let capacity = self
            .capacities
            .get(&request.item)
            .copied()
            .ok_or(PolicyError::UnknownInventoryItem)?;
        let used =
            u64::try_from(history.records().len()).map_err(|_| PolicyError::InventoryExhausted)?;
        if used >= capacity {
            return Err(PolicyError::InventoryExhausted);
        }
        Ok(PolicyDecision {
            allocation: FiniteInventoryAllocation {
                item: request.item.clone(),
                quantity: 1,
            },
            allocation_state: self.allocation_state(&request.item, identity.effect_key())?,
            fencing_ref: None,
        })
    }

    fn validate_history(
        &self,
        history: ResourceStreamView<'_>,
    ) -> std::result::Result<(), PolicyError> {
        let mut counts = BTreeMap::<String, u64>::new();
        for record in history.records() {
            if record.policy_binding() != &self.binding
                || record.resource_key_value().schema_contract() != INVENTORY_RESOURCE_KEY_SCHEMA
                || record.fencing_ref().is_some()
            {
                return Err(PolicyError::InvalidHistory);
            }
            let item = json_string_field(record.resource_key_value(), "item")?;
            let count = counts.entry(item.clone()).or_default();
            *count = count.checked_add(1).ok_or(PolicyError::InvalidHistory)?;
            let capacity = self
                .capacities
                .get(&item)
                .copied()
                .ok_or(PolicyError::InvalidHistory)?;
            let expected = self.allocation_state(&item, record.effect_key())?;
            if *count > capacity
                || record.typed_allocation_state_ref()
                    != &AllocationStateRef::from_reviewed(
                        content_ref(&expected).map_err(|_| PolicyError::InvalidContract)?,
                    )
            {
                return Err(PolicyError::InvalidHistory);
            }
        }
        Ok(())
    }

    fn restore_allocation(
        &self,
        identity: &EffectIdentity,
        request: &Self::Request,
        record: &crate::ResourceLedgerRecord,
    ) -> std::result::Result<Self::Allocation, PolicyError> {
        if record.effect_key() != identity.effect_key()
            || record.resource_key_ref() != &self.resource_key(request)?
            || record.policy_binding() != &self.binding
            || record.resource_key_value().schema_contract() != INVENTORY_RESOURCE_KEY_SCHEMA
            || record.typed_allocation_state().schema_contract() != INVENTORY_ALLOCATION_SCHEMA
        {
            return Err(PolicyError::InvalidHistory);
        }
        let item = json_string_field(record.typed_allocation_state(), "item")?;
        let quantity = json_decimal_u64_field(record.typed_allocation_state(), "quantity")?;
        if item != request.item || quantity != 1 {
            return Err(PolicyError::InvalidHistory);
        }
        Ok(FiniteInventoryAllocation { item, quantity })
    }
}

fn inventory_resource_key_value(
    request: &FiniteInventoryRequest,
) -> std::result::Result<ValidatedCanonicalValueV1, PolicyError> {
    encode(
        INVENTORY_RESOURCE_KEY_SCHEMA,
        &canonical_object([
            (
                "inventory_namespace",
                CanonicalValue::String(request.inventory_namespace.clone()),
            ),
            ("item", CanonicalValue::String(request.item.clone())),
            (
                "version",
                CanonicalValue::String(INVENTORY_RESOURCE_KEY_SCHEMA.to_owned()),
            ),
        ])
        .map_err(|_| PolicyError::InvalidContract)?,
    )
    .map_err(|_| PolicyError::InvalidContract)
}

fn json_string_field(
    value: &ValidatedCanonicalValueV1,
    field: &str,
) -> std::result::Result<String, PolicyError> {
    let json: serde_json::Value =
        serde_json::from_slice(value.as_bytes()).map_err(|_| PolicyError::InvalidHistory)?;
    json.get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or(PolicyError::InvalidHistory)
}

fn json_decimal_u64_field(
    value: &ValidatedCanonicalValueV1,
    field: &str,
) -> std::result::Result<u64, PolicyError> {
    json_string_field(value, field)?
        .parse()
        .map_err(|_| PolicyError::InvalidHistory)
}
