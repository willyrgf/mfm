//! Runtime catalog bindings for the six exact EVM read operations.
//!
//! These wrappers consume runtime's affine read authority and delegate to one
//! typed transport method. They own no append, retry, failover, replay, or
//! semantic reduction authority.
//!
//! Runtime qualification deliberately preserves the kernel's reviewed error
//! classification instead of replacing it with a smaller local error.

#![allow(clippy::result_large_err)]

use std::sync::Arc;

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::{
    BoundaryStage, CoarseSizeClass, FailureClass, SafeFailureClassifierDescriptor,
    SafeFailureClassifierRule, SafeFailureDiagnosticConstraint, SafeFailureDiagnosticRule,
    SafeFailureOutcome, SafeFailureSizeRule,
};
use mfm_evm::{
    evm_read_capability_contract_canonical, evm_read_capability_contract_ref,
    evm_read_value_contracts, evm_safe_failure_contract_canonical, evm_safe_failure_contract_ref,
    EvmAnchorConfirmationRequest, EvmBlockResponse, EvmChainIdentityRequest,
    EvmChainIdentityResponse, EvmCoarseSizeClass, EvmLatestAnchorRequest, EvmNativeBalanceRequest,
    EvmQuantityResponse, EvmResponseInvalidKind, EvmSafeDiagnostic, EvmSafeFailure,
    EvmTokenBalanceRequest, EvmTokenDecimalsRequest, EvmTokenDecimalsResponse,
    EVM_CHAIN_ID_OPERATION_ID, EVM_CONFIRM_ANCHOR_OPERATION_ID, EVM_LATEST_ANCHOR_OPERATION_ID,
    EVM_NATIVE_BALANCE_OPERATION_ID, EVM_TOKEN_BALANCE_OPERATION_ID,
    EVM_TOKEN_DECIMALS_OPERATION_ID,
};
use mfm_ids::{ContentRef, DigestAlgorithm, FieldPath, SchemaId, SemanticTypeId, StableId};
use mfm_journal::v1::ReadCapabilityBinding;
use mfm_program::{
    boundary_content_ref, QualifiedProgramRegistryBuilder, QualifiedReadEntry,
    QualifiedReadOperationContract,
};
use mfm_runtime::{
    qualify_read_capability, AuditedReadCapability, AuthorizedReadAccess, ReadCapabilityFuture,
    ReadCapabilityOutcome, RuntimeError,
};
use mfm_spec::{ComponentImplementationDescriptor, ComponentKind};
use mfm_store::{AdmittedSupportGraph, AdmittedSupportMember, SafeFailureMetadata};
use mfm_values::{MfmValue, RetainedValueContract};
use serde::{Deserialize, Serialize};

use crate::transport::{EvmJsonRpcTransport, EvmTransportOutcome};

/// Exact version of the sealed live adapter callback surface.
pub const EVM_ADAPTER_CALLBACK_SURFACE_VERSION: &str = "mfm.evm-live.adapter-callback-surface.v1";
const OBJECT_EVIDENCE_PATH: &str = "qualification.object_evidence_contract";
const EXECUTABLE_IDENTITY_PATH: &str = "executable.identity";
const SHARED_QUALIFICATION_PATH: &str = "qualification.descriptor";
const EVM_ADAPTER_SEMANTIC_PATH: &str = "capability.evm_live.semantic_contract";
const EVM_ADAPTER_CALLBACK_PATH: &str = "capability.evm_live.callback_surface";
const EVM_ADAPTER_IMPLEMENTATION_PATH: &str = "capability.evm_live.adapter_implementation";

const OBJECT_EVIDENCE_ROLE: &str = "mfm.product.qualification.object-evidence-contract";
const EXECUTABLE_IDENTITY_ROLE: &str = "mfm.qualification.executable-identity";
const SHARED_QUALIFICATION_ROLE: &str = "mfm.product.qualification.descriptor";
const EVM_ADAPTER_SEMANTIC_ROLE: &str =
    "mfm.qualification.read-capability-adapter-verifier.semantic-contract";
const EVM_ADAPTER_CALLBACK_ROLE: &str =
    "mfm.qualification.read-capability-adapter-verifier.callback-surface";
const EVM_ADAPTER_IMPLEMENTATION_ROLE: &str =
    "mfm.qualification.read-capability-adapter-verifier.implementation";
const SHARED_QUALIFICATION_VERSION: &str = "mfm.component-qualification.v1";
const SHARED_QUALIFICATION_COMPONENT_COUNT: usize = 12;

/// Returns the sealed six-callback adapter surface.
pub fn evm_adapter_callback_surface_canonical() -> mfm_runtime::Result<PlainCanonicalJsonBytes> {
    PlainCanonicalJsonBytes::from_json_str(
        r#"{"arbitrary_methods":false,"batching":false,"callbacks":["eth_call_erc20_balance_of","eth_call_erc20_decimals","eth_chain_id","eth_get_balance","eth_get_block_by_number_confirm","eth_get_block_by_number_latest"],"failover":false,"redirects":false,"reselection":false,"retries":false,"version":"mfm.evm-live.adapter-callback-surface.v1"}"#,
    )
    .map_err(|_| RuntimeError::CatalogSelection)
}

/// Returns the sealed six-callback adapter surface identity.
pub fn evm_adapter_callback_surface_ref() -> mfm_runtime::Result<ContentRef> {
    boundary_content_ref(
        descriptor_schema_id("mfm.evm-live.adapter-callback-surface")?,
        &evm_adapter_callback_surface_canonical()?,
    )
    .map_err(Into::into)
}

/// Builds retained metadata for the sealed callback-surface support object.
pub fn evm_adapter_callback_surface_support_contract(
    role: StableId,
    evidence_contract_ref: ContentRef,
) -> mfm_runtime::Result<RetainedValueContract> {
    descriptor_support_contract(
        "mfm.evm-live.adapter-callback-surface",
        "adapter-callback-surface",
        role,
        evidence_contract_ref,
    )
}

/// Returns the exact exhaustive safe-classifier descriptor.
pub fn evm_safe_classifier_canonical() -> mfm_runtime::Result<PlainCanonicalJsonBytes> {
    evm_safe_classifier()?
        .canonical()
        .map_err(|_| RuntimeError::CatalogSelection)
}

/// Returns the exact exhaustive safe-classifier identity.
pub fn evm_safe_classifier_contract_ref() -> mfm_runtime::Result<ContentRef> {
    evm_safe_classifier()?
        .content_ref()
        .map_err(|_| RuntimeError::CatalogSelection)
}

/// Builds retained metadata for the sealed safe-classifier support object.
pub fn evm_safe_classifier_support_contract(
    role: StableId,
    evidence_contract_ref: ContentRef,
) -> mfm_runtime::Result<RetainedValueContract> {
    retained_support_contract(
        SafeFailureClassifierDescriptor::schema_id().map_err(|_| RuntimeError::CatalogSelection)?,
        "mfm.evm-live",
        "safe-classifier",
        role,
        evidence_contract_ref,
    )
}

/// Sealed proof that the admitted product support graph contains the exact EVM read adapter.
///
/// Construction binds the adapter to the product's sole executable qualification. The proof
/// borrows that admitted graph so a read registry cannot outlive or substitute its support
/// authority.
pub struct EvmReadQualificationArtifacts<'support> {
    support_graph: &'support AdmittedSupportGraph,
    adapter_implementation: ComponentImplementationDescriptor,
    object_evidence_contract_ref: ContentRef,
}

