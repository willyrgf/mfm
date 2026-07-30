use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;

use mfm_canonical::{
    CanonicalValue, PlainCanonicalJsonBytes, RecoverabilityContractV3, RecoverabilityError,
    ValidatedCanonicalValueV3,
};
use mfm_ids::{
    ContentRef, EffectKey, NodeId, RequestDigest, RunId, SchemaId, SemanticDigest, StoreScopeId,
    TenantScopeId,
};
use mfm_values::{RetainedValueContract, ValueError};

use crate::frontier::{
    EvidenceBounds, DELIVERY_FRONTIER_SCHEMA, TERMINAL_PROOF_SCHEMA, TERMINAL_TOMBSTONE_SCHEMA,
};
use crate::{ExecutorError, Result};

pub(crate) const EXECUTOR_BINDING_SCHEMA: &str = "mfm.executor-binding.v1";
pub(crate) const EXECUTOR_CONTRACT_DESCRIPTOR_SCHEMA: &str = "mfm.executor-contract-descriptor.v1";
pub(crate) const EXECUTOR_DEPLOYMENT_SCHEMA: &str = "mfm.executor-deployment.v1";
pub(crate) const RESOURCE_OWNERSHIP_SCHEMA: &str = "mfm.resource-ownership.v1";
const EXECUTOR_RETAINED_CLOSURE_CONTRACT_SCHEMA: &str = "mfm.executor-retained-closure-contract.v1";
const EXECUTOR_ENSURE_RESULT_SCHEMA: &str = "mfm.executor-ensure-result.v1";
const SAFE_FAILURE_VALUE_SCHEMA: &str = "mfm.safe-failure.v1";
const TERMINAL_EFFECT_EVIDENCE_SCHEMA: &str = "mfm.terminal-effect-evidence.v1";
const REQUIRED_PLAN_EXPANSION_SCHEMA: &str = "mfm.required-plan-expansion.v1";
const REQUEST_PREIMAGE_SCHEMA: &str = "mfm.request-digest-preimage.v1";
const EFFECT_KEY_PREIMAGE_SCHEMA: &str = "mfm.effect-key-preimage.v1";
const EFFECT_KEY_DOMAIN: &str = "mfm.effect-key.v1";
const STABLE_ID_SCHEMA: &str = "mfm.primitive-stable_id.v1";
const MAX_REFERENCE_EFFECT_IDENTIFIER_BYTES: usize = 256;
const MAX_SCHEMA_QUALIFIED_VALUE_BYTES: usize = 16 * 1024 * 1024;

pub(crate) fn recoverability_contract() -> Result<&'static RecoverabilityContractV3> {
    RecoverabilityContractV3::embedded().map_err(contract_error)
}

pub(crate) fn contract_error(error: RecoverabilityError) -> ExecutorError {
    ExecutorError::Recoverability(error.code())
}

pub(crate) fn encode(
    schema_contract: &str,
    value: &CanonicalValue,
) -> Result<ValidatedCanonicalValueV3> {
    recoverability_contract()?
        .encode(schema_contract, value)
        .map_err(contract_error)
}

pub(crate) fn content_ref(value: &ValidatedCanonicalValueV3) -> Result<ContentRef> {
    recoverability_contract()?
        .content_ref(value)
        .map_err(contract_error)
}

/// Exact float-free canonical JSON paired with its externally admitted schema.
///
/// Unlike [`ValidatedCanonicalValueV3`], this type is not restricted to the
/// frozen recoverability annex. The schema identity must already have been
/// admitted by the caller's certified contract; this value proves only exact
/// bytes and their lightweight content identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SchemaQualifiedCanonicalValue {
    schema_id: SchemaId,
    canonical: PlainCanonicalJsonBytes,
}

impl SchemaQualifiedCanonicalValue {
    /// Strictly reconstructs already-canonical, float-free JSON under one
    /// admitted schema identity.
    pub fn new(schema_id: SchemaId, canonical_bytes: &[u8]) -> Result<Self> {
        if canonical_bytes.len() > MAX_SCHEMA_QUALIFIED_VALUE_BYTES {
            return Err(ExecutorError::EvidenceBoundsExhausted);
        }
        let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(canonical_bytes)
            .map_err(|_| ExecutorError::CanonicalEncoding)?;
        Ok(Self {
            schema_id,
            canonical,
        })
    }

    /// Lifts one annex-validated value into the general executor value type.
    pub fn from_validated(value: &ValidatedCanonicalValueV3) -> Result<Self> {
        Self::new(value.schema_id().clone(), value.as_bytes())
    }

    /// Returns the admitted schema identity.
    pub const fn schema_id(&self) -> &SchemaId {
        &self.schema_id
    }

    /// Returns the exact retained canonical bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.canonical.as_bytes()
    }

    /// Returns the owned-form canonical JSON wrapper for graph promotion.
    pub const fn canonical_json(&self) -> &PlainCanonicalJsonBytes {
        &self.canonical
    }

    /// Returns the exact retained canonical text.
    pub fn as_str(&self) -> &str {
        self.canonical.as_str()
    }

    /// Computes the lightweight exact-byte content identity.
    pub fn reference(&self) -> Result<ContentRef> {
        ContentRef::new(
            self.schema_id.clone(),
            recoverability_contract()?.raw_content_digest(self.as_bytes()),
        )
        .map_err(|_| ExecutorError::CanonicalEncoding)
    }

    pub(crate) fn canonical_value(&self) -> Result<CanonicalValue> {
        let value: serde_json::Value = serde_json::from_slice(self.as_bytes())
            .map_err(|_| ExecutorError::CanonicalEncoding)?;
        plain_json_to_canonical_value(value)
    }
}

impl CanonicalExecutorRequest for SchemaQualifiedCanonicalValue {
    fn canonical_request(&self) -> &SchemaQualifiedCanonicalValue {
        self
    }
}

pub(crate) fn content_ref_value(value: &ContentRef) -> Result<CanonicalValue> {
    canonical_object([
        (
            "content_digest",
            CanonicalValue::String(value.content_digest().as_str().to_owned()),
        ),
        (
            "schema_id",
            CanonicalValue::String(value.schema_id().as_str().to_owned()),
        ),
    ])
}

pub(crate) fn optional_content_ref_value(value: Option<&ContentRef>) -> Result<CanonicalValue> {
    match value {
        Some(value) => content_ref_value(value),
        None => Ok(CanonicalValue::Null),
    }
}

pub(crate) fn canonical_object<const N: usize>(
    entries: [(&'static str, CanonicalValue); N],
) -> Result<CanonicalValue> {
    CanonicalValue::object(entries).map_err(|_| ExecutorError::CanonicalEncoding)
}

pub(crate) fn require_schema_ref(value: &ContentRef, schema_contract: &str) -> Result<()> {
    let expected = recoverability_contract()?
        .schema_id(schema_contract)
        .map_err(contract_error)?;
    if value.schema_id() != expected {
        return Err(ExecutorError::SchemaReferenceMismatch);
    }
    Ok(())
}

pub(crate) fn validate_reference_effect_identifier(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > MAX_REFERENCE_EFFECT_IDENTIFIER_BYTES {
        return Err(ExecutorError::InvalidReferenceEffectIdentifier);
    }
    encode(STABLE_ID_SCHEMA, &CanonicalValue::String(value.to_owned()))?;
    Ok(())
}

macro_rules! reviewed_content_ref {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(ContentRef);

        impl $name {
            /// Brands a non-authorizing reference selected by a reviewed contract.
            pub const fn from_reviewed(content_ref: ContentRef) -> Self {
                Self(content_ref)
            }

            /// Returns the lightweight schema-qualified content identity.
            pub const fn as_content_ref(&self) -> &ContentRef {
                &self.0
            }

            pub(crate) fn canonical_value(&self) -> Result<CanonicalValue> {
                content_ref_value(&self.0)
            }
        }
    };
}

reviewed_content_ref!(
    /// Lightweight content identity of a reviewed external-resource key.
    ResourceKeyRef
);
reviewed_content_ref!(
    /// Lightweight content identity of reviewed typed allocation state.
    AllocationStateRef
);
reviewed_content_ref!(
    /// Lightweight content identity of an authoritative public fencing commitment.
    FencingRef
);

/// Content identity of one exact executor deployment descriptor.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExecutorDeploymentRef(ContentRef);

impl ExecutorDeploymentRef {
    /// Returns the lightweight deployment content identity.
    pub const fn as_content_ref(&self) -> &ContentRef {
        &self.0
    }

    pub(crate) fn from_content_ref(value: ContentRef) -> Result<Self> {
        require_schema_ref(&value, EXECUTOR_DEPLOYMENT_SCHEMA)?;
        Ok(Self(value))
    }

    pub(crate) fn canonical_value(&self) -> Result<CanonicalValue> {
        content_ref_value(&self.0)
    }
}

/// Content identity of one exact external-resource ownership descriptor.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceOwnershipRef(ContentRef);

impl ResourceOwnershipRef {
    /// Returns the lightweight resource-ownership content identity.
    pub const fn as_content_ref(&self) -> &ContentRef {
        &self.0
    }

    pub(crate) fn from_content_ref(value: ContentRef) -> Result<Self> {
        require_schema_ref(&value, RESOURCE_OWNERSHIP_SCHEMA)?;
        Ok(Self(value))
    }

    pub(crate) fn canonical_value(&self) -> Result<CanonicalValue> {
        content_ref_value(&self.0)
    }
}

/// Exact immutable executor variant of a capability-binding reference.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExecutorBindingRef(ContentRef);

impl ExecutorBindingRef {
    /// Returns the lightweight capability-binding content identity.
    pub const fn as_content_ref(&self) -> &ContentRef {
        &self.0
    }

    /// Constructs an executor-binding reference after checking its exact schema.
    pub fn from_content_ref(value: ContentRef) -> Result<Self> {
        require_schema_ref(&value, EXECUTOR_BINDING_SCHEMA)?;
        Ok(Self(value))
    }

    pub(crate) fn canonical_value(&self) -> Result<CanonicalValue> {
        content_ref_value(&self.0)
    }
}

/// Non-secret deployment identity selected by an executor binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorDeployment {
    executor_namespace_ref: ContentRef,
    durable_ledger_generation_ref: ContentRef,
    tenant_scope_id: TenantScopeId,
    evidence_authority_ref: ContentRef,
    resource_ownership_ref: Option<ResourceOwnershipRef>,
}

impl ExecutorDeployment {
    /// Constructs and annex-validates one closed deployment descriptor.
    pub fn new(
        executor_namespace_ref: ContentRef,
        durable_ledger_generation_ref: ContentRef,
        tenant_scope_id: TenantScopeId,
        evidence_authority_ref: ContentRef,
        resource_ownership_ref: Option<ResourceOwnershipRef>,
    ) -> Result<Self> {
        let deployment = Self {
            executor_namespace_ref,
            durable_ledger_generation_ref,
            tenant_scope_id,
            evidence_authority_ref,
            resource_ownership_ref,
        };
        deployment.validated()?;
        Ok(deployment)
    }

    /// Strictly reconstructs exact frozen executor-deployment bytes.
    pub fn strict_decode(bytes: &[u8]) -> Result<Self> {
        let validated = recoverability_contract()?
            .strict_decode(EXECUTOR_DEPLOYMENT_SCHEMA, bytes)
            .map_err(contract_error)?;
        let json: serde_json::Value = serde_json::from_slice(validated.as_bytes())
            .map_err(|_| ExecutorError::CanonicalEncoding)?;
        let resource_ownership_ref = match json
            .get("resource_ownership_ref")
            .ok_or(ExecutorError::CanonicalEncoding)?
        {
            serde_json::Value::Null => None,
            value => Some(ResourceOwnershipRef::from_content_ref(decode_content_ref(
                value,
            )?)?),
        };
        let deployment = Self::new(
            decode_content_ref(
                json.get("executor_namespace_ref")
                    .ok_or(ExecutorError::CanonicalEncoding)?,
            )?,
            decode_content_ref(
                json.get("durable_ledger_generation_ref")
                    .ok_or(ExecutorError::CanonicalEncoding)?,
            )?,
            TenantScopeId::from_str(
                json.get("tenant_scope_id")
                    .and_then(serde_json::Value::as_str)
                    .ok_or(ExecutorError::CanonicalEncoding)?,
            )
            .map_err(|_| ExecutorError::CanonicalEncoding)?,
            decode_content_ref(
                json.get("evidence_authority_ref")
                    .ok_or(ExecutorError::CanonicalEncoding)?,
            )?,
            resource_ownership_ref,
        )?;
        if deployment.validated()? != validated {
            return Err(ExecutorError::CanonicalEncoding);
        }
        Ok(deployment)
    }