impl<'support> EvmReadQualificationArtifacts<'support> {
    /// Verifies and seals the exact product-qualified EVM read adapter tuple.
    pub fn new(
        support_graph: &'support AdmittedSupportGraph,
        executable_identity_ref: &ContentRef,
        shared_qualification_ref: &ContentRef,
        object_evidence_contract_ref: &ContentRef,
    ) -> mfm_runtime::Result<Self> {
        let expected_evidence_ref = mfm_values::component_object_evidence_contract_ref()
            .map_err(|_| RuntimeError::CatalogSelection)?;
        if object_evidence_contract_ref != &expected_evidence_ref
            || executable_identity_ref.schema_id()
                != &mfm_spec::schema_id("mfm.executable-bytes-descriptor.v1")?
            || shared_qualification_ref.schema_id()
                != &mfm_spec::schema_id(SHARED_QUALIFICATION_VERSION)?
        {
            return Err(RuntimeError::CatalogSelection);
        }

        let evidence_contract = retained_support_contract(
            object_evidence_contract_ref.schema_id().clone(),
            "mfm.product",
            "component-object-evidence-contract",
            stable_id(OBJECT_EVIDENCE_ROLE)?,
            object_evidence_contract_ref.clone(),
        )?;
        verify_admitted_member(
            support_graph,
            OBJECT_EVIDENCE_PATH,
            object_evidence_contract_ref,
            Some(
                mfm_values::component_object_evidence_contract_canonical()
                    .map_err(|_| RuntimeError::CatalogSelection)?
                    .as_bytes(),
            ),
            &evidence_contract,
        )?;

        let executable_contract = retained_support_contract(
            executable_identity_ref.schema_id().clone(),
            "mfm.recoverability",
            "executable-bytes-descriptor",
            stable_id(EXECUTABLE_IDENTITY_ROLE)?,
            object_evidence_contract_ref.clone(),
        )?;
        verify_admitted_member(
            support_graph,
            EXECUTABLE_IDENTITY_PATH,
            executable_identity_ref,
            None,
            &executable_contract,
        )?;

        let qualification_contract = retained_support_contract(
            shared_qualification_ref.schema_id().clone(),
            "mfm.product",
            "component-qualification",
            stable_id(SHARED_QUALIFICATION_ROLE)?,
            object_evidence_contract_ref.clone(),
        )?;
        let qualification_bytes = verify_admitted_member(
            support_graph,
            SHARED_QUALIFICATION_PATH,
            shared_qualification_ref,
            None,
            &qualification_contract,
        )?;

        let capability_contract_ref = evm_read_capability_contract_ref()?;
        let callback_surface_ref = evm_adapter_callback_surface_ref()?;
        let adapter_implementation = ComponentImplementationDescriptor::new(
            ComponentKind::ReadCapabilityAdapterVerifier,
            capability_contract_ref.clone(),
            callback_surface_ref.clone(),
            shared_qualification_ref.clone(),
        )?;
        verify_shared_qualification(
            qualification_bytes,
            executable_identity_ref,
            &capability_contract_ref,
            &callback_surface_ref,
        )?;

        let semantic_contract = mfm_evm::evm_read_capability_support_contract(
            stable_id(EVM_ADAPTER_SEMANTIC_ROLE)?,
            object_evidence_contract_ref.clone(),
        )?;
        verify_admitted_member(
            support_graph,
            EVM_ADAPTER_SEMANTIC_PATH,
            &capability_contract_ref,
            Some(evm_read_capability_contract_canonical()?.as_bytes()),
            &semantic_contract,
        )?;

        let callback_contract = evm_adapter_callback_surface_support_contract(
            stable_id(EVM_ADAPTER_CALLBACK_ROLE)?,
            object_evidence_contract_ref.clone(),
        )?;
        verify_admitted_member(
            support_graph,
            EVM_ADAPTER_CALLBACK_PATH,
            &callback_surface_ref,
            Some(evm_adapter_callback_surface_canonical()?.as_bytes()),
            &callback_contract,
        )?;

        let implementation_ref = adapter_implementation.content_ref()?;
        let implementation_contract = retained_support_contract(
            implementation_ref.schema_id().clone(),
            "mfm.product",
            "component-implementation-descriptor",
            stable_id(EVM_ADAPTER_IMPLEMENTATION_ROLE)?,
            object_evidence_contract_ref.clone(),
        )?;
        verify_admitted_member(
            support_graph,
            EVM_ADAPTER_IMPLEMENTATION_PATH,
            &implementation_ref,
            Some(adapter_implementation.canonical_json()?.as_bytes()),
            &implementation_contract,
        )?;

        Ok(Self {
            support_graph,
            adapter_implementation,
            object_evidence_contract_ref: object_evidence_contract_ref.clone(),
        })
    }