    /// Returns the executor ledger namespace.
    pub const fn executor_namespace_ref(&self) -> &ContentRef {
        &self.executor_namespace_ref
    }

    /// Returns the non-rollback durable ledger generation.
    pub const fn durable_ledger_generation_ref(&self) -> &ContentRef {
        &self.durable_ledger_generation_ref
    }

    /// Returns the deployment tenant ownership scope.
    pub const fn tenant_scope_id(&self) -> &TenantScopeId {
        &self.tenant_scope_id
    }

    /// Returns the admitted evidence authority.
    pub const fn evidence_authority_ref(&self) -> &ContentRef {
        &self.evidence_authority_ref
    }

    /// Returns cross-effect resource ownership when required.
    pub const fn resource_ownership_ref(&self) -> Option<&ResourceOwnershipRef> {
        self.resource_ownership_ref.as_ref()
    }

    /// Computes the exact annex-derived deployment content identity.
    pub fn reference(&self) -> Result<ExecutorDeploymentRef> {
        ExecutorDeploymentRef::from_content_ref(content_ref(&self.validated()?)?)
    }

    /// Returns the exact canonical deployment object.
    pub fn validated(&self) -> Result<ValidatedCanonicalValueV3> {
        encode(
            EXECUTOR_DEPLOYMENT_SCHEMA,
            &canonical_object([
                (
                    "durable_ledger_generation_ref",
                    content_ref_value(&self.durable_ledger_generation_ref)?,
                ),
                (
                    "evidence_authority_ref",
                    content_ref_value(&self.evidence_authority_ref)?,
                ),
                (
                    "executor_namespace_ref",
                    content_ref_value(&self.executor_namespace_ref)?,
                ),
                (
                    "resource_ownership_ref",
                    optional_content_ref_value(
                        self.resource_ownership_ref
                            .as_ref()
                            .map(ResourceOwnershipRef::as_content_ref),
                    )?,
                ),
                (
                    "tenant_scope_id",
                    CanonicalValue::String(self.tenant_scope_id.as_str().to_owned()),
                ),
                (
                    "version",
                    CanonicalValue::String(EXECUTOR_DEPLOYMENT_SCHEMA.to_owned()),
                ),
            ])?,
        )
    }
}

/// Non-secret ownership identity for a shared external resource domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceOwnership {
    coordination_namespace_ref: ContentRef,
    external_resource_domain_ref: ContentRef,
    durable_ledger_generation_ref: ContentRef,
    destination_fencing_authority_ref: Option<ContentRef>,
}

impl ResourceOwnership {
    /// Constructs and annex-validates one ownership descriptor.
    pub fn new(
        coordination_namespace_ref: ContentRef,
        external_resource_domain_ref: ContentRef,
        durable_ledger_generation_ref: ContentRef,
        destination_fencing_authority_ref: Option<ContentRef>,
    ) -> Result<Self> {
        let ownership = Self {
            coordination_namespace_ref,
            external_resource_domain_ref,
            durable_ledger_generation_ref,
            destination_fencing_authority_ref,
        };
        ownership.validated()?;
        Ok(ownership)
    }

    /// Strictly reconstructs exact frozen resource-ownership bytes.
    pub fn strict_decode(bytes: &[u8]) -> Result<Self> {
        let validated = recoverability_contract()?
            .strict_decode(RESOURCE_OWNERSHIP_SCHEMA, bytes)
            .map_err(contract_error)?;
        let json: serde_json::Value = serde_json::from_slice(validated.as_bytes())
            .map_err(|_| ExecutorError::CanonicalEncoding)?;
        let destination_fencing_authority_ref = match json
            .get("destination_fencing_authority_ref")
            .ok_or(ExecutorError::CanonicalEncoding)?
        {
            serde_json::Value::Null => None,
            value => Some(decode_content_ref(value)?),
        };
        let ownership = Self::new(
            decode_content_ref(
                json.get("coordination_namespace_ref")
                    .ok_or(ExecutorError::CanonicalEncoding)?,
            )?,
            decode_content_ref(
                json.get("external_resource_domain_ref")
                    .ok_or(ExecutorError::CanonicalEncoding)?,
            )?,
            decode_content_ref(
                json.get("durable_ledger_generation_ref")
                    .ok_or(ExecutorError::CanonicalEncoding)?,
            )?,
            destination_fencing_authority_ref,
        )?;
        if ownership.validated()? != validated {
            return Err(ExecutorError::CanonicalEncoding);
        }
        Ok(ownership)
    }

    /// Returns the shared coordination namespace.
    pub const fn coordination_namespace_ref(&self) -> &ContentRef {
        &self.coordination_namespace_ref
    }

    /// Returns the exact external resource domain.
    pub const fn external_resource_domain_ref(&self) -> &ContentRef {
        &self.external_resource_domain_ref
    }

    /// Returns the resource owner's durable generation.
    pub const fn durable_ledger_generation_ref(&self) -> &ContentRef {
        &self.durable_ledger_generation_ref
    }

    /// Returns the destination authority that fences old owners.
    pub const fn destination_fencing_authority_ref(&self) -> Option<&ContentRef> {
        self.destination_fencing_authority_ref.as_ref()
    }

    /// Computes the exact annex-derived ownership content identity.
    pub fn reference(&self) -> Result<ResourceOwnershipRef> {
        ResourceOwnershipRef::from_content_ref(content_ref(&self.validated()?)?)
    }