    /// Returns the exact adapter implementation selected by the shared qualification.
    pub const fn adapter_implementation(&self) -> &ComponentImplementationDescriptor {
        &self.adapter_implementation
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SharedQualification {
    version: String,
    executable_identity_ref: ContentRef,
    #[serde(rename = "qualification_profile_ref")]
    _qualification_profile_ref: ContentRef,
    components: Vec<QualifiedComponent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct QualifiedComponent {
    component_kind: ComponentKind,
    semantic_contract_ref: ContentRef,
    callback_surface_ref: ContentRef,
}

fn evm_safe_classifier() -> mfm_runtime::Result<SafeFailureClassifierDescriptor> {
    let none = SafeFailureSizeRule::None;
    let from_failure = SafeFailureSizeRule::FromFailure;
    let forbidden = || SafeFailureDiagnosticRule::Forbidden;
    let required = |diagnostic: &'static str, kinds: &[&'static str]| {
        let mut constraints = vec![SafeFailureDiagnosticConstraint::new(
            FieldPath::new("diagnostic").map_err(|_| RuntimeError::CatalogSelection)?,
            vec![diagnostic.to_owned()],
        )
        .map_err(|_| RuntimeError::CatalogSelection)?];
        if !kinds.is_empty() {
            constraints.push(
                SafeFailureDiagnosticConstraint::new(
                    FieldPath::new("kind").map_err(|_| RuntimeError::CatalogSelection)?,
                    kinds.iter().map(|kind| (*kind).to_owned()).collect(),
                )
                .map_err(|_| RuntimeError::CatalogSelection)?,
            );
        }
        SafeFailureDiagnosticRule::required(constraints).map_err(|_| RuntimeError::CatalogSelection)
    };
    let rules = vec![
        SafeFailureClassifierRule::new(
            stable_id("access_cancelled")?,
            SafeFailureOutcome::DidNotEnter,
            FailureClass::Cancellation,
            BoundaryStage::BeforeBoundaryEntry,
            none,
            forbidden(),
        ),
        SafeFailureClassifierRule::new(
            stable_id("access_cancelled")?,
            SafeFailureOutcome::Indeterminate,
            FailureClass::Cancellation,
            BoundaryStage::BoundaryEntry,
            none,
            forbidden(),
        ),
        SafeFailureClassifierRule::new(
            stable_id("configuration_invalid")?,
            SafeFailureOutcome::DidNotEnter,
            FailureClass::Configuration,
            BoundaryStage::BeforeBoundaryEntry,
            none,
            forbidden(),
        ),
        SafeFailureClassifierRule::new(
            stable_id("http_status")?,
            SafeFailureOutcome::Indeterminate,
            FailureClass::Destination,
            BoundaryStage::BoundaryObservation,
            none,
            required("http_status", &[])?,
        ),
        SafeFailureClassifierRule::new(
            stable_id("json_rpc_error")?,
            SafeFailureOutcome::Indeterminate,
            FailureClass::Destination,
            BoundaryStage::BoundaryObservation,
            none,
            required("json_rpc_error", &[])?,
        ),
        SafeFailureClassifierRule::new(
            stable_id("request_invalid")?,
            SafeFailureOutcome::DidNotEnter,
            FailureClass::Request,
            BoundaryStage::BeforeBoundaryEntry,
            none,
            forbidden(),
        ),
        SafeFailureClassifierRule::new(
            stable_id("response_invalid")?,
            SafeFailureOutcome::Indeterminate,
            FailureClass::UnrepresentableResponse,
            BoundaryStage::BoundaryObservation,
            from_failure,
            required(
                "response_invalid",
                &["malformed_envelope", "invalid_result"],
            )?,
        ),
        SafeFailureClassifierRule::new(
            stable_id("response_missing_result")?,
            SafeFailureOutcome::Indeterminate,
            FailureClass::UnrepresentableResponse,
            BoundaryStage::BoundaryObservation,
            from_failure,
            required("response_invalid", &["missing_result"])?,
        ),
        SafeFailureClassifierRule::new(
            stable_id("response_too_large")?,
            SafeFailureOutcome::Indeterminate,
            FailureClass::UnrepresentableResponse,
            BoundaryStage::BoundaryObservation,
            from_failure,
            required("response_invalid", &["too_large"])?,
        ),
        SafeFailureClassifierRule::new(
            stable_id("routing_generation_unavailable")?,
            SafeFailureOutcome::DidNotEnter,
            FailureClass::Authorization,
            BoundaryStage::BeforeBoundaryEntry,
            none,
            forbidden(),
        ),
        SafeFailureClassifierRule::new(
            stable_id("transport_failed")?,
            SafeFailureOutcome::DidNotEnter,
            FailureClass::Transport,
            BoundaryStage::BeforeBoundaryEntry,
            none,
            forbidden(),
        ),
        SafeFailureClassifierRule::new(
            stable_id("transport_failed")?,
            SafeFailureOutcome::Indeterminate,
            FailureClass::Transport,
            BoundaryStage::BoundaryEntry,
            none,
            forbidden(),
        ),
        SafeFailureClassifierRule::new(
            stable_id("unclassified_failure")?,
            SafeFailureOutcome::Indeterminate,
            FailureClass::Unclassified,
            BoundaryStage::BoundaryObservation,
            none,
            forbidden(),
        ),
    ];
    SafeFailureClassifierDescriptor::new(
        evm_safe_failure_contract_ref()?,
        Some(
            EvmSafeDiagnostic::schema_descriptor()
                .map_err(|_| RuntimeError::CatalogSelection)?
                .identity,
        ),
        rules,
    )
    .map_err(|_| RuntimeError::CatalogSelection)
}

fn stable_id(raw: &'static str) -> mfm_program::Result<StableId> {
    StableId::new(raw).map_err(|error| mfm_program::ProgramError::Spec(error.to_string()))
}

const fn coarse_size(size: EvmCoarseSizeClass) -> CoarseSizeClass {
    match size {
        EvmCoarseSizeClass::Zero => CoarseSizeClass::Zero,
        EvmCoarseSizeClass::UpTo16Kib => CoarseSizeClass::UpTo16Kib,
        EvmCoarseSizeClass::UpTo1Mib => CoarseSizeClass::UpTo1Mib,
        EvmCoarseSizeClass::Over1Mib => CoarseSizeClass::Over1Mib,
    }
}

#[derive(Clone)]
struct EvmSafeFailureClassifier {
    descriptor: SafeFailureClassifierDescriptor,
    routing_generation_unavailable: StableId,
    configuration_invalid: StableId,
    request_invalid: StableId,
    access_cancelled: StableId,
    transport_failed: StableId,
    http_status: StableId,
    json_rpc_error: StableId,
    response_invalid: StableId,
    response_missing_result: StableId,
    response_too_large: StableId,
    unclassified_failure: StableId,
    unclassified_metadata: SafeFailureMetadata,
}

impl EvmSafeFailureClassifier {
    fn new() -> mfm_runtime::Result<Self> {
        let descriptor = evm_safe_classifier()?;
        let routing_generation_unavailable = stable_id("routing_generation_unavailable")?;
        let configuration_invalid = stable_id("configuration_invalid")?;
        let request_invalid = stable_id("request_invalid")?;
        let access_cancelled = stable_id("access_cancelled")?;
        let transport_failed = stable_id("transport_failed")?;
        let http_status = stable_id("http_status")?;
        let json_rpc_error = stable_id("json_rpc_error")?;
        let response_invalid = stable_id("response_invalid")?;
        let response_missing_result = stable_id("response_missing_result")?;
        let response_too_large = stable_id("response_too_large")?;
        let unclassified_failure = stable_id("unclassified_failure")?;
        let fallback = descriptor
            .classify(
                &unclassified_failure,
                SafeFailureOutcome::Indeterminate,
                None,
                false,
            )
            .map_err(|_| RuntimeError::CatalogSelection)?;
        let unclassified_metadata = SafeFailureMetadata::new(
            descriptor.safe_failure_contract_ref().clone(),
            unclassified_failure.clone(),
            fallback.failure_class(),
            fallback.boundary_stage(),
            None,
        );
        Ok(Self {
            descriptor,
            routing_generation_unavailable,
            configuration_invalid,
            request_invalid,
            access_cancelled,
            transport_failed,
            http_status,
            json_rpc_error,
            response_invalid,
            response_missing_result,
            response_too_large,
            unclassified_failure,
            unclassified_metadata,
        })
    }

    fn projection<'a>(
        &'a self,
        failure: &EvmSafeFailure,
    ) -> (
        &'a StableId,
        Option<CoarseSizeClass>,
        Option<EvmSafeDiagnostic>,
    ) {
        match failure {
            EvmSafeFailure::RoutingGenerationUnavailable => {
                (&self.routing_generation_unavailable, None, None)
            }
            EvmSafeFailure::ConfigurationInvalid => (&self.configuration_invalid, None, None),
            EvmSafeFailure::RequestInvalid => (&self.request_invalid, None, None),
            EvmSafeFailure::AccessCancelled => (&self.access_cancelled, None, None),
            EvmSafeFailure::TransportFailed => (&self.transport_failed, None, None),
            EvmSafeFailure::HttpStatus { status } => (
                &self.http_status,
                None,
                Some(EvmSafeDiagnostic::HttpStatus { status: *status }),
            ),
            EvmSafeFailure::JsonRpcError { json_rpc_code } => (
                &self.json_rpc_error,
                None,
                Some(EvmSafeDiagnostic::JsonRpcError {
                    code: *json_rpc_code,
                }),
            ),
            EvmSafeFailure::ResponseInvalid {
                response_kind,
                size_class,
            } => (
                &self.response_invalid,
                Some(coarse_size(*size_class)),
                Some(EvmSafeDiagnostic::ResponseInvalid {
                    kind: *response_kind,
                }),
            ),
            EvmSafeFailure::ResponseMissingResult { size_class } => (
                &self.response_missing_result,
                Some(coarse_size(*size_class)),
                Some(EvmSafeDiagnostic::ResponseInvalid {
                    kind: EvmResponseInvalidKind::MissingResult,
                }),
            ),
            EvmSafeFailure::ResponseTooLarge { size_class } => (
                &self.response_too_large,
                Some(coarse_size(*size_class)),
                Some(EvmSafeDiagnostic::ResponseInvalid {
                    kind: EvmResponseInvalidKind::TooLarge,
                }),
            ),
            EvmSafeFailure::UnclassifiedFailure => (&self.unclassified_failure, None, None),
        }
    }

    fn metadata(
        &self,
        failure: &EvmSafeFailure,
        outcome: SafeFailureOutcome,
    ) -> Option<(SafeFailureMetadata, Option<EvmSafeDiagnostic>)> {
        let (code, size, diagnostic) = self.projection(failure);
        let rule = self
            .descriptor
            .classify(code, outcome, size, diagnostic.is_some())
            .ok()?;
        Some((
            SafeFailureMetadata::new(
                self.descriptor.safe_failure_contract_ref().clone(),
                code.clone(),
                rule.failure_class(),
                rule.boundary_stage(),
                size,
            ),
            diagnostic,
        ))
    }

    fn unclassified_outcome<R>(&self) -> ReadCapabilityOutcome<R, EvmSafeDiagnostic> {
        ReadCapabilityOutcome::Indeterminate {
            diagnostic: None,
            metadata: self.unclassified_metadata.clone(),
        }
    }
}

fn runtime_outcome<R>(
    outcome: EvmTransportOutcome<R>,
    classifier: &EvmSafeFailureClassifier,
) -> ReadCapabilityOutcome<R, EvmSafeDiagnostic> {
    let (failure, actual_outcome) = match outcome {
        EvmTransportOutcome::Returned(response) => {
            return ReadCapabilityOutcome::Returned(response);
        }
        EvmTransportOutcome::DidNotEnter(failure) => (failure, SafeFailureOutcome::DidNotEnter),
        EvmTransportOutcome::Indeterminate(failure) => (failure, SafeFailureOutcome::Indeterminate),
    };
    let Some((metadata, diagnostic)) = classifier.metadata(&failure, actual_outcome) else {
        return classifier.unclassified_outcome();
    };
    match actual_outcome {
        SafeFailureOutcome::DidNotEnter => ReadCapabilityOutcome::DidNotEnter {
            diagnostic,
            metadata,
        },
        SafeFailureOutcome::Indeterminate => ReadCapabilityOutcome::Indeterminate {
            diagnostic,
            metadata,
        },
    }
}

/// One aggregate adapter implementing the closed six-operation EVM read table.
pub struct EvmReadAdapter {
    transport: Arc<EvmJsonRpcTransport>,
    safe_classifier: EvmSafeFailureClassifier,
}

macro_rules! impl_read_capability {
    ($request:ty, $response:ty, $method:ident, $routing_generation:expr) => {
        impl AuditedReadCapability<$request> for EvmReadAdapter {
            type Response = $response;
            type SafeDiagnostic = EvmSafeDiagnostic;

            fn routing_generation_ref(&self, request: &$request) -> Option<ContentRef> {
                $routing_generation(request)
            }

            fn call<'a>(
                &'a self,
                access: AuthorizedReadAccess<$request>,
            ) -> ReadCapabilityFuture<'a, Self::Response, Self::SafeDiagnostic> {
                Box::pin(async move {
                    runtime_outcome(
                        self.transport.$method(access.request()).await,
                        &self.safe_classifier,
                    )
                })
            }
        }
    };
}