    /// Returns the exact canonical ownership object.
    pub fn validated(&self) -> Result<ValidatedCanonicalValueV3> {
        encode(
            RESOURCE_OWNERSHIP_SCHEMA,
            &canonical_object([
                (
                    "coordination_namespace_ref",
                    content_ref_value(&self.coordination_namespace_ref)?,
                ),
                (
                    "destination_fencing_authority_ref",
                    optional_content_ref_value(self.destination_fencing_authority_ref.as_ref())?,
                ),
                (
                    "durable_ledger_generation_ref",
                    content_ref_value(&self.durable_ledger_generation_ref)?,
                ),
                (
                    "external_resource_domain_ref",
                    content_ref_value(&self.external_resource_domain_ref)?,
                ),
                (
                    "version",
                    CanonicalValue::String(RESOURCE_OWNERSHIP_SCHEMA.to_owned()),
                ),
            ])?,
        )
    }
}

/// One planner expansion required by an executor contract.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RequiredPlanExpansion {
    executor_operation_id: String,
    typed_boundary_contract_ref: ContentRef,
    expansion_contract_ref: ContentRef,
}

impl RequiredPlanExpansion {
    /// Constructs one exact operation-to-boundary expansion requirement.
    pub fn new(
        executor_operation_id: impl Into<String>,
        typed_boundary_contract_ref: ContentRef,
        expansion_contract_ref: ContentRef,
    ) -> Result<Self> {
        let executor_operation_id = executor_operation_id.into();
        encode(
            STABLE_ID_SCHEMA,
            &CanonicalValue::String(executor_operation_id.clone()),
        )?;
        let expansion = Self {
            executor_operation_id,
            typed_boundary_contract_ref,
            expansion_contract_ref,
        };
        expansion.validated()?;
        Ok(expansion)
    }

    /// Returns the certified executor operation.
    pub fn executor_operation_id(&self) -> &str {
        &self.executor_operation_id
    }

    /// Returns the typed boundary selected by that operation.
    pub const fn typed_boundary_contract_ref(&self) -> &ContentRef {
        &self.typed_boundary_contract_ref
    }

    /// Returns the exact expansion contract.
    pub const fn expansion_contract_ref(&self) -> &ContentRef {
        &self.expansion_contract_ref
    }

    fn validated(&self) -> Result<ValidatedCanonicalValueV3> {
        encode(
            REQUIRED_PLAN_EXPANSION_SCHEMA,
            &canonical_object([
                (
                    "executor_operation_id",
                    CanonicalValue::String(self.executor_operation_id.clone()),
                ),
                (
                    "expansion_contract_ref",
                    content_ref_value(&self.expansion_contract_ref)?,
                ),
                (
                    "typed_boundary_contract_ref",
                    content_ref_value(&self.typed_boundary_contract_ref)?,
                ),
            ])?,
        )
    }
}

/// Closed descriptor-bound contracts for every retained executor relation.
///
/// The ensure-result and terminal-evidence contracts are exposed for runtime's
/// deterministic outer-object construction. The executor itself returns only
/// producer-free executor-owned closure members.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorRetainedClosureContract {
    ensure_result_contract: RetainedValueContract,
    delivery_audit_contract: RetainedValueContract,
    executor_frontier_contract: RetainedValueContract,
    terminal_evidence_contract: RetainedValueContract,
    terminal_tombstone_contract: RetainedValueContract,
    terminal_proof_contract: RetainedValueContract,
    domain_evidence_contract: RetainedValueContract,
}

impl ExecutorRetainedClosureContract {
    /// Constructs and annex-validates the complete relation contract.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ensure_result_contract: RetainedValueContract,
        delivery_audit_contract: RetainedValueContract,
        executor_frontier_contract: RetainedValueContract,
        terminal_evidence_contract: RetainedValueContract,
        terminal_tombstone_contract: RetainedValueContract,
        terminal_proof_contract: RetainedValueContract,
        domain_evidence_contract: RetainedValueContract,
    ) -> Result<Self> {
        require_retained_schema(&ensure_result_contract, EXECUTOR_ENSURE_RESULT_SCHEMA)?;
        require_retained_schema(&delivery_audit_contract, DELIVERY_FRONTIER_SCHEMA)?;
        require_retained_schema(&executor_frontier_contract, DELIVERY_FRONTIER_SCHEMA)?;
        require_retained_schema(&terminal_evidence_contract, TERMINAL_EFFECT_EVIDENCE_SCHEMA)?;
        require_retained_schema(&terminal_tombstone_contract, TERMINAL_TOMBSTONE_SCHEMA)?;
        require_retained_schema(&terminal_proof_contract, TERMINAL_PROOF_SCHEMA)?;
        let contract = Self {
            ensure_result_contract,
            delivery_audit_contract,
            executor_frontier_contract,
            terminal_evidence_contract,
            terminal_tombstone_contract,
            terminal_proof_contract,
            domain_evidence_contract,
        };
        contract.validated()?;
        Ok(contract)
    }

    /// Strictly reconstructs closure-contract bytes under the frozen annex.
    pub fn strict_decode(bytes: &[u8]) -> Result<Self> {
        let validated = recoverability_contract()?
            .strict_decode(EXECUTOR_RETAINED_CLOSURE_CONTRACT_SCHEMA, bytes)
            .map_err(contract_error)?;
        Self::from_validated(validated)
    }

    /// Reconstructs a closure contract from an annex-validated value.
    pub fn from_validated(validated: ValidatedCanonicalValueV3) -> Result<Self> {
        if validated.schema_contract() != EXECUTOR_RETAINED_CLOSURE_CONTRACT_SCHEMA {
            return Err(ExecutorError::SchemaReferenceMismatch);
        }
        let json: serde_json::Value = serde_json::from_slice(validated.as_bytes())
            .map_err(|_| ExecutorError::CanonicalEncoding)?;
        let contract = Self::from_json(&json)?;
        if contract.validated()? != validated {
            return Err(ExecutorError::CanonicalEncoding);
        }
        Ok(contract)
    }

    /// Returns the runtime-constructed outer ensure-result contract.
    pub const fn ensure_result_contract(&self) -> &RetainedValueContract {
        &self.ensure_result_contract
    }

    /// Returns the head delivery-audit relation contract.
    pub const fn delivery_audit_contract(&self) -> &RetainedValueContract {
        &self.delivery_audit_contract
    }

    /// Returns the predecessor executor-frontier relation contract.
    pub const fn executor_frontier_contract(&self) -> &RetainedValueContract {
        &self.executor_frontier_contract
    }

    /// Returns the runtime-constructed terminal-evidence contract.
    pub const fn terminal_evidence_contract(&self) -> &RetainedValueContract {
        &self.terminal_evidence_contract
    }

    /// Returns the immutable terminal-tombstone relation contract.
    pub const fn terminal_tombstone_contract(&self) -> &RetainedValueContract {
        &self.terminal_tombstone_contract
    }

    /// Returns the exact-attempt terminal-proof relation contract.
    pub const fn terminal_proof_contract(&self) -> &RetainedValueContract {
        &self.terminal_proof_contract
    }

    /// Returns the domain-evidence relation contract.
    pub const fn domain_evidence_contract(&self) -> &RetainedValueContract {
        &self.domain_evidence_contract
    }

    /// Returns the exact canonical executor retained-closure contract.
    pub fn validated(&self) -> Result<ValidatedCanonicalValueV3> {
        encode(
            EXECUTOR_RETAINED_CLOSURE_CONTRACT_SCHEMA,
            &self.canonical_value()?,
        )
    }

    fn canonical_value(&self) -> Result<CanonicalValue> {
        canonical_object([
            (
                "delivery_audit_contract",
                retained_contract_canonical_value(&self.delivery_audit_contract)?,
            ),
            (
                "domain_evidence_contract",
                retained_contract_canonical_value(&self.domain_evidence_contract)?,
            ),
            (
                "ensure_result_contract",
                retained_contract_canonical_value(&self.ensure_result_contract)?,
            ),
            (
                "executor_frontier_contract",
                retained_contract_canonical_value(&self.executor_frontier_contract)?,
            ),
            (
                "terminal_evidence_contract",
                retained_contract_canonical_value(&self.terminal_evidence_contract)?,
            ),
            (
                "terminal_proof_contract",
                retained_contract_canonical_value(&self.terminal_proof_contract)?,
            ),
            (
                "terminal_tombstone_contract",
                retained_contract_canonical_value(&self.terminal_tombstone_contract)?,
            ),
        ])
    }

    fn from_json(value: &serde_json::Value) -> Result<Self> {
        let retained = |name| retained_contract_from_json(required_json_field(value, name)?);
        Self::new(
            retained("ensure_result_contract")?,
            retained("delivery_audit_contract")?,
            retained("executor_frontier_contract")?,
            retained("terminal_evidence_contract")?,
            retained("terminal_tombstone_contract")?,
            retained("terminal_proof_contract")?,
            retained("domain_evidence_contract")?,
        )
    }
}