impl_read_capability!(
    EvmChainIdentityRequest,
    EvmChainIdentityResponse,
    chain_identity,
    |request: &EvmChainIdentityRequest| {
        request
            .binding()
            .routing_generation_ref()
            .to_content_ref()
            .ok()
    }
);
impl_read_capability!(
    EvmLatestAnchorRequest,
    EvmBlockResponse,
    latest_anchor,
    |request: &EvmLatestAnchorRequest| {
        request
            .source()
            .binding()
            .routing_generation_ref()
            .to_content_ref()
            .ok()
    }
);
impl_read_capability!(
    EvmTokenDecimalsRequest,
    EvmTokenDecimalsResponse,
    token_decimals,
    |request: &EvmTokenDecimalsRequest| {
        request
            .source()
            .source()
            .binding()
            .routing_generation_ref()
            .to_content_ref()
            .ok()
    }
);
impl_read_capability!(
    EvmNativeBalanceRequest,
    EvmQuantityResponse,
    native_balance,
    |request: &EvmNativeBalanceRequest| {
        request
            .source()
            .source()
            .binding()
            .routing_generation_ref()
            .to_content_ref()
            .ok()
    }
);
impl_read_capability!(
    EvmTokenBalanceRequest,
    EvmQuantityResponse,
    token_balance,
    |request: &EvmTokenBalanceRequest| {
        request
            .source()
            .source()
            .binding()
            .routing_generation_ref()
            .to_content_ref()
            .ok()
    }
);
impl_read_capability!(
    EvmAnchorConfirmationRequest,
    EvmBlockResponse,
    confirm_anchor,
    |request: &EvmAnchorConfirmationRequest| {
        request
            .source()?
            .source()
            .binding()
            .routing_generation_ref()
            .to_content_ref()
            .ok()
    }
);

/// Closed qualified entry bundle for the six exact EVM read operations.
pub struct QualifiedEvmReadEntries {
    chain_identity: QualifiedReadEntry,
    latest_anchor: QualifiedReadEntry,
    token_decimals: QualifiedReadEntry,
    native_balance: QualifiedReadEntry,
    token_balance: QualifiedReadEntry,
    confirm_anchor: QualifiedReadEntry,
}

impl QualifiedEvmReadEntries {
    /// Registers the closed operation inventory into the sole program registry.
    #[allow(clippy::result_large_err)]
    pub fn register_into(
        self,
        registry: &mut QualifiedProgramRegistryBuilder,
    ) -> mfm_program::Result<&mut QualifiedProgramRegistryBuilder> {
        registry
            .register_read(self.chain_identity)?
            .register_read(self.latest_anchor)?
            .register_read(self.token_decimals)?
            .register_read(self.native_balance)?
            .register_read(self.token_balance)?
            .register_read(self.confirm_anchor)
    }
}

/// Qualifies six typed read entries against one sealed adapter qualification.
#[allow(clippy::result_large_err)]
pub fn qualify_evm_read_entries(
    binding: ReadCapabilityBinding,
    qualification: &EvmReadQualificationArtifacts<'_>,
    transport: Arc<EvmJsonRpcTransport>,
) -> mfm_runtime::Result<QualifiedEvmReadEntries> {
    let support_graph = qualification.support_graph;
    let fields = binding.fields()?;
    let capability_contract_ref = evm_read_capability_contract_ref()?;
    let safe_failure_contract_ref = evm_safe_failure_contract_ref()?;
    let safe_classifier_contract_ref = evm_safe_classifier_contract_ref()?;
    if fields.capability_contract_ref != capability_contract_ref
        || fields.safe_failure_contract_ref != safe_failure_contract_ref
        || fields.safe_classifier_contract_ref != safe_classifier_contract_ref
    {
        return Err(RuntimeError::CatalogSelection);
    }
    verify_support_bytes(support_graph, &binding.content_ref()?, binding.as_bytes())?;
    verify_support_bytes(
        support_graph,
        &safe_failure_contract_ref,
        evm_safe_failure_contract_canonical()?.as_bytes(),
    )?;
    verify_support_bytes(
        support_graph,
        &safe_classifier_contract_ref,
        evm_safe_classifier_canonical()?.as_bytes(),
    )?;
    verify_transport_support(support_graph, &fields.routing_catalog_ref, &transport)?;

    let implementation = qualification.adapter_implementation();
    if implementation.semantic_contract_ref() != &capability_contract_ref
        || implementation.content_ref()? != fields.admitted_implementation_ref
        || support_bytes(support_graph, &fields.reviewed_source_scope_ref).is_none()
    {
        return Err(RuntimeError::CatalogSelection);
    }

    let contracts = evm_read_value_contracts(qualification.object_evidence_contract_ref.clone())?;
    let allowed_routing_generations = transport
        .routing_generation_descriptors()
        .map(|(generation, _)| {
            generation
                .to_content_ref()
                .map_err(|_| RuntimeError::CatalogSelection)
        })
        .collect::<mfm_runtime::Result<Vec<_>>>()?;
    let safe_classifier = EvmSafeFailureClassifier::new()?;
    let operation = |operation_id: &'static str,
                     request_contract: RetainedValueContract,
                     returned_contract: RetainedValueContract|
     -> mfm_runtime::Result<QualifiedReadOperationContract> {
        QualifiedReadOperationContract::new(
            stable_id(operation_id)?,
            capability_contract_ref.clone(),
            request_contract,
            returned_contract,
            contracts.safe_failure().clone(),
            allowed_routing_generations.clone(),
        )
        .map_err(RuntimeError::from)
    };
    let adapter = |transport| EvmReadAdapter {
        transport,
        safe_classifier: safe_classifier.clone(),
    };
    Ok(QualifiedEvmReadEntries {
        chain_identity: qualify_read_capability::<EvmChainIdentityRequest, _>(
            binding.clone(),
            operation(
                EVM_CHAIN_ID_OPERATION_ID,
                contracts.chain_identity_request().clone(),
                contracts.chain_identity_response().clone(),
            )?,
            implementation.clone(),
            adapter(Arc::clone(&transport)),
        )?,
        latest_anchor: qualify_read_capability::<EvmLatestAnchorRequest, _>(
            binding.clone(),
            operation(
                EVM_LATEST_ANCHOR_OPERATION_ID,
                contracts.latest_anchor_request().clone(),
                contracts.block_response().clone(),
            )?,
            implementation.clone(),
            adapter(Arc::clone(&transport)),
        )?,
        token_decimals: qualify_read_capability::<EvmTokenDecimalsRequest, _>(
            binding.clone(),
            operation(
                EVM_TOKEN_DECIMALS_OPERATION_ID,
                contracts.token_decimals_request().clone(),
                contracts.token_decimals_response().clone(),
            )?,
            implementation.clone(),
            adapter(Arc::clone(&transport)),
        )?,
        native_balance: qualify_read_capability::<EvmNativeBalanceRequest, _>(
            binding.clone(),
            operation(
                EVM_NATIVE_BALANCE_OPERATION_ID,
                contracts.native_balance_request().clone(),
                contracts.quantity_response().clone(),
            )?,
            implementation.clone(),
            adapter(Arc::clone(&transport)),
        )?,
        token_balance: qualify_read_capability::<EvmTokenBalanceRequest, _>(
            binding.clone(),
            operation(
                EVM_TOKEN_BALANCE_OPERATION_ID,
                contracts.token_balance_request().clone(),
                contracts.quantity_response().clone(),
            )?,
            implementation.clone(),
            adapter(Arc::clone(&transport)),
        )?,
        confirm_anchor: qualify_read_capability::<EvmAnchorConfirmationRequest, _>(
            binding,
            operation(
                EVM_CONFIRM_ANCHOR_OPERATION_ID,
                contracts.anchor_confirmation_request().clone(),
                contracts.block_response().clone(),
            )?,
            implementation.clone(),
            adapter(transport),
        )?,
    })
}