fn retained_contract_canonical_value(contract: &RetainedValueContract) -> Result<CanonicalValue> {
    contract
        .validated()
        .map_err(retained_contract_error)?
        .canonical_value()
        .map_err(contract_error)
}

fn retained_contract_from_json(value: &serde_json::Value) -> Result<RetainedValueContract> {
    serde_json::from_value(value.clone()).map_err(|_| ExecutorError::CanonicalEncoding)
}

fn required_json_field<'a>(
    value: &'a serde_json::Value,
    name: &str,
) -> Result<&'a serde_json::Value> {
    value.get(name).ok_or(ExecutorError::CanonicalEncoding)
}

fn retained_contract_error(error: ValueError) -> ExecutorError {
    match error {
        ValueError::Recoverability(error) => contract_error(error),
        ValueError::Descriptor(_)
        | ValueError::Identity(_)
        | ValueError::InvalidSchemaIdentity
        | ValueError::SchemaShapeMismatch
        | ValueError::ArtifactTypeMismatch { .. }
        | ValueError::Config(_)
        | ValueError::RetainedValueContract => ExecutorError::CanonicalEncoding,
    }
}

fn require_retained_schema(contract: &RetainedValueContract, schema_contract: &str) -> Result<()> {
    let expected = recoverability_contract()?
        .schema_id(schema_contract)
        .map_err(contract_error)?;
    if contract.schema_id() != expected {
        return Err(ExecutorError::RetainedValueContractMismatch);
    }
    Ok(())
}

/// Frozen generic semantic contract selected by an executor binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorContractDescriptor {
    ensure_contract_ref: ContentRef,
    semantic_request_contract: RetainedValueContract,
    safe_failure_value_contract: RetainedValueContract,
    retained_closure_contract: ExecutorRetainedClosureContract,
    safe_failure_contract_ref: ContentRef,
    downstream_convergence_contract_ref: ContentRef,
    evidence_bounds: EvidenceBounds,
    resource_domain_requirement: Option<ContentRef>,
    required_plan_expansions: Vec<RequiredPlanExpansion>,
}

impl ExecutorContractDescriptor {
    /// Constructs and strictly annex-validates one generic executor contract.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ensure_contract_ref: ContentRef,
        semantic_request_contract: RetainedValueContract,
        safe_failure_value_contract: RetainedValueContract,
        retained_closure_contract: ExecutorRetainedClosureContract,
        safe_failure_contract_ref: ContentRef,
        downstream_convergence_contract_ref: ContentRef,
        evidence_bounds: EvidenceBounds,
        resource_domain_requirement: Option<ContentRef>,
        required_plan_expansions: Vec<RequiredPlanExpansion>,
    ) -> Result<Self> {
        require_retained_schema(&safe_failure_value_contract, SAFE_FAILURE_VALUE_SCHEMA)?;
        let descriptor = Self {
            ensure_contract_ref,
            semantic_request_contract,
            safe_failure_value_contract,
            retained_closure_contract,
            safe_failure_contract_ref,
            downstream_convergence_contract_ref,
            evidence_bounds,
            resource_domain_requirement,
            required_plan_expansions,
        };
        descriptor.validated()?;
        Ok(descriptor)
    }

    /// Strictly reconstructs the exact frozen descriptor bytes.
    pub fn strict_decode(bytes: &[u8]) -> Result<Self> {
        let validated = recoverability_contract()?
            .strict_decode(EXECUTOR_CONTRACT_DESCRIPTOR_SCHEMA, bytes)
            .map_err(contract_error)?;
        Self::from_validated(validated)
    }

    /// Reconstructs a descriptor from an already annex-validated value.
    pub fn from_validated(validated: ValidatedCanonicalValueV3) -> Result<Self> {
        if validated.schema_contract() != EXECUTOR_CONTRACT_DESCRIPTOR_SCHEMA {
            return Err(ExecutorError::SchemaReferenceMismatch);
        }
        let json: serde_json::Value = serde_json::from_slice(validated.as_bytes())
            .map_err(|_| ExecutorError::CanonicalEncoding)?;
        let field = |name| json.get(name).ok_or(ExecutorError::CanonicalEncoding);
        let content = |name| decode_content_ref(field(name)?);
        let bounds_json = field("evidence_bounds")?;
        let bounds_bytes =
            serde_json::to_vec(bounds_json).map_err(|_| ExecutorError::CanonicalEncoding)?;
        let bounds = EvidenceBounds::strict_decode(&bounds_bytes)?;
        let resource_domain_requirement = match field("resource_domain_requirement")? {
            serde_json::Value::Null => None,
            value => Some(decode_content_ref(value)?),
        };
        let expansions = field("required_plan_expansions")?
            .as_array()
            .ok_or(ExecutorError::CanonicalEncoding)?
            .iter()
            .map(|value| {
                let operation_id = value
                    .get("executor_operation_id")
                    .and_then(serde_json::Value::as_str)
                    .ok_or(ExecutorError::CanonicalEncoding)?;
                RequiredPlanExpansion::new(
                    operation_id,
                    decode_content_ref(
                        value
                            .get("typed_boundary_contract_ref")
                            .ok_or(ExecutorError::CanonicalEncoding)?,
                    )?,
                    decode_content_ref(
                        value
                            .get("expansion_contract_ref")
                            .ok_or(ExecutorError::CanonicalEncoding)?,
                    )?,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let descriptor = Self::new(
            content("ensure_contract_ref")?,
            retained_contract_from_json(field("semantic_request_contract")?)?,
            retained_contract_from_json(field("safe_failure_value_contract")?)?,
            ExecutorRetainedClosureContract::from_json(field("retained_closure_contract")?)?,
            content("safe_failure_contract_ref")?,
            content("downstream_convergence_contract_ref")?,
            bounds,
            resource_domain_requirement,
            expansions,
        )?;
        if descriptor.validated()? != validated {
            return Err(ExecutorError::CanonicalEncoding);
        }
        Ok(descriptor)
    }

    /// Returns the generic ensure interface contract.
    pub const fn ensure_contract_ref(&self) -> &ContentRef {
        &self.ensure_contract_ref
    }

    /// Returns the exact retained semantic-request contract.
    pub const fn semantic_request_contract(&self) -> &RetainedValueContract {
        &self.semantic_request_contract
    }

    /// Returns the exact retained safe-failure value contract.
    pub const fn safe_failure_value_contract(&self) -> &RetainedValueContract {
        &self.safe_failure_value_contract
    }

    /// Returns every descriptor-bound retained closure relation.
    pub const fn retained_closure_contract(&self) -> &ExecutorRetainedClosureContract {
        &self.retained_closure_contract
    }

    /// Returns the closed safe-failure contract.
    pub const fn safe_failure_contract_ref(&self) -> &ContentRef {
        &self.safe_failure_contract_ref
    }

    /// Returns the downstream convergence contract.
    pub const fn downstream_convergence_contract_ref(&self) -> &ContentRef {
        &self.downstream_convergence_contract_ref
    }

    /// Returns the immutable retained-evidence bounds.
    pub const fn evidence_bounds(&self) -> &EvidenceBounds {
        &self.evidence_bounds
    }

    /// Returns the required shared resource domain, when any.
    pub const fn resource_domain_requirement(&self) -> Option<&ContentRef> {
        self.resource_domain_requirement.as_ref()
    }

    /// Returns the ordered unique planning expansions.
    pub fn required_plan_expansions(&self) -> &[RequiredPlanExpansion] {
        &self.required_plan_expansions
    }

    /// Returns the exact canonical descriptor object.
    pub fn validated(&self) -> Result<ValidatedCanonicalValueV3> {
        let expansions = self
            .required_plan_expansions
            .iter()
            .map(|expansion| {
                expansion
                    .validated()?
                    .canonical_value()
                    .map_err(contract_error)
            })
            .collect::<Result<Vec<_>>>()?;
        encode(
            EXECUTOR_CONTRACT_DESCRIPTOR_SCHEMA,
            &canonical_object([
                (
                    "downstream_convergence_contract_ref",
                    content_ref_value(&self.downstream_convergence_contract_ref)?,
                ),
                (
                    "ensure_contract_ref",
                    content_ref_value(&self.ensure_contract_ref)?,
                ),
                (
                    "evidence_bounds",
                    self.evidence_bounds
                        .validated()?
                        .canonical_value()
                        .map_err(contract_error)?,
                ),
                (
                    "required_plan_expansions",
                    CanonicalValue::Array(expansions),
                ),
                (
                    "retained_closure_contract",
                    self.retained_closure_contract.canonical_value()?,
                ),
                (
                    "resource_domain_requirement",
                    optional_content_ref_value(self.resource_domain_requirement.as_ref())?,
                ),
                (
                    "safe_failure_value_contract",
                    retained_contract_canonical_value(&self.safe_failure_value_contract)?,
                ),
                (
                    "safe_failure_contract_ref",
                    content_ref_value(&self.safe_failure_contract_ref)?,
                ),
                (
                    "semantic_request_contract",
                    retained_contract_canonical_value(&self.semantic_request_contract)?,
                ),
                (
                    "version",
                    CanonicalValue::String(EXECUTOR_CONTRACT_DESCRIPTOR_SCHEMA.to_owned()),
                ),
            ])?,
        )
    }

    /// Computes the descriptor's exact content identity.
    pub fn reference(&self) -> Result<ContentRef> {
        content_ref(&self.validated()?)
    }
}

/// Immutable executor capability-binding variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorBinding {
    executor_contract_ref: ContentRef,
    admitted_implementation_ref: ContentRef,
    executor_deployment_ref: ExecutorDeploymentRef,
}

impl ExecutorBinding {
    /// Constructs and annex-validates one exact executor binding.
    pub fn new(
        executor_contract_ref: ContentRef,
        admitted_implementation_ref: ContentRef,
        executor_deployment_ref: ExecutorDeploymentRef,
    ) -> Result<Self> {
        let binding = Self {
            executor_contract_ref,
            admitted_implementation_ref,
            executor_deployment_ref,
        };
        binding.validated()?;
        Ok(binding)
    }

    /// Strictly reconstructs exact frozen executor-binding bytes.
    pub fn strict_decode(bytes: &[u8]) -> Result<Self> {
        let validated = recoverability_contract()?
            .strict_decode(EXECUTOR_BINDING_SCHEMA, bytes)
            .map_err(contract_error)?;
        let json: serde_json::Value = serde_json::from_slice(validated.as_bytes())
            .map_err(|_| ExecutorError::CanonicalEncoding)?;
        let binding = Self::new(
            decode_content_ref(
                json.get("executor_contract_ref")
                    .ok_or(ExecutorError::CanonicalEncoding)?,
            )?,
            decode_content_ref(
                json.get("admitted_implementation_ref")
                    .ok_or(ExecutorError::CanonicalEncoding)?,
            )?,
            ExecutorDeploymentRef::from_content_ref(decode_content_ref(
                json.get("executor_deployment_ref")
                    .ok_or(ExecutorError::CanonicalEncoding)?,
            )?)?,
        )?;
        if binding.validated()? != validated {
            return Err(ExecutorError::CanonicalEncoding);
        }
        Ok(binding)
    }

    /// Returns the admitted executor semantic contract.
    pub const fn executor_contract_ref(&self) -> &ContentRef {
        &self.executor_contract_ref
    }

    /// Returns the admitted implementation descriptor.
    pub const fn admitted_implementation_ref(&self) -> &ContentRef {
        &self.admitted_implementation_ref
    }

    /// Returns the exact selected deployment.
    pub const fn executor_deployment_ref(&self) -> &ExecutorDeploymentRef {
        &self.executor_deployment_ref
    }

    /// Computes the exact annex-derived capability-binding reference.
    pub fn reference(&self) -> Result<ExecutorBindingRef> {
        ExecutorBindingRef::from_content_ref(content_ref(&self.validated()?)?)
    }

    /// Returns the exact canonical binding object.
    pub fn validated(&self) -> Result<ValidatedCanonicalValueV3> {
        encode(
            EXECUTOR_BINDING_SCHEMA,
            &canonical_object([
                (
                    "admitted_implementation_ref",
                    content_ref_value(&self.admitted_implementation_ref)?,
                ),
                (
                    "executor_contract_ref",
                    content_ref_value(&self.executor_contract_ref)?,
                ),
                (
                    "executor_deployment_ref",
                    self.executor_deployment_ref.canonical_value()?,
                ),
                (
                    "version",
                    CanonicalValue::String(EXECUTOR_BINDING_SCHEMA.to_owned()),
                ),
            ])?,
        )
    }
}

/// Fully resolved and cross-checked executor binding material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedExecutorBinding {
    binding: ExecutorBinding,
    binding_ref: ExecutorBindingRef,
    contract: ExecutorContractDescriptor,
    deployment: ExecutorDeployment,
    resource_ownership: Option<ResourceOwnership>,
}