fn verify_transport_support(
    support_graph: &AdmittedSupportGraph,
    routing_catalog_ref: &ContentRef,
    transport: &EvmJsonRpcTransport,
) -> mfm_runtime::Result<()> {
    let catalog = transport.routing_catalog_descriptor();
    if &catalog
        .content_ref()
        .map_err(|_| RuntimeError::CatalogSelection)?
        != routing_catalog_ref
    {
        return Err(RuntimeError::CatalogSelection);
    }
    verify_support_bytes(
        support_graph,
        routing_catalog_ref,
        catalog
            .canonical()
            .map_err(|_| RuntimeError::CatalogSelection)?
            .as_bytes(),
    )?;
    for (generation_ref, descriptor) in transport.routing_generation_descriptors() {
        let reference = generation_ref
            .to_content_ref()
            .map_err(|_| RuntimeError::CatalogSelection)?;
        if descriptor
            .content_ref()
            .map_err(|_| RuntimeError::CatalogSelection)?
            != reference
        {
            return Err(RuntimeError::CatalogSelection);
        }
        verify_support_bytes(
            support_graph,
            &reference,
            descriptor
                .canonical()
                .map_err(|_| RuntimeError::CatalogSelection)?
                .as_bytes(),
        )?;
    }
    Ok(())
}

fn verify_shared_qualification(
    bytes: &[u8],
    executable_identity_ref: &ContentRef,
    capability_contract_ref: &ContentRef,
    callback_surface_ref: &ContentRef,
) -> mfm_runtime::Result<()> {
    let qualification = serde_json::from_slice::<SharedQualification>(bytes)
        .map_err(|_| RuntimeError::CatalogSelection)?;
    let expected_adapter = QualifiedComponent {
        component_kind: ComponentKind::ReadCapabilityAdapterVerifier,
        semantic_contract_ref: capability_contract_ref.clone(),
        callback_surface_ref: callback_surface_ref.clone(),
    };
    let exact_kinds = qualification
        .components
        .iter()
        .filter(|component| component.component_kind == ComponentKind::Planner)
        .count()
        == 1
        && qualification
            .components
            .iter()
            .filter(|component| component.component_kind == ComponentKind::State)
            .count()
            == 10
        && qualification
            .components
            .iter()
            .filter(|component| {
                component.component_kind == ComponentKind::ReadCapabilityAdapterVerifier
            })
            .count()
            == 1
        && qualification
            .components
            .iter()
            .all(|component| component.component_kind != ComponentKind::ExecutorClientVerifier);
    let canonical_component_order =
        qualification_components_are_canonical(&qualification.components)?;
    if qualification.version != SHARED_QUALIFICATION_VERSION
        || &qualification.executable_identity_ref != executable_identity_ref
        || qualification.components.len() != SHARED_QUALIFICATION_COMPONENT_COUNT
        || !canonical_component_order
        || !exact_kinds
        || qualification
            .components
            .iter()
            .filter(|component| *component == &expected_adapter)
            .count()
            != 1
    {
        return Err(RuntimeError::CatalogSelection);
    }
    Ok(())
}

fn qualification_components_are_canonical(
    components: &[QualifiedComponent],
) -> mfm_runtime::Result<bool> {
    let mut previous = None;
    for component in components {
        let canonical = canonical_json(component)?;
        if previous
            .as_ref()
            .is_some_and(|value: &PlainCanonicalJsonBytes| value.as_bytes() >= canonical.as_bytes())
        {
            return Ok(false);
        }
        previous = Some(canonical);
    }
    Ok(true)
}

fn verify_admitted_member<'a>(
    support_graph: &'a AdmittedSupportGraph,
    path: &'static str,
    expected_ref: &ContentRef,
    expected_bytes: Option<&[u8]>,
    expected_contract: &RetainedValueContract,
) -> mfm_runtime::Result<&'a [u8]> {
    let member = support_graph
        .member(&field_path(path)?)
        .ok_or(RuntimeError::CatalogSelection)?;
    member.value_ref().validate_contract(expected_contract)?;
    if &admitted_member_content_ref(member)? != expected_ref
        || expected_bytes.is_some_and(|expected| member.bytes() != expected)
    {
        return Err(RuntimeError::CatalogSelection);
    }
    Ok(member.bytes())
}

fn admitted_member_content_ref(member: &AdmittedSupportMember) -> mfm_runtime::Result<ContentRef> {
    let fields = member.value_ref().fields()?;
    ContentRef::new(fields.schema_id, fields.content_digest).map_err(Into::into)
}

fn verify_support_bytes(
    support_graph: &AdmittedSupportGraph,
    reference: &ContentRef,
    expected: &[u8],
) -> mfm_runtime::Result<()> {
    if support_bytes(support_graph, reference) == Some(expected) {
        Ok(())
    } else {
        Err(RuntimeError::CatalogSelection)
    }
}

fn support_bytes<'a>(
    support_graph: &'a AdmittedSupportGraph,
    reference: &ContentRef,
) -> Option<&'a [u8]> {
    support_graph
        .members()
        .values()
        .find(|member| {
            admitted_member_content_ref(member)
                .as_ref()
                .is_ok_and(|content_ref| content_ref == reference)
        })
        .map(AdmittedSupportMember::bytes)
}

fn canonical_json<T: Serialize>(value: &T) -> mfm_runtime::Result<PlainCanonicalJsonBytes> {
    let json = serde_json::to_string(value).map_err(|_| RuntimeError::CatalogSelection)?;
    PlainCanonicalJsonBytes::from_json_str(&json).map_err(|_| RuntimeError::CatalogSelection)
}

fn descriptor_schema_id(name: &'static str) -> mfm_runtime::Result<SchemaId> {
    SchemaId::new(
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("schema:{name}:1").as_bytes()),
    )
    .map_err(|_| RuntimeError::CatalogSelection)
}

fn descriptor_support_contract(
    schema_name: &'static str,
    semantic_name: &'static str,
    role: StableId,
    evidence_contract_ref: ContentRef,
) -> mfm_runtime::Result<RetainedValueContract> {
    retained_support_contract(
        descriptor_schema_id(schema_name)?,
        "mfm.evm-live",
        semantic_name,
        role,
        evidence_contract_ref,
    )
}