impl VerifiedExecutorBinding {
    /// Resolves one binding and proves contract, deployment, ownership,
    /// generation, resource-domain, and tenant equality.
    pub fn verify(
        binding: ExecutorBinding,
        contract: ExecutorContractDescriptor,
        deployment: ExecutorDeployment,
        resource_ownership: Option<ResourceOwnership>,
        admitted_tenant_scope_id: &TenantScopeId,
    ) -> Result<Self> {
        if binding.executor_contract_ref() != &contract.reference()? {
            return Err(ExecutorError::ExecutorContractReferenceMismatch);
        }
        if binding.executor_deployment_ref() != &deployment.reference()? {
            return Err(ExecutorError::DeploymentReferenceMismatch);
        }
        if deployment.tenant_scope_id() != admitted_tenant_scope_id {
            return Err(ExecutorError::TenantScopeMismatch);
        }
        match (
            deployment.resource_ownership_ref(),
            resource_ownership.as_ref(),
        ) {
            (None, None) => {}
            (Some(expected), Some(ownership)) => {
                if expected != &ownership.reference()? {
                    return Err(ExecutorError::ResourceOwnershipReferenceMismatch);
                }
                if ownership.durable_ledger_generation_ref()
                    != deployment.durable_ledger_generation_ref()
                {
                    return Err(ExecutorError::LedgerGenerationMismatch);
                }
            }
            _ => return Err(ExecutorError::ResourceOwnershipReferenceMismatch),
        }
        if let Some(required_domain) = contract.resource_domain_requirement() {
            let ownership = resource_ownership
                .as_ref()
                .ok_or(ExecutorError::ResourceOwnershipRequired)?;
            if ownership.external_resource_domain_ref() != required_domain {
                return Err(ExecutorError::ResourceDomainMismatch);
            }
        }
        let binding_ref = binding.reference()?;
        Ok(Self {
            binding,
            binding_ref,
            contract,
            deployment,
            resource_ownership,
        })
    }

    /// Returns the immutable binding reference used by every effect surface.
    pub const fn binding_ref(&self) -> &ExecutorBindingRef {
        &self.binding_ref
    }

    /// Returns the resolved binding descriptor.
    pub const fn binding(&self) -> &ExecutorBinding {
        &self.binding
    }

    /// Returns the exact resolved semantic contract.
    pub const fn contract(&self) -> &ExecutorContractDescriptor {
        &self.contract
    }

    /// Returns the resolved deployment descriptor.
    pub const fn deployment(&self) -> &ExecutorDeployment {
        &self.deployment
    }

    /// Returns the resolved resource owner, when present.
    pub const fn resource_ownership(&self) -> Option<&ResourceOwnership> {
        self.resource_ownership.as_ref()
    }
}

/// Exact immutable identity tuple retained by one executor effect stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectIdentity {
    tenant_scope_id: TenantScopeId,
    executor_binding_ref: ExecutorBindingRef,
    effect_key: EffectKey,
    request_digest: RequestDigest,
}

impl EffectIdentity {
    pub(crate) fn from_parts(
        tenant_scope_id: TenantScopeId,
        executor_binding_ref: ExecutorBindingRef,
        effect_key: EffectKey,
        request_digest: RequestDigest,
    ) -> Self {
        Self {
            tenant_scope_id,
            executor_binding_ref,
            effect_key,
            request_digest,
        }
    }

    /// Reconstructs retained identity fields under one exact verified binding.
    pub fn reconstruct(
        binding: &VerifiedExecutorBinding,
        tenant_scope_id: TenantScopeId,
        executor_binding_ref: ExecutorBindingRef,
        effect_key: EffectKey,
        request_digest: RequestDigest,
    ) -> Result<Self> {
        if &executor_binding_ref != binding.binding_ref() {
            return Err(ExecutorError::WrongExecutorBinding);
        }
        if &tenant_scope_id != binding.deployment().tenant_scope_id() {
            return Err(ExecutorError::TenantScopeMismatch);
        }
        Ok(Self::from_parts(
            tenant_scope_id,
            executor_binding_ref,
            effect_key,
            request_digest,
        ))
    }

    /// Returns the tenant partition authenticated by the binding.
    pub const fn tenant_scope_id(&self) -> &TenantScopeId {
        &self.tenant_scope_id
    }

    /// Returns the exact executor binding reference.
    pub const fn executor_binding_ref(&self) -> &ExecutorBindingRef {
        &self.executor_binding_ref
    }

    /// Returns the kernel-derived effect key.
    pub const fn effect_key(&self) -> &EffectKey {
        &self.effect_key
    }

    /// Returns the immutable semantic request digest.
    pub const fn request_digest(&self) -> &RequestDigest {
        &self.request_digest
    }
}

/// Pure executor request contract backed by one schema-qualified canonical value.
pub trait CanonicalExecutorRequest {
    /// Returns the exact certified request value.
    fn canonical_request(&self) -> &SchemaQualifiedCanonicalValue;
}

/// Exact non-authorizing request data reconstructed from committed MFM history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedEffectRequest<Request> {
    identity: EffectIdentity,
    request: Request,
}

impl<Request> CommittedEffectRequest<Request>
where
    Request: CanonicalExecutorRequest,
{
    /// Derives request and effect identity from the frozen semantic domains.
    pub fn new(
        executor_binding_ref: ExecutorBindingRef,
        tenant_scope_id: TenantScopeId,
        store_scope_id: &StoreScopeId,
        run_id: &RunId,
        node_id: &NodeId,
        request: Request,
    ) -> Result<Self> {
        let request_value = request.canonical_request();
        let request_preimage = encode(
            REQUEST_PREIMAGE_SCHEMA,
            &canonical_object([
                (
                    "request_schema_id",
                    CanonicalValue::String(request_value.schema_id().as_str().to_owned()),
                ),
                ("request_value", request_value.canonical_value()?),
            ])?,
        )?;
        let request_digest = recoverability_contract()?
            .derive_request_digest(&request_preimage)
            .map_err(contract_error)?;

        let effect_preimage = encode(
            EFFECT_KEY_PREIMAGE_SCHEMA,
            &canonical_object([
                (
                    "executor_binding_ref",
                    executor_binding_ref.canonical_value()?,
                ),
                (
                    "node_id",
                    CanonicalValue::String(node_id.as_str().to_owned()),
                ),
                ("run_id", CanonicalValue::String(run_id.as_str().to_owned())),
                (
                    "store_scope_id",
                    CanonicalValue::String(store_scope_id.as_str().to_owned()),
                ),
            ])?,
        )?;
        let effect_digest = recoverability_contract()?
            .semantic_digest(EFFECT_KEY_DOMAIN, &effect_preimage)
            .map_err(contract_error)?;
        let effect_key = EffectKey::from_semantic_digest(effect_digest);

        Ok(Self {
            identity: EffectIdentity {
                tenant_scope_id,
                executor_binding_ref,
                effect_key,
                request_digest,
            },
            request,
        })
    }
}

impl<Request> CommittedEffectRequest<Request> {
    /// Returns the exact immutable effect identity.
    pub const fn identity(&self) -> &EffectIdentity {
        &self.identity
    }

    /// Returns the reviewed semantic request.
    pub const fn request(&self) -> &Request {
        &self.request
    }

    /// Splits the non-authorizing request data into identity and request.
    pub fn into_parts(self) -> (EffectIdentity, Request) {
        (self.identity, self.request)
    }
}

/// Sendable future returned by an executor implementation.
pub type ExecutorFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub(crate) fn semantic_digest_value(value: &SemanticDigest) -> CanonicalValue {
    CanonicalValue::String(value.as_str().to_owned())
}

pub(crate) fn effect_key_value(value: &EffectKey) -> CanonicalValue {
    CanonicalValue::String(value.as_str().to_owned())
}

fn decode_content_ref(value: &serde_json::Value) -> Result<ContentRef> {
    serde_json::from_value(value.clone()).map_err(|_| ExecutorError::CanonicalEncoding)
}

pub(crate) fn plain_json_to_canonical_value(value: serde_json::Value) -> Result<CanonicalValue> {
    match value {
        serde_json::Value::Null => Ok(CanonicalValue::Null),
        serde_json::Value::Bool(value) => Ok(CanonicalValue::Bool(value)),
        serde_json::Value::Number(value) => {
            if let Some(value) = value.as_u64() {
                Ok(CanonicalValue::Unsigned(value))
            } else if let Some(value) = value.as_i64() {
                Ok(CanonicalValue::Signed(value))
            } else {
                Err(ExecutorError::CanonicalEncoding)
            }
        }
        serde_json::Value::String(value) => Ok(CanonicalValue::String(value)),
        serde_json::Value::Array(values) => values
            .into_iter()
            .map(plain_json_to_canonical_value)
            .collect::<Result<Vec<_>>>()
            .map(CanonicalValue::Array),
        serde_json::Value::Object(values) => CanonicalValue::object(
            values
                .into_iter()
                .map(|(key, value)| Ok((key, plain_json_to_canonical_value(value)?)))
                .collect::<Result<Vec<_>>>()?,
        )
        .map_err(|_| ExecutorError::CanonicalEncoding),
    }
}