fn retained_support_contract(
    schema_id: SchemaId,
    semantic_namespace: &str,
    semantic_name: &str,
    role: StableId,
    evidence_contract_ref: ContentRef,
) -> mfm_runtime::Result<RetainedValueContract> {
    let semantic_type_id = SemanticTypeId::new(
        semantic_namespace,
        semantic_name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("semantic:{semantic_namespace}:{semantic_name}:1").as_bytes()),
    )
    .map_err(|_| RuntimeError::CatalogSelection)?;
    RetainedValueContract::new(
        schema_id,
        semantic_type_id,
        role,
        "application/json",
        evidence_contract_ref,
    )
    .map_err(|_| RuntimeError::CatalogSelection)
}

fn field_path(path: &'static str) -> mfm_runtime::Result<FieldPath> {
    FieldPath::new(path).map_err(|_| RuntimeError::CatalogSelection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{
        routing_catalog_descriptor_support_contract,
        routing_generation_descriptor_support_contract, EvmRoutingCatalogBuilder, EvmRpcEndpoint,
    };
    use alloy_primitives::{Address, B256, U256};
    use mfm_ids::{FieldPath, StoreEpoch, StoreScopeId};
    use mfm_store::{
        open_in_memory, AdmittedSupportGraph, QualifiedSupportGraph, QualifiedSupportMember,
        StoreIdentity,
    };

    fn test_schema(name: &str) -> SchemaId {
        SchemaId::new(
            &format!("mfm.test.evm-live.{name}"),
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(format!("schema:mfm.test.evm-live:{name}:1").as_bytes()),
        )
        .expect("test schema")
    }

    fn test_semantic(name: &str) -> SemanticTypeId {
        SemanticTypeId::new(
            "mfm.test.evm-live",
            name,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(format!("semantic:mfm.test.evm-live:{name}:1").as_bytes()),
        )
        .expect("test semantic type")
    }

    fn support_contract(
        schema_id: SchemaId,
        semantic_name: &str,
        evidence_ref: ContentRef,
    ) -> RetainedValueContract {
        RetainedValueContract::new(
            schema_id,
            test_semantic(semantic_name),
            StableId::new("mfm.test.evm-live.support").expect("support role"),
            "application/json",
            evidence_ref,
        )
        .expect("support contract")
    }

    fn support_object(
        path: &str,
        canonical: PlainCanonicalJsonBytes,
        contract: RetainedValueContract,
    ) -> QualifiedSupportMember {
        QualifiedSupportMember::new(
            FieldPath::new(path).expect("support path"),
            canonical,
            contract,
        )
    }

    fn arbitrary_object(
        path: &str,
        name: &str,
        canonical: PlainCanonicalJsonBytes,
        evidence_ref: &ContentRef,
    ) -> QualifiedSupportMember {
        support_object(
            path,
            canonical,
            support_contract(test_schema(name), name, evidence_ref.clone()),
        )
    }

    fn support_ref(member: &QualifiedSupportMember) -> ContentRef {
        boundary_content_ref(
            member.value_contract().schema_id().clone(),
            member.canonical(),
        )
        .expect("support ref")
    }

    fn test_ref(name: &str) -> ContentRef {
        let canonical = PlainCanonicalJsonBytes::from_json_str(&format!(
            r#"{{"version":"mfm.test.evm-live.{name}.v1"}}"#
        ))
        .expect("test ref bytes");
        boundary_content_ref(test_schema(name), &canonical).expect("test content ref")
    }

    fn app_shaped_qualification_components(
        capability_ref: &ContentRef,
        callback_ref: &ContentRef,
    ) -> Vec<QualifiedComponent> {
        let mut components = vec![QualifiedComponent {
            component_kind: ComponentKind::Planner,
            semantic_contract_ref: test_ref("planner-semantic"),
            callback_surface_ref: test_ref("planner-callback"),
        }];
        components.extend((0..10).map(|index| QualifiedComponent {
            component_kind: ComponentKind::State,
            semantic_contract_ref: test_ref(&format!("state-{index}-semantic")),
            callback_surface_ref: test_ref(&format!("state-{index}-callback")),
        }));
        components.push(QualifiedComponent {
            component_kind: ComponentKind::ReadCapabilityAdapterVerifier,
            semantic_contract_ref: capability_ref.clone(),
            callback_surface_ref: callback_ref.clone(),
        });
        components.sort_by_cached_key(|component| {
            canonical_json(component)
                .expect("component canonical JSON")
                .as_bytes()
                .to_vec()
        });
        components
    }

    async fn qualified_factory_fixture(
        include_route_generation: bool,
    ) -> (
        ReadCapabilityBinding,
        AdmittedSupportGraph,
        ContentRef,
        ContentRef,
        ContentRef,
        Arc<EvmJsonRpcTransport>,
    ) {
        let evidence_canonical =
            mfm_values::component_object_evidence_contract_canonical().expect("evidence bytes");
        let evidence_ref =
            mfm_values::component_object_evidence_contract_ref().expect("evidence ref");
        let evidence = support_object(
            OBJECT_EVIDENCE_PATH,
            evidence_canonical,
            retained_support_contract(
                evidence_ref.schema_id().clone(),
                "mfm.product",
                "component-object-evidence-contract",
                stable_id(OBJECT_EVIDENCE_ROLE).expect("evidence role"),
                evidence_ref.clone(),
            )
            .expect("evidence contract"),
        );

        let executable_canonical = PlainCanonicalJsonBytes::from_json_str(
            r#"{"contract":"mfm.executable-bytes.v1","sha256":"0000000000000000000000000000000000000000000000000000000000000000"}"#,
        )
        .expect("executable bytes");
        let executable_schema =
            mfm_spec::schema_id("mfm.executable-bytes-descriptor.v1").expect("executable schema");
        let executable_ref = boundary_content_ref(executable_schema.clone(), &executable_canonical)
            .expect("executable ref");
        let executable = support_object(
            EXECUTABLE_IDENTITY_PATH,
            executable_canonical,
            retained_support_contract(
                executable_schema,
                "mfm.recoverability",
                "executable-bytes-descriptor",
                stable_id(EXECUTABLE_IDENTITY_ROLE).expect("executable role"),
                evidence_ref.clone(),
            )
            .expect("executable contract"),
        );

        let reviewed_source = arbitrary_object(
            "source/reviewed",
            "reviewed-source",
            PlainCanonicalJsonBytes::from_json_str(
                r#"{"source_ref":"primary","version":"mfm.test.reviewed-source.v1"}"#,
            )
            .expect("source bytes"),
            &evidence_ref,
        );

        let mut routes = EvmRoutingCatalogBuilder::new();
        routes
            .insert(
                "ethereum-mainnet",
                "primary",
                1,
                StableId::new("generation-0001").expect("generation id"),
                EvmRpcEndpoint::new("http://127.0.0.1:9").expect("endpoint"),
                None,
            )
            .expect("generation");
        let transport = Arc::new(
            EvmJsonRpcTransport::new(routes.build().expect("catalog")).expect("transport"),
        );
        let catalog = transport.routing_catalog_descriptor();
        let catalog_canonical = catalog.canonical().expect("catalog bytes");
        let catalog_ref = catalog.content_ref().expect("catalog ref");
        let catalog_object = support_object(
            "routing/catalog",
            catalog_canonical,
            routing_catalog_descriptor_support_contract(
                StableId::new("mfm.test.evm-live.routing-catalog").expect("catalog role"),
                evidence_ref.clone(),
            )
            .expect("catalog contract"),
        );
        let generation_objects = transport
            .routing_generation_descriptors()
            .enumerate()
            .map(|(index, (_, descriptor))| {
                support_object(
                    &format!("routing/generation/{index:04}"),
                    descriptor.canonical().expect("generation bytes"),
                    routing_generation_descriptor_support_contract(
                        StableId::new(format!("mfm.test.evm-live.routing-generation-{index:04}"))
                            .expect("generation role"),
                        evidence_ref.clone(),
                    )
                    .expect("generation contract"),
                )
            })
            .collect::<Vec<_>>();

        let capability_ref = evm_read_capability_contract_ref().expect("capability ref");
        let capability_object = support_object(
            EVM_ADAPTER_SEMANTIC_PATH,
            evm_read_capability_contract_canonical().expect("capability bytes"),
            mfm_evm::evm_read_capability_support_contract(
                stable_id(EVM_ADAPTER_SEMANTIC_ROLE).expect("capability role"),
                evidence_ref.clone(),
            )
            .expect("capability contract"),
        );
        let failure_ref = evm_safe_failure_contract_ref().expect("failure ref");
        let failure_object = support_object(
            "contracts/safe-failure",
            evm_safe_failure_contract_canonical().expect("failure bytes"),
            mfm_evm::evm_safe_failure_support_contract(
                StableId::new("mfm.test.evm-live.safe-failure").expect("failure role"),
                evidence_ref.clone(),
            )
            .expect("failure contract"),
        );
        let classifier_ref = evm_safe_classifier_contract_ref().expect("classifier ref");
        let classifier_object = support_object(
            "contracts/safe-classifier",
            evm_safe_classifier_canonical().expect("classifier bytes"),
            evm_safe_classifier_support_contract(
                StableId::new("mfm.test.evm-live.safe-classifier").expect("classifier role"),
                evidence_ref.clone(),
            )
            .expect("classifier contract"),
        );
        let callback_ref = evm_adapter_callback_surface_ref().expect("callback ref");
        let callback_object = support_object(
            EVM_ADAPTER_CALLBACK_PATH,
            evm_adapter_callback_surface_canonical().expect("callback bytes"),
            evm_adapter_callback_surface_support_contract(
                stable_id(EVM_ADAPTER_CALLBACK_ROLE).expect("callback role"),
                evidence_ref.clone(),
            )
            .expect("callback contract"),
        );

        let components = app_shaped_qualification_components(&capability_ref, &callback_ref);
        let qualification_canonical = canonical_json(&SharedQualification {
            version: SHARED_QUALIFICATION_VERSION.to_owned(),
            executable_identity_ref: executable_ref.clone(),
            _qualification_profile_ref: test_ref("qualification-profile"),
            components,
        })
        .expect("qualification bytes");
        let qualification_schema =
            mfm_spec::schema_id(SHARED_QUALIFICATION_VERSION).expect("qualification schema");
        let qualification_ref =
            boundary_content_ref(qualification_schema.clone(), &qualification_canonical)
                .expect("qualification ref");
        let qualification_object = support_object(
            SHARED_QUALIFICATION_PATH,
            qualification_canonical,
            retained_support_contract(
                qualification_schema,
                "mfm.product",
                "component-qualification",
                stable_id(SHARED_QUALIFICATION_ROLE).expect("qualification role"),
                evidence_ref.clone(),
            )
            .expect("qualification contract"),
        );

        let implementation = ComponentImplementationDescriptor::new(
            ComponentKind::ReadCapabilityAdapterVerifier,
            capability_ref.clone(),
            callback_ref,
            qualification_ref.clone(),
        )
        .expect("implementation descriptor");
        let implementation_ref = implementation.content_ref().expect("implementation ref");
        let implementation_object = support_object(
            EVM_ADAPTER_IMPLEMENTATION_PATH,
            implementation
                .canonical_json()
                .expect("implementation bytes"),
            retained_support_contract(
                implementation_ref.schema_id().clone(),
                "mfm.product",
                "component-implementation-descriptor",
                stable_id(EVM_ADAPTER_IMPLEMENTATION_ROLE).expect("implementation role"),
                evidence_ref.clone(),
            )
            .expect("implementation contract"),
        );

        let binding = ReadCapabilityBinding::new(
            &capability_ref,
            &implementation_ref,
            &classifier_ref,
            &failure_ref,
            &support_ref(&reviewed_source),
            &catalog_ref,
        )
        .expect("binding");
        let binding_canonical =
            PlainCanonicalJsonBytes::from_canonical_json_slice(binding.as_bytes())
                .expect("binding bytes");
        let binding_object = support_object(
            "binding/read",
            binding_canonical,
            support_contract(
                binding.schema_id().clone(),
                "read-binding",
                evidence_ref.clone(),
            ),
        );

        let mut objects = vec![
            evidence,
            executable,
            qualification_object,
            reviewed_source,
            catalog_object,
            capability_object,
            failure_object,
            classifier_object,
            callback_object,
            implementation_object,
            binding_object,
        ];
        if include_route_generation {
            objects.extend(generation_objects);
        }
        let scope = test_semantic("qualification-scope");
        let graph = QualifiedSupportGraph::new(scope.clone(), objects).expect("support graph");
        let (store, issuer) = open_in_memory(StoreIdentity::new(
            StoreScopeId::new(format!("{}{}", StoreScopeId::PREFIX, "e".repeat(32)))
                .expect("store scope"),
            StoreEpoch::new(1),
        ));
        let authority = issuer.authorize_qualified_deployment(scope);
        let admitted = store
            .admit_support_graph(&authority, graph)
            .await
            .expect("admitted support graph");
        (
            binding,
            admitted,
            executable_ref,
            qualification_ref,
            evidence_ref,
            transport,
        )
    }

    #[test]
    fn safe_classifier_relation_is_exhaustive_and_phase_closed() {
        let classifier = EvmSafeFailureClassifier::new().expect("classifier");
        let cases = [
            (
                EvmSafeFailure::RoutingGenerationUnavailable,
                SafeFailureOutcome::DidNotEnter,
            ),
            (
                EvmSafeFailure::ConfigurationInvalid,
                SafeFailureOutcome::DidNotEnter,
            ),
            (
                EvmSafeFailure::RequestInvalid,
                SafeFailureOutcome::DidNotEnter,
            ),
            (
                EvmSafeFailure::AccessCancelled,
                SafeFailureOutcome::DidNotEnter,
            ),
            (
                EvmSafeFailure::AccessCancelled,
                SafeFailureOutcome::Indeterminate,
            ),
            (
                EvmSafeFailure::TransportFailed,
                SafeFailureOutcome::DidNotEnter,
            ),
            (
                EvmSafeFailure::TransportFailed,
                SafeFailureOutcome::Indeterminate,
            ),
            (
                EvmSafeFailure::HttpStatus { status: 503 },
                SafeFailureOutcome::Indeterminate,
            ),
            (
                EvmSafeFailure::JsonRpcError {
                    json_rpc_code: -32005,
                },
                SafeFailureOutcome::Indeterminate,
            ),
            (
                EvmSafeFailure::ResponseInvalid {
                    response_kind: mfm_evm::EvmResponseInvalidKind::MalformedEnvelope,
                    size_class: EvmCoarseSizeClass::UpTo16Kib,
                },
                SafeFailureOutcome::Indeterminate,
            ),
            (
                EvmSafeFailure::ResponseInvalid {
                    response_kind: mfm_evm::EvmResponseInvalidKind::InvalidResult,
                    size_class: EvmCoarseSizeClass::UpTo16Kib,
                },
                SafeFailureOutcome::Indeterminate,
            ),
            (
                EvmSafeFailure::ResponseMissingResult {
                    size_class: EvmCoarseSizeClass::Zero,
                },
                SafeFailureOutcome::Indeterminate,
            ),
            (
                EvmSafeFailure::ResponseTooLarge {
                    size_class: EvmCoarseSizeClass::Over1Mib,
                },
                SafeFailureOutcome::Indeterminate,
            ),
            (
                EvmSafeFailure::UnclassifiedFailure,
                SafeFailureOutcome::Indeterminate,
            ),
        ];
        for (failure, expected) in cases {
            let (code, size, diagnostic) = classifier.projection(&failure);
            let diagnostic_bytes = diagnostic
                .as_ref()
                .map(|diagnostic| canonical_json(diagnostic).expect("diagnostic bytes"));
            let diagnostic_schema_id = classifier
                .descriptor
                .diagnostic_schema_identity()
                .expect("diagnostic identity")
                .schema_id()
                .expect("diagnostic schema id");
            let diagnostic = diagnostic_bytes
                .as_ref()
                .map(|bytes| (&diagnostic_schema_id, bytes.as_bytes()));
            let rule = classifier
                .descriptor
                .classify(code, expected, size, diagnostic.is_some())
                .expect("classified failure");
            assert_eq!(rule.outcome(), expected);
            classifier
                .descriptor
                .verify(
                    classifier.descriptor.safe_failure_contract_ref(),
                    code,
                    expected,
                    rule.failure_class(),
                    rule.boundary_stage(),
                    size,
                    diagnostic,
                )
                .expect("exact tuple");
            let hostile = match expected {
                SafeFailureOutcome::DidNotEnter => SafeFailureOutcome::Indeterminate,
                SafeFailureOutcome::Indeterminate => SafeFailureOutcome::DidNotEnter,
            };
            assert!(classifier
                .descriptor
                .verify(
                    classifier.descriptor.safe_failure_contract_ref(),
                    code,
                    hostile,
                    rule.failure_class(),
                    rule.boundary_stage(),
                    size,
                    diagnostic,
                )
                .is_err());
        }
    }

    #[test]
    fn shared_qualification_accepts_app_canonical_json_component_order() {
        let executable_ref = test_ref("executable");
        let capability_ref = evm_read_capability_contract_ref().expect("capability ref");
        let callback_ref = evm_adapter_callback_surface_ref().expect("callback ref");
        let components = app_shaped_qualification_components(&capability_ref, &callback_ref);
        let mut former_rust_field_order = components.clone();
        former_rust_field_order.sort_by(|left, right| {
            left.component_kind
                .cmp(&right.component_kind)
                .then_with(|| left.semantic_contract_ref.cmp(&right.semantic_contract_ref))
                .then_with(|| left.callback_surface_ref.cmp(&right.callback_surface_ref))
        });
        assert_ne!(
            components, former_rust_field_order,
            "fixture must distinguish annex canonical-JSON ordering from derived field ordering"
        );
        let qualification = canonical_json(&SharedQualification {
            version: SHARED_QUALIFICATION_VERSION.to_owned(),
            executable_identity_ref: executable_ref.clone(),
            _qualification_profile_ref: test_ref("qualification-profile"),
            components,
        })
        .expect("shared qualification");

        verify_shared_qualification(
            qualification.as_bytes(),
            &executable_ref,
            &capability_ref,
            &callback_ref,
        )
        .expect("app-shaped canonical qualification");
    }

    #[test]
    fn classifier_and_callback_descriptors_are_closed_and_redaction_safe() {
        let classifier = evm_safe_classifier_canonical().expect("classifier");
        let classifier_json: serde_json::Value =
            serde_json::from_slice(classifier.as_bytes()).expect("classifier JSON");
        let rendered = format!(
            "{}{}",
            classifier.as_str(),
            evm_adapter_callback_surface_canonical()
                .expect("callbacks")
                .as_str(),
        );
        for forbidden in [
            "endpoint",
            "authorization_header",
            "credential",
            "provider_message",
            "response_body",
            "filesystem",
        ] {
            assert!(!rendered.contains(forbidden));
        }
        assert_eq!(
            classifier_json["rules"]
                .as_array()
                .expect("classifier rules")
                .len(),
            13,
            "every legal safe failure outcome has exactly one classifier rule"
        );
    }

    #[tokio::test]
    async fn qualified_factory_closes_binding_implementation_routes_and_evidence() {
        let (binding, graph, executable_ref, qualification_ref, evidence_ref, transport) =
            qualified_factory_fixture(true).await;
        let artifacts = EvmReadQualificationArtifacts::new(
            &graph,
            &executable_ref,
            &qualification_ref,
            &evidence_ref,
        )
        .expect("qualified adapter");
        let qualified =
            qualify_evm_read_entries(binding, &artifacts, transport).expect("qualified reads");
        assert_eq!(
            qualified.chain_identity.operation().operation_id().as_str(),
            EVM_CHAIN_ID_OPERATION_ID
        );
        assert_eq!(
            qualified.confirm_anchor.operation().operation_id().as_str(),
            EVM_CONFIRM_ANCHOR_OPERATION_ID
        );
    }

    #[tokio::test]
    async fn qualified_factory_rejects_missing_route_generation_before_registration() {
        let (binding, graph, executable_ref, qualification_ref, evidence_ref, transport) =
            qualified_factory_fixture(false).await;
        let artifacts = EvmReadQualificationArtifacts::new(
            &graph,
            &executable_ref,
            &qualification_ref,
            &evidence_ref,
        )
        .expect("adapter qualification does not own routing");
        assert!(qualify_evm_read_entries(binding, &artifacts, transport).is_err());
    }

    #[tokio::test]
    async fn qualified_factory_rejects_another_qualification_authority() {
        let (_, graph, executable_ref, _, evidence_ref, _) = qualified_factory_fixture(true).await;
        let other = ContentRef::new(
            test_schema("other-qualification"),
            mfm_ids::ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                sha256_digest_bytes(b"other qualification"),
            ),
        )
        .expect("other qualification ref");
        assert!(
            EvmReadQualificationArtifacts::new(&graph, &executable_ref, &other, &evidence_ref,)
                .is_err()
        );
    }

    #[tokio::test]
    async fn every_typed_request_projects_its_exact_generation_before_io() {
        let (_, _, _, _, _, transport) = qualified_factory_fixture(true).await;
        let generation = transport
            .routing_generation_descriptors()
            .next()
            .expect("generation")
            .0
            .clone();
        let generation_ref = generation.to_content_ref().expect("generation ref");
        let binding =
            mfm_evm::EvmNetworkBinding::new("ethereum-mainnet", 1, generation).expect("binding");
        let checked =
            mfm_evm::EvmCheckedSource::new(binding.clone(), "primary", "mfm.evm.json-rpc.v1")
                .expect("checked source");
        let anchored = mfm_evm::EvmAnchoredSource::new(
            checked.clone(),
            mfm_evm::EvmBlockAnchor::new(U256::from(42), B256::from([0x11; 32])),
        )
        .expect("anchored source");
        let adapter = EvmReadAdapter {
            transport,
            safe_classifier: EvmSafeFailureClassifier::new().expect("safe classifier"),
        };

        let chain = EvmChainIdentityRequest::new(binding);
        let latest = EvmLatestAnchorRequest::new(checked);
        let decimals = EvmTokenDecimalsRequest::new(anchored.clone(), Address::from([1; 20]));
        let native = EvmNativeBalanceRequest::new(anchored.clone(), Address::from([2; 20]));
        let token = EvmTokenBalanceRequest::new(
            anchored.clone(),
            Address::from([2; 20]),
            Address::from([1; 20]),
        );
        let confirm = EvmAnchorConfirmationRequest::new(anchored);

        assert_eq!(
            <EvmReadAdapter as AuditedReadCapability<EvmChainIdentityRequest>>::routing_generation_ref(
                &adapter, &chain,
            ),
            Some(generation_ref.clone())
        );
        assert_eq!(
            <EvmReadAdapter as AuditedReadCapability<EvmLatestAnchorRequest>>::routing_generation_ref(
                &adapter, &latest,
            ),
            Some(generation_ref.clone())
        );
        assert_eq!(
            <EvmReadAdapter as AuditedReadCapability<EvmTokenDecimalsRequest>>::routing_generation_ref(
                &adapter, &decimals,
            ),
            Some(generation_ref.clone())
        );
        assert_eq!(
            <EvmReadAdapter as AuditedReadCapability<EvmNativeBalanceRequest>>::routing_generation_ref(
                &adapter, &native,
            ),
            Some(generation_ref.clone())
        );
        assert_eq!(
            <EvmReadAdapter as AuditedReadCapability<EvmTokenBalanceRequest>>::routing_generation_ref(
                &adapter, &token,
            ),
            Some(generation_ref.clone())
        );
        assert_eq!(
            <EvmReadAdapter as AuditedReadCapability<EvmAnchorConfirmationRequest>>::routing_generation_ref(
                &adapter, &confirm,
            ),
            Some(generation_ref)
        );
        assert_eq!(
            <EvmReadAdapter as AuditedReadCapability<EvmAnchorConfirmationRequest>>::routing_generation_ref(
                &adapter,
                &EvmAnchorConfirmationRequest::invalid_input(),
            ),
            None
        );
    }
}
