use mfm_certify::structured::{
    PhysicalBindingSelection, ProgramRegistryBuilder, RuntimeReadPhysicalBinding,
    RuntimeReadPhysicalBindingSource,
};
use std::collections::BTreeMap;
use std::sync::Arc;

use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::{
    BoundedComponentContract, BoundedComponentInvoker, CapabilityContractFault, ComponentFuture,
    ReadAdapterCompletion, ReadAdapterInvoker, ReadCapabilityContract,
    ReadCapabilityImplementation, ResourceAuthorityContract, SignerContract,
};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, SchemaId, StableId};
use mfm_journal::structured::HistoryObject;
use mfm_program::structured::{
    state_contract, AuthoringPolicy, CapabilityExpansion, ChildOperation, ClosedSum,
    CustomFailureHandler, DefaultFailureMapper, Direct, FanOutResults, Never, OperationBuilder,
    PolicyExpansionRecipe, PolicyFailurePostBuilder, PolicyRecipeBuilder, ProposedSuccessOutcome,
    Pure, Read, RecoveryRouteBuilder, RequiresCapability, RuntimeReadAdapter,
    RuntimeReadCapability, RuntimeResourceAuthority, RuntimeSigner, SafeFailureMayFail,
    SafeFailureNotApplicable, SafeFailureSuccessOnly, State, StateFrame, StateSettlement,
    StructuredStateCallbacks,
};
use mfm_program_derive::MfmValue;
use mfm_spec::structured::{
    AuthoredBlock, AuthoredDeclaration, AuthoredFailureDirective, CertifiedComponentObject,
    CertifiedFailureBoundary, CertifiedProgramComponents, ExpandedDeclaration, ExpansionBoundaryId,
    ExpansionPolicyContract, ExpansionStage, FailurePlan, PolicyExpansionBinding,
    ProposedStateOutcome, SecretFreeExecutableIdentity, SecretFreeImplementationDescriptor,
    SecretFreeImplementationManifest, SecretFreeQualificationArtifact,
    StateCapabilityAdapterSignerResourceManifest, StructuredComponentDependency,
    StructuredComponentKind, StructuredExpansionProfile, StructuredLiveComponentContract,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.fixture",
    name = "certification_request",
    version = "1",
    schema = "mfm.fixture.certification_request"
)]
struct Request {
    value: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.fixture",
    name = "certification_response",
    version = "1",
    schema = "mfm.fixture.certification_response"
)]
struct Response {
    value: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.fixture",
    name = "certification_state_failure",
    version = "1",
    schema = "mfm.fixture.certification_state_failure"
)]
struct StateFailure {
    code: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.fixture",
    name = "certification_normalized_failure",
    version = "1",
    schema = "mfm.fixture.certification_normalized_failure"
)]
struct NormalizedFailure {
    code: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.fixture",
    name = "certification_root_failure",
    version = "1",
    schema = "mfm.fixture.certification_root_failure"
)]
struct RootFailure {
    code: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.fixture",
    name = "certification_post_failure",
    version = "1",
    schema = "mfm.fixture.certification_post_failure"
)]
struct PostFailure {
    code: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.fixture",
    name = "certification_default_route",
    version = "1",
    schema = "mfm.fixture.certification_default_route"
)]
enum DefaultRoute {
    Propagate { failure: RootFailure },
}

impl ClosedSum for DefaultRoute {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.fixture",
    name = "certification_guard_decision",
    version = "1",
    schema = "mfm.fixture.certification_guard_decision"
)]
enum GuardDecision {
    Allow { request: Request },
    Deny { failure: StateFailure },
}

impl ClosedSum for GuardDecision {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.fixture",
    name = "certification_recovery_route",
    version = "1",
    schema = "mfm.fixture.certification_recovery_route"
)]
enum RecoveryRoute {
    Recover { response: Response },
}

impl ClosedSum for RecoveryRoute {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.fixture",
    name = "certification_post_failure_route",
    version = "1",
    schema = "mfm.fixture.certification_post_failure_route"
)]
enum PostFailureRoute {
    Propagate { failure: StateFailure },
}

impl ClosedSum for PostFailureRoute {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.fixture",
    name = "certification_state_failure_route",
    version = "1",
    schema = "mfm.fixture.certification_state_failure_route"
)]
enum StateFailureRoute {
    Propagate { failure: StateFailure },
}

impl ClosedSum for StateFailureRoute {}

struct CopyState;
struct FallibleState;
struct RootFailureMapperState;
struct RootFailureMapper;
struct AbstractCapabilityState;
struct ChildCapabilityState;
struct NestedChildCapabilityState;
struct RecursiveChildCapabilityState;
struct RecoveringChildCapabilityState;
struct RecursiveCapabilityState;
struct ConcreteCapabilityState;
struct CopyChild;
struct NestedCopyChild;
struct RecursiveSupportChild;
struct RecoverableChild;
struct FailureAuditState;
struct RedactStateFailure;
struct NormalizeStateFailure;
struct RebuildStateFailure;
struct OuterPreState;
struct OuterPostState;
struct InnerPreState;
struct InnerPostState;
struct GuardState;
struct LaneAState;
struct LaneBState;
struct RecoveryHandlerState;
struct RecoveryReadState;
struct RecoveryHandler;
struct FailingFailurePostState;
struct PostFailureMapperState;
struct PostFailureMapper;
struct AbstractFallibleCapabilityState;
struct FixtureReadCapability;
struct CapabilityRecipeExpansion;
struct ChildCapabilityRecipeExpansion;
struct NestedChildCapabilityRecipeExpansion;
struct RecursiveChildCapabilityRecipeExpansion;
struct RecoveringChildCapabilityRecipeExpansion;
struct RecursiveCapabilityRecipeExpansion;
struct FallibleCapabilityRecipeExpansion;
struct StateFailureMapperState;
struct StateFailureMapper;
struct PipelineChild;
type PipelineInnerJoin = FanOutResults<Response, Never>;
type PipelineOuterJoin = FanOutResults<PipelineInnerJoin, Never>;
struct PipelineAggregateState;

impl ReadCapabilityContract for FixtureReadCapability {
    type Request = Request;
    type Returned = Response;
    type SafeFailure = StateFailure;
}

impl RuntimeReadCapability for FixtureReadCapability {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        fixture_capability_contract()
    }
}

struct FixtureReadCapabilityImplementation;

impl ReadCapabilityImplementation<FixtureReadCapability> for FixtureReadCapabilityImplementation {
    fn validate_request(
        &self,
        _request: &Request,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }

    fn validate_returned(
        &self,
        _returned: &Response,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }

    fn validate_safe_failure(
        &self,
        _failure: &StateFailure,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }
}

struct FixtureReadAdapter {
    certificate: HistoryObject,
}

impl ReadAdapterInvoker<FixtureReadCapability> for FixtureReadAdapter {
    fn invoke<'a>(
        &'a self,
        request: &'a Request,
    ) -> ComponentFuture<'a, ReadAdapterCompletion<Response, StateFailure>> {
        Box::pin(async move {
            ReadAdapterCompletion::Returned(Response {
                value: request.value,
            })
        })
    }
}

impl RuntimeReadAdapter<FixtureReadCapability> for FixtureReadAdapter {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        fixture_adapter_contract()
    }
}

impl RuntimeReadPhysicalBinding<FixtureReadCapability> for FixtureReadAdapter {
    fn public_certificate(&self) -> &HistoryObject {
        &self.certificate
    }
}

struct FixtureReadBindingSource {
    binding: Arc<FixtureReadAdapter>,
}

impl RuntimeReadPhysicalBindingSource<FixtureReadCapability> for FixtureReadBindingSource {
    type Binding = FixtureReadAdapter;

    fn current_binding<'a>(
        &'a self,
        _selection: PhysicalBindingSelection<'a>,
        _request: &'a Request,
    ) -> ComponentFuture<'a, Option<Arc<Self::Binding>>> {
        let binding = Arc::clone(&self.binding);
        Box::pin(async move { Some(binding) })
    }
}

struct FixtureSigner;
struct FixtureSignerInvoker;

impl BoundedComponentContract for FixtureSigner {
    type Request = ();
    type Completion = ();
}

impl SignerContract for FixtureSigner {}

impl RuntimeSigner for FixtureSigner {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        fixture_signer_contract()
    }
}

impl BoundedComponentInvoker<FixtureSigner> for FixtureSignerInvoker {
    fn invoke<'a>(&'a self, _request: &'a ()) -> ComponentFuture<'a, ()> {
        Box::pin(async {})
    }
}

struct FixtureResource;
struct FixtureResourceInvoker;

impl BoundedComponentContract for FixtureResource {
    type Request = ();
    type Completion = ();
}

impl ResourceAuthorityContract for FixtureResource {}

impl RuntimeResourceAuthority for FixtureResource {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        fixture_resource_contract()
    }
}

impl BoundedComponentInvoker<FixtureResource> for FixtureResourceInvoker {
    fn invoke<'a>(&'a self, _request: &'a ()) -> ComponentFuture<'a, ()> {
        Box::pin(async {})
    }
}

impl CapabilityExpansion for CapabilityRecipeExpansion {
    fn recipe() -> mfm_program::Result<mfm_spec::structured::AuthoredStructuredProgram> {
        Ok(capability_recipe())
    }
}

impl CapabilityExpansion for ChildCapabilityRecipeExpansion {
    fn recipe() -> mfm_program::Result<mfm_spec::structured::AuthoredStructuredProgram> {
        Ok(child_capability_recipe())
    }
}

impl CapabilityExpansion for NestedChildCapabilityRecipeExpansion {
    fn recipe() -> mfm_program::Result<mfm_spec::structured::AuthoredStructuredProgram> {
        Ok(nested_child_capability_recipe())
    }
}

impl CapabilityExpansion for RecursiveChildCapabilityRecipeExpansion {
    fn recipe() -> mfm_program::Result<mfm_spec::structured::AuthoredStructuredProgram> {
        Ok(recursive_child_capability_recipe())
    }
}

impl CapabilityExpansion for RecoveringChildCapabilityRecipeExpansion {
    fn recipe() -> mfm_program::Result<mfm_spec::structured::AuthoredStructuredProgram> {
        Ok(recovering_child_capability_recipe())
    }
}

impl CapabilityExpansion for RecursiveCapabilityRecipeExpansion {
    fn recipe() -> mfm_program::Result<mfm_spec::structured::AuthoredStructuredProgram> {
        Ok(recursive_capability_recipe())
    }
}

impl CapabilityExpansion for FallibleCapabilityRecipeExpansion {
    fn recipe() -> mfm_program::Result<mfm_spec::structured::AuthoredStructuredProgram> {
        Ok(fallible_capability_recipe())
    }
}

impl State for CopyState {
    type Input = Request;
    type Output = Response;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(stable("mfm.fixture/copy-state"))
    }
}

impl State for FallibleState {
    type Input = Request;
    type Output = Response;
    type Failure = StateFailure;
    type Request = Request;
    type Returned = Response;
    type SafeFailure = StateFailure;
    type Execution = Read<FixtureReadCapability>;
    type SafeFailureDisposition = SafeFailureMayFail;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(stable("mfm.fixture/fallible-state"))
    }
}

impl State for RecoveryReadState {
    type Input = Response;
    type Output = Response;
    type Failure = Never;
    type Request = Request;
    type Returned = Response;
    type SafeFailure = StateFailure;
    type Execution = Read<FixtureReadCapability>;
    type SafeFailureDisposition = SafeFailureSuccessOnly;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(stable("mfm.fixture/recovery-read"))
    }
}

impl State for RootFailureMapperState {
    type Input = StateFailure;
    type Output = DefaultRoute;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(stable("mfm.fixture/root-failure-mapper"))
    }
}

impl DefaultFailureMapper<StateFailure, RootFailure> for RootFailureMapper {
    type Route = DefaultRoute;
    type Mapper = RootFailureMapperState;
}

impl State for AbstractCapabilityState {
    type Input = Request;
    type Output = Response;
    type Failure = Never;
    type Request = Request;
    type Returned = Response;
    type SafeFailure = StateFailure;
    type Execution = Read<FixtureReadCapability>;
    type SafeFailureDisposition = SafeFailureSuccessOnly;
    type Capability = RequiresCapability<CapabilityRecipeExpansion>;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(stable("mfm.fixture/abstract-capability-state"))
    }
}

impl State for ChildCapabilityState {
    type Input = Request;
    type Output = Response;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = RequiresCapability<ChildCapabilityRecipeExpansion>;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(stable("mfm.fixture/child-capability-state"))
    }
}

impl State for NestedChildCapabilityState {
    type Input = Request;
    type Output = Response;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = RequiresCapability<NestedChildCapabilityRecipeExpansion>;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(stable("mfm.fixture/nested-child-capability-state"))
    }
}

impl State for RecursiveChildCapabilityState {
    type Input = Request;
    type Output = Response;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = RequiresCapability<RecursiveChildCapabilityRecipeExpansion>;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(stable("mfm.fixture/recursive-child-capability-state"))
    }
}

impl State for RecoveringChildCapabilityState {
    type Input = Request;
    type Output = Response;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = RequiresCapability<RecoveringChildCapabilityRecipeExpansion>;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(stable("mfm.fixture/recovering-child-capability-state"))
    }
}

impl State for RecursiveCapabilityState {
    type Input = Request;
    type Output = Response;
    type Failure = Never;
    type Request = Request;
    type Returned = Response;
    type SafeFailure = StateFailure;
    type Execution = Read<FixtureReadCapability>;
    type SafeFailureDisposition = SafeFailureSuccessOnly;
    type Capability = RequiresCapability<RecursiveCapabilityRecipeExpansion>;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(stable("mfm.fixture/recursive-capability-state"))
    }
}

impl State for ConcreteCapabilityState {
    type Input = Request;
    type Output = Response;
    type Failure = Never;
    type Request = Request;
    type Returned = Response;
    type SafeFailure = StateFailure;
    type Execution = Read<FixtureReadCapability>;
    type SafeFailureDisposition = SafeFailureSuccessOnly;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(stable("mfm.fixture/concrete-capability-state"))
    }
}

impl State for AbstractFallibleCapabilityState {
    type Input = Request;
    type Output = Response;
    type Failure = StateFailure;
    type Request = Request;
    type Returned = Response;
    type SafeFailure = StateFailure;
    type Execution = Read<FixtureReadCapability>;
    type SafeFailureDisposition = SafeFailureMayFail;
    type Capability = RequiresCapability<FallibleCapabilityRecipeExpansion>;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(stable("mfm.fixture/abstract-fallible-capability"))
    }
}

impl State for StateFailureMapperState {
    type Input = StateFailure;
    type Output = StateFailureRoute;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(stable("mfm.fixture/state-failure-mapper"))
    }
}

impl DefaultFailureMapper<StateFailure, StateFailure> for StateFailureMapper {
    type Route = StateFailureRoute;
    type Mapper = StateFailureMapperState;
}

impl ChildOperation for PipelineChild {
    type Output = Response;
    type Failure = StateFailure;

    fn authored_program_ref() -> mfm_program::Result<mfm_ids::ContentRef> {
        pipeline_child_program().content_ref().map_err(Into::into)
    }
}

impl State for PipelineAggregateState {
    type Input = PipelineOuterJoin;
    type Output = Response;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(stable("mfm.fixture/pipeline-aggregate"))
    }
}

impl ChildOperation for CopyChild {
    type Output = Response;
    type Failure = Never;

    fn authored_program_ref() -> mfm_program::Result<mfm_ids::ContentRef> {
        copy_child_program().content_ref().map_err(Into::into)
    }
}

impl ChildOperation for NestedCopyChild {
    type Output = Response;
    type Failure = Never;

    fn authored_program_ref() -> mfm_program::Result<mfm_ids::ContentRef> {
        nested_copy_child_program()
            .content_ref()
            .map_err(Into::into)
    }
}

impl ChildOperation for RecursiveSupportChild {
    type Output = Response;
    type Failure = Never;

    fn authored_program_ref() -> mfm_program::Result<mfm_ids::ContentRef> {
        recursive_support_child_program()
            .content_ref()
            .map_err(Into::into)
    }
}

impl ChildOperation for RecoverableChild {
    type Output = Response;
    type Failure = StateFailure;

    fn authored_program_ref() -> mfm_program::Result<mfm_ids::ContentRef> {
        recoverable_child_program()
            .content_ref()
            .map_err(Into::into)
    }
}

impl State for FailureAuditState {
    type Input = StateFailure;
    type Output = StateFailure;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(stable("mfm.fixture/failure-audit-state"))
    }
}

macro_rules! infallible_state {
    ($name:ident, $input:ty, $output:ty, $id:literal) => {
        impl State for $name {
            type Input = $input;
            type Output = $output;
            type Failure = Never;
            type Request = ();
            type Returned = ();
            type SafeFailure = ();
            type Execution = Pure;
            type SafeFailureDisposition = SafeFailureNotApplicable;
            type Capability = Direct;

            fn semantic_state_id() -> mfm_program::Result<StableId> {
                Ok(stable($id))
            }
        }
    };
}

infallible_state!(OuterPreState, Request, Request, "mfm.fixture/outer-pre");
infallible_state!(OuterPostState, Response, Response, "mfm.fixture/outer-post");
infallible_state!(LaneAState, Request, Response, "mfm.fixture/lane-a");
infallible_state!(LaneBState, Request, Response, "mfm.fixture/lane-b");
infallible_state!(
    RedactStateFailure,
    StateFailure,
    StateFailure,
    "mfm.fixture/redact-state-failure"
);
infallible_state!(
    NormalizeStateFailure,
    StateFailure,
    NormalizedFailure,
    "mfm.fixture/normalize-state-failure"
);
infallible_state!(
    RebuildStateFailure,
    NormalizedFailure,
    StateFailure,
    "mfm.fixture/rebuild-state-failure"
);
infallible_state!(
    RecoveryHandlerState,
    StateFailure,
    RecoveryRoute,
    "mfm.fixture/recovery-handler"
);

impl CustomFailureHandler<StateFailure, Response, Never> for RecoveryHandler {
    type Route = RecoveryRoute;
    type Handler = RecoveryHandlerState;

    fn author_routes<Policy>(
        routes: &mut RecoveryRouteBuilder<Response, Never, Policy>,
    ) -> mfm_program::Result<()>
    where
        Policy: AuthoringPolicy,
    {
        routes.arm("recover", stable("recover"), |block, payloads| {
            let response = payloads.value::<Response>(&[stable("response")])?;
            let recovered = block
                .state::<RecoveryReadState>(stable("recovery-read"), &response)?
                .infallible()?;
            block.normal(&recovered)
        })
    }
}

impl State for FailingFailurePostState {
    type Input = StateFailure;
    type Output = StateFailure;
    type Failure = PostFailure;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(stable("mfm.fixture/failing-failure-post"))
    }
}

impl State for PostFailureMapperState {
    type Input = PostFailure;
    type Output = PostFailureRoute;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(stable("mfm.fixture/post-failure-mapper"))
    }
}

impl DefaultFailureMapper<PostFailure, StateFailure> for PostFailureMapper {
    type Route = PostFailureRoute;
    type Mapper = PostFailureMapperState;
}

infallible_state!(
    GuardState,
    Request,
    GuardDecision,
    "mfm.fixture/guard-state"
);
infallible_state!(InnerPreState, Request, Request, "mfm.fixture/inner-pre");
infallible_state!(InnerPostState, Response, Response, "mfm.fixture/inner-post");

trait FixtureStateProcess: State {
    fn callbacks() -> StructuredStateCallbacks<Self>
    where
        Self: Sized;
}

macro_rules! pure_state_process {
    ($state:ty, |$frame:ident| $output:expr) => {
        impl FixtureStateProcess for $state {
            fn callbacks() -> StructuredStateCallbacks<Self> {
                StructuredStateCallbacks::Pure {
                    apply: Arc::new(|$frame: StateFrame<'_, <Self as State>::Input>| {
                        ProposedStateOutcome::Success($output)
                    }),
                }
            }
        }
    };
}

pure_state_process!(CopyState, |frame| Response {
    value: frame.input().value
});
pure_state_process!(RootFailureMapperState, |frame| DefaultRoute::Propagate {
    failure: RootFailure {
        code: frame.input().code,
    },
});
pure_state_process!(StateFailureMapperState, |frame| {
    StateFailureRoute::Propagate {
        failure: frame.input().clone(),
    }
});
pure_state_process!(PipelineAggregateState, |_frame| Response { value: 0 });
pure_state_process!(FailureAuditState, |frame| frame.input().clone());
pure_state_process!(RedactStateFailure, |frame| StateFailure {
    code: frame.input().code.min(999),
});
pure_state_process!(NormalizeStateFailure, |frame| NormalizedFailure {
    code: frame.input().code + 10,
});
pure_state_process!(RebuildStateFailure, |frame| StateFailure {
    code: frame.input().code + 100,
});
pure_state_process!(OuterPreState, |frame| frame.input().clone());
pure_state_process!(OuterPostState, |frame| frame.input().clone());
pure_state_process!(InnerPreState, |frame| frame.input().clone());
pure_state_process!(InnerPostState, |frame| frame.input().clone());
pure_state_process!(LaneAState, |frame| Response {
    value: frame.input().value
});
pure_state_process!(LaneBState, |frame| Response {
    value: frame.input().value
});
pure_state_process!(RecoveryHandlerState, |frame| RecoveryRoute::Recover {
    response: Response {
        value: frame.input().code,
    },
});
pure_state_process!(PostFailureMapperState, |frame| {
    PostFailureRoute::Propagate {
        failure: StateFailure {
            code: frame.input().code,
        },
    }
});
pure_state_process!(GuardState, |frame| GuardDecision::Allow {
    request: frame.input().clone(),
});

impl FixtureStateProcess for FailingFailurePostState {
    fn callbacks() -> StructuredStateCallbacks<Self> {
        StructuredStateCallbacks::Pure {
            apply: Arc::new(|frame: StateFrame<'_, StateFailure>| {
                ProposedStateOutcome::Failure(PostFailure {
                    code: frame.input().code,
                })
            }),
        }
    }
}

fn read_state_callbacks<S>() -> StructuredStateCallbacks<S>
where
    S: State<
        Input = Request,
        Output = Response,
        Failure = StateFailure,
        Request = Request,
        Returned = Response,
        SafeFailure = StateFailure,
        Execution = Read<FixtureReadCapability>,
        SafeFailureDisposition = SafeFailureMayFail,
    >,
{
    StructuredStateCallbacks::Read {
        request: Arc::new(|frame| frame.input().clone()),
        settle_returned: Arc::new(|_frame, returned| {
            StateSettlement::Proposed(ProposedStateOutcome::Success(returned.clone()))
        }),
        settle_safe_failure: Arc::new(|_frame, failure| {
            ProposedStateOutcome::Failure(failure.clone())
        }),
    }
}

impl FixtureStateProcess for FallibleState {
    fn callbacks() -> StructuredStateCallbacks<Self> {
        read_state_callbacks::<Self>()
    }
}

impl FixtureStateProcess for RecoveryReadState {
    fn callbacks() -> StructuredStateCallbacks<Self> {
        StructuredStateCallbacks::Read {
            request: Arc::new(|frame| Request {
                value: frame.input().value,
            }),
            settle_returned: Arc::new(|_frame, returned| {
                StateSettlement::Proposed(ProposedStateOutcome::Success(returned.clone()))
            }),
            settle_safe_failure: Arc::new(|_frame, failure| {
                ProposedSuccessOutcome::new(Response {
                    value: failure.code,
                })
            }),
        }
    }
}

impl FixtureStateProcess for ConcreteCapabilityState {
    fn callbacks() -> StructuredStateCallbacks<Self> {
        StructuredStateCallbacks::Read {
            request: Arc::new(|frame| frame.input().clone()),
            settle_returned: Arc::new(|_frame, returned| {
                StateSettlement::Proposed(ProposedStateOutcome::Success(returned.clone()))
            }),
            settle_safe_failure: Arc::new(|_frame, failure| {
                ProposedSuccessOutcome::new(Response {
                    value: failure.code,
                })
            }),
        }
    }
}

trait FixtureRegistryExt {
    fn register_fixture_state<S: FixtureStateProcess>(
        &mut self,
        implementation_id: StableId,
    ) -> mfm_certify::Result<ContentRef>;
}

impl FixtureRegistryExt for ProgramRegistryBuilder {
    fn register_fixture_state<S: FixtureStateProcess>(
        &mut self,
        implementation_id: StableId,
    ) -> mfm_certify::Result<ContentRef> {
        let semantic_contract_ref = state_contract::<S>()
            .map_err(|error| mfm_certify::CertifyError::Certification(error.to_string()))?
            .state_contract_ref;
        let descriptor = fixture_implementation_descriptor(
            self,
            StructuredComponentKind::State,
            semantic_contract_ref,
            implementation_id,
        )?;
        ProgramRegistryBuilder::register_state::<S>(self, descriptor, S::callbacks())
    }
}

fn fixture_implementation_descriptor(
    assembly: &mut ProgramRegistryBuilder,
    component_kind: StructuredComponentKind,
    semantic_contract_ref: ContentRef,
    implementation_id: StableId,
) -> mfm_certify::Result<SecretFreeImplementationDescriptor> {
    let executable_identity_ref =
        assembly.register_executable_identity(SecretFreeExecutableIdentity {
            executable_id: stable("mfm.fixture/test-executable"),
        })?;
    let qualification_artifact_ref =
        assembly.register_qualification_artifact(SecretFreeQualificationArtifact {
            qualification_id: stable("mfm.fixture/test-qualification"),
        })?;
    Ok(SecretFreeImplementationDescriptor {
        component_kind,
        semantic_contract_ref,
        implementation_id,
        executable_identity_ref,
        qualification_artifact_ref,
    })
}

#[test]
fn immutable_registry_certifies_and_reverifies_one_exact_program() {
    let operation_id = stable("mfm.fixture/certification");
    let authored = copy_program(operation_id.clone());
    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<Request>()
        .expect("request contract");
    assembly
        .register_value::<Response>()
        .expect("response contract");
    assembly
        .register_fixture_state::<CopyState>(stable("mfm.fixture/copy-state-implementation"))
        .expect("copy state registration");
    assembly
        .register_entry_point(operation_id.clone(), authored.clone(), profile())
        .expect("entry-point registration");

    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified registry");
    let certified = registry
        .certifier(&operation_id)
        .expect("qualified certifier")
        .certify(authored.clone())
        .expect("certified program");

    assert_eq!(certified.authored(), &authored);
    assert_eq!(certified.expanded().root.declarations.len(), 1);
    assert!(matches!(
        certified.expanded().root.declarations[0],
        ExpandedDeclaration::State(_)
    ));
    assert!(certified.expansion_proof().substitution_trace.is_empty());
    assert!(certified.policy_coverage_proof().entries.is_empty());
    assert_eq!(
        certified.reference().expect("certified reference"),
        certified
            .document()
            .content_ref()
            .expect("document reference")
    );

    let verified = registry
        .admission_verifier(&operation_id)
        .expect("qualified admission verifier")
        .verify(certified.document())
        .expect("exact registered document");
    assert_eq!(
        verified.reference().expect("verified reference"),
        certified.reference().expect("certified reference")
    );
    let component_manifest: StateCapabilityAdapterSignerResourceManifest = serde_json::from_value(
        certified
            .document()
            .component_closure
            .iter()
            .find(|object| {
                object.content_ref
                    == certified
                        .document()
                        .root
                        .components
                        .state_capability_adapter_signer_resource_manifest_closure_ref
            })
            .expect("component manifest")
            .value
            .as_json()
            .clone(),
    )
    .expect("component manifest schema");
    assert_eq!(component_manifest.entries.len(), 1);
    assert_eq!(
        component_manifest.entries[0].component_kind,
        StructuredComponentKind::State
    );
}

#[test]
fn one_entry_certifies_distinct_candidate_roots_inside_its_support_envelope() {
    let operation_id = stable("mfm.fixture/dynamic-candidates");
    let template = copy_program_with_label(operation_id.clone(), "copy");
    let alternate = copy_program_with_label(operation_id.clone(), "alternate-copy");
    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<Request>()
        .expect("request contract");
    assembly
        .register_value::<Response>()
        .expect("response contract");
    assembly
        .register_fixture_state::<CopyState>(stable("mfm.fixture/copy-state-implementation"))
        .expect("copy state registration");
    assembly
        .register_entry_point(operation_id.clone(), template.clone(), profile())
        .expect("entry-point registration");

    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified registry");
    let first = registry
        .certifier(&operation_id)
        .expect("certifier")
        .certify(template)
        .expect("template candidate");
    let second = registry
        .certifier(&operation_id)
        .expect("certifier")
        .certify(alternate)
        .expect("alternate candidate");
    assert_ne!(
        first.reference().expect("first root"),
        second.reference().expect("second root")
    );
    registry
        .admission_verifier(&operation_id)
        .expect("verifier")
        .verify(first.document())
        .expect("first persisted candidate");
    registry
        .admission_verifier(&operation_id)
        .expect("verifier")
        .verify(second.document())
        .expect("second persisted candidate");
}

#[test]
fn entry_support_envelopes_reject_foreign_components_signatures_and_manifests() {
    let entry_a = stable("mfm.fixture/support-envelope-a");
    let entry_b = stable("mfm.fixture/support-envelope-b");
    let authored_a = copy_program(entry_a.clone());
    let authored_b = lane_a_program(entry_b.clone());
    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<Request>()
        .expect("request contract");
    assembly
        .register_value::<Response>()
        .expect("response contract");
    assembly
        .register_fixture_state::<CopyState>(stable("mfm.fixture/copy-state-implementation"))
        .expect("copy state registration");
    assembly
        .register_fixture_state::<LaneAState>(stable("mfm.fixture/lane-a-implementation"))
        .expect("lane state registration");
    assembly
        .register_entry_point(entry_a.clone(), authored_a.clone(), profile())
        .expect("entry A registration");
    assembly
        .register_entry_point(entry_b.clone(), authored_b.clone(), profile())
        .expect("entry B registration");
    let registry = assembly
        .build(&[entry_a.clone(), entry_b.clone()])
        .expect("two-entry qualified registry");

    registry
        .certifier(&entry_a)
        .expect("entry A certifier")
        .certify(lane_a_program(entry_a.clone()))
        .expect_err("entry B-only state must exceed entry A's support envelope");
    registry
        .certifier(&entry_a)
        .expect("entry A certifier")
        .certify(copy_program(entry_b.clone()))
        .expect_err("entry B operation signature must not qualify for entry A");

    let certified_a = registry
        .certifier(&entry_a)
        .expect("entry A certifier")
        .certify(authored_a)
        .expect("entry A candidate");
    let certified_b = registry
        .certifier(&entry_b)
        .expect("entry B certifier")
        .certify(authored_b)
        .expect("entry B candidate");
    let a_manifest_ref = certified_a
        .document()
        .root
        .components
        .secret_free_implementation_manifest_closure_ref
        .clone();
    let b_manifest_ref = certified_b
        .document()
        .root
        .components
        .secret_free_implementation_manifest_closure_ref
        .clone();
    assert_ne!(a_manifest_ref, b_manifest_ref);
    let b_manifest = certified_b
        .document()
        .component_closure
        .iter()
        .find(|object| object.content_ref == b_manifest_ref)
        .expect("entry B implementation manifest")
        .clone();

    let mut substituted = certified_a.document().clone();
    let a_manifest_index = substituted
        .component_closure
        .iter()
        .position(|object| object.content_ref == a_manifest_ref)
        .expect("entry A implementation manifest");
    substituted.component_closure[a_manifest_index] = b_manifest.clone();
    substituted
        .root
        .components
        .secret_free_implementation_manifest_closure_ref = b_manifest.content_ref.clone();
    substituted.root.canonical_component_closure_digest =
        fixture_closure_digest(&substituted.root.components, &substituted.component_closure);
    registry
        .admission_verification_registry()
        .verify(&entry_a, &substituted)
        .expect_err("entry B implementation manifest cannot substitute into entry A");

    let mut unused = certified_a.document().clone();
    unused.component_closure.push(b_manifest);
    unused.root.canonical_component_closure_digest =
        fixture_closure_digest(&unused.root.components, &unused.component_closure);
    registry
        .admission_verification_registry()
        .verify(&entry_a, &unused)
        .expect_err("an unreferenced foreign implementation manifest must be rejected");
}

#[test]
fn admission_verifier_rejects_persisted_authority_mutation() {
    let operation_id = stable("mfm.fixture/mutation");
    let authored = copy_program(operation_id.clone());
    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<Request>()
        .expect("request contract");
    assembly
        .register_value::<Response>()
        .expect("response contract");
    assembly
        .register_fixture_state::<CopyState>(stable("mfm.fixture/copy-state-implementation"))
        .expect("copy state registration");
    assembly
        .register_entry_point(operation_id.clone(), authored.clone(), profile())
        .expect("entry-point registration");
    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified registry");
    let certified = registry
        .certifier(&operation_id)
        .expect("qualified certifier")
        .certify(authored)
        .expect("certified program");
    let mut hostile = certified.document().clone();
    hostile.component_closure.swap(0, 1);
    assert_eq!(
        hostile.content_ref().expect("closure-only hostile ref"),
        certified.reference().expect("certified ref"),
        "the sole authority ref hashes the root, whose closure digest binds the external closure"
    );

    let error = registry
        .admission_verifier(&operation_id)
        .expect("admission verifier")
        .verify(&hostile)
        .expect_err("closure order mutation must fail");
    assert!(error.to_string().contains("exact qualified recomputation"));

    let components = &certified.document().root.components;
    assert_ne!(
        components.state_capability_adapter_signer_resource_manifest_closure_ref,
        components.secret_free_implementation_manifest_closure_ref
    );
    let mut hostile = certified.document().clone();
    hostile
        .root
        .components
        .state_capability_adapter_signer_resource_manifest_closure_ref = components
        .secret_free_implementation_manifest_closure_ref
        .clone();
    hostile.root.canonical_component_closure_digest =
        fixture_closure_digest(&hostile.root.components, &hostile.component_closure);
    assert_ne!(
        hostile.content_ref().expect("semantic-root hostile ref"),
        certified.reference().expect("certified ref")
    );
    registry
        .admission_verifier(&operation_id)
        .expect("admission verifier")
        .verify(&hostile)
        .expect_err("implementation data cannot substitute for the semantic manifest root");

    let mut hostile = certified.document().clone();
    hostile
        .root
        .components
        .secret_free_implementation_manifest_closure_ref = components
        .state_capability_adapter_signer_resource_manifest_closure_ref
        .clone();
    hostile.root.canonical_component_closure_digest =
        fixture_closure_digest(&hostile.root.components, &hostile.component_closure);
    registry
        .admission_verifier(&operation_id)
        .expect("admission verifier")
        .verify(&hostile)
        .expect_err("semantic data cannot substitute for the implementation manifest root");

    let mut hostile = certified.document().clone();
    hostile.component_closure.retain(|object| {
        object.content_ref
            != components.state_capability_adapter_signer_resource_manifest_closure_ref
    });
    registry
        .admission_verifier(&operation_id)
        .expect("admission verifier")
        .verify(&hostile)
        .expect_err("a referenced manifest object cannot be absent");

    for object in &certified.document().component_closure {
        let encoded = serde_json::to_value(object).expect("component JSON");
        let fields = encoded.as_object().expect("component object fields");
        assert_eq!(fields.len(), 3);
        assert!(!fields.contains_key("outbound_references"));
    }

    let mut old_shape = serde_json::to_value(&certified.document().root).expect("root JSON");
    old_shape.as_object_mut().expect("root object").insert(
        "component_closure".to_owned(),
        serde_json::to_value(&certified.document().component_closure)
            .expect("retired closure JSON"),
    );
    serde_json::from_value::<mfm_spec::structured::CertifiedProgramRoot>(old_shape)
        .expect_err("a root cannot embed the retired full component closure");

    let mut hybrid = serde_json::to_value(&certified.document().root).expect("root JSON");
    let components = hybrid
        .get_mut("components")
        .and_then(serde_json::Value::as_object_mut)
        .expect("certified components");
    components.insert(
        "implementation_manifest_ref".to_owned(),
        serde_json::to_value(
            &certified
                .document()
                .root
                .components
                .secret_free_implementation_manifest_closure_ref,
        )
        .expect("legacy manifest ref JSON"),
    );
    serde_json::from_value::<mfm_spec::structured::CertifiedProgramRoot>(hybrid)
        .expect_err("a hybrid root with the retired one-manifest key must not decode");

    let mut sidecar =
        serde_json::to_value(&certified.document().component_closure[0]).expect("component JSON");
    let first_component = sidecar.as_object_mut().expect("first closure object");
    first_component.insert(
        "outbound_references".to_owned(),
        serde_json::Value::Array(Vec::new()),
    );
    serde_json::from_value::<mfm_spec::structured::CertifiedComponentObject>(sidecar)
        .expect_err("a retired caller-authored outbound-reference sidecar must not decode");
}

#[test]
fn fresh_verification_rejects_each_persisted_certification_layer_independently() {
    let operation_id = stable("mfm.fixture/persisted-layer-tamper");
    let authored = copy_program(operation_id.clone());
    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<Request>()
        .expect("request contract");
    assembly
        .register_value::<Response>()
        .expect("response contract");
    assembly
        .register_fixture_state::<CopyState>(stable("mfm.fixture/copy-state-implementation"))
        .expect("copy state registration");
    assembly
        .register_entry_point(operation_id.clone(), authored.clone(), profile())
        .expect("entry-point registration");
    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified registry");
    let certified = registry
        .certifier(&operation_id)
        .expect("qualified certifier")
        .certify(authored)
        .expect("certified program");
    let baseline = certified.document();

    let mut authored_attack = baseline.clone();
    let authored_ref = authored_attack.root.components.authored_program_ref.clone();
    let hostile_authored_ref =
        mutate_component_value(&mut authored_attack, &authored_ref, |value| {
            value["operation_id"] =
                serde_json::to_value(stable("mfm.fixture/foreign-authored-operation"))
                    .expect("foreign authored operation JSON");
        });
    authored_attack.root.components.authored_program_ref = hostile_authored_ref.clone();
    repoint_program_evidence(
        &mut authored_attack,
        "authored_program_ref",
        &hostile_authored_ref,
    );
    refresh_fixture_closure_digest(&mut authored_attack);

    let mut expanded_attack = baseline.clone();
    let expanded_ref = expanded_attack.root.components.expanded_program_ref.clone();
    let hostile_expanded_ref =
        mutate_component_value(&mut expanded_attack, &expanded_ref, |value| {
            value["operation_id"] =
                serde_json::to_value(stable("mfm.fixture/foreign-expanded-operation"))
                    .expect("foreign expanded operation JSON");
        });
    expanded_attack.root.components.expanded_program_ref = hostile_expanded_ref.clone();
    repoint_program_evidence(
        &mut expanded_attack,
        "expanded_program_ref",
        &hostile_expanded_ref,
    );
    refresh_fixture_closure_digest(&mut expanded_attack);

    let mut proof_attack = baseline.clone();
    let proof_ref = proof_attack.root.components.expansion_proof_ref.clone();
    let foreign_profile_ref = proof_attack.root.components.authored_program_ref.clone();
    let hostile_proof_ref = mutate_component_value(&mut proof_attack, &proof_ref, |value| {
        value["expansion_profile_ref"] =
            serde_json::to_value(&foreign_profile_ref).expect("foreign profile reference JSON");
    });
    proof_attack.root.components.expansion_proof_ref = hostile_proof_ref;
    refresh_fixture_closure_digest(&mut proof_attack);

    let mut root_attack = baseline.clone();
    root_attack
        .root
        .components
        .certified_structural_bounds
        .max_occurrences += 1;
    refresh_fixture_closure_digest(&mut root_attack);

    let mut document_attack = baseline.clone();
    document_attack.component_closure.rotate_left(1);

    for (label, hostile) in [
        ("authored object", authored_attack),
        ("expanded object", expanded_attack),
        ("expansion proof", proof_attack),
        ("certified root", root_attack),
        ("document closure", document_attack),
    ] {
        let result = registry
            .admission_verification_registry()
            .verify(&operation_id, &hostile);
        assert!(result.is_err(), "fresh verifier accepted tampered {label}");
    }
}

#[test]
fn admission_verifier_rejects_every_persisted_policy_authority_substitution() {
    let operation_id = stable("mfm.fixture/policy-authority-substitution");
    let authored = copy_program(operation_id.clone());
    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<Request>()
        .expect("request contract");
    assembly
        .register_value::<Response>()
        .expect("response contract");
    assembly
        .register_fixture_state::<CopyState>(stable("mfm.fixture/copy-state-implementation"))
        .expect("copy state registration");
    assembly
        .register_entry_point(operation_id.clone(), authored.clone(), profile())
        .expect("entry-point registration");
    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified registry");
    let certified = registry
        .certifier(&operation_id)
        .expect("qualified certifier")
        .certify(authored)
        .expect("certified program");
    let exact = &certified.document().root.components;

    for attack in [
        "caller-selected-policy",
        "unqualified-policy",
        "stale-profile",
        "unqualified-entry-contract",
    ] {
        let mut hostile = certified.document().clone();
        let components = &mut hostile.root.components;
        match attack {
            "caller-selected-policy" => {
                components.qualified_entry_point_admission_policy_ref =
                    exact.certified_program_contract_ref.clone();
            }
            "unqualified-policy" => {
                components.qualified_entry_point_admission_policy_ref =
                    exact.authored_program_ref.clone();
            }
            "stale-profile" => {
                components.expansion_profile_ref = exact.authored_program_ref.clone();
            }
            "unqualified-entry-contract" => {
                components.entry_point_contract_ref = exact.expanded_program_ref.clone();
            }
            _ => unreachable!("enumerated policy attack"),
        }
        hostile.root.canonical_component_closure_digest =
            fixture_closure_digest(&hostile.root.components, &hostile.component_closure);

        let result = registry
            .admission_verifier(&operation_id)
            .expect("admission verifier")
            .verify(&hostile);
        assert!(
            result.is_err(),
            "{attack} must not become admission authority"
        );
    }
}

#[test]
fn admission_verifier_rejects_well_formed_weaker_coverage_and_alternate_predicates() {
    let operation_id = stable("mfm.fixture/well-formed-policy-substitution");
    let authored = copy_program(operation_id.clone());
    let protected = state_contract::<CopyState>().expect("copy state contract");
    let recipe = identity_policy_recipe();
    let policy = ExpansionPolicyContract::new(vec![PolicyExpansionBinding {
        boundary_contract_ref: protected.state_contract_ref,
        recipe_ref: recipe.content_ref().expect("policy recipe ref"),
    }])
    .expect("policy contract");
    let mut exact_profile = profile();
    exact_profile.policies.push(policy);

    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<Request>()
        .expect("request contract");
    assembly
        .register_value::<Response>()
        .expect("response contract");
    assembly
        .register_fixture_state::<CopyState>(stable("mfm.fixture/copy-state-implementation"))
        .expect("copy state registration");
    assembly
        .register_policy_recipe(recipe)
        .expect("policy recipe registration");
    assembly
        .register_entry_point(operation_id.clone(), authored.clone(), exact_profile)
        .expect("entry-point registration");
    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified registry");
    let certified = registry
        .certifier(&operation_id)
        .expect("qualified certifier")
        .certify(authored)
        .expect("certified program");
    let verifier = registry
        .admission_verifier(&operation_id)
        .expect("admission verifier");
    assert_eq!(certified.policy_coverage_proof().entries.len(), 1);

    let mut weaker = certified.policy_coverage_proof().clone();
    weaker.entries.clear();
    let mut hostile = certified.document().clone();
    let exact_coverage_ref = hostile.root.components.policy_coverage_proof_ref.clone();
    let coverage_index = hostile
        .component_closure
        .iter()
        .position(|object| object.content_ref == exact_coverage_ref)
        .expect("exact coverage object");
    let weaker_coverage = fixture_component_with_same_schema(
        &hostile.component_closure[coverage_index],
        serde_json::to_value(&weaker).expect("weaker coverage JSON"),
    );
    assert_eq!(
        weaker_coverage.object_type.as_str(),
        "structured.policy_coverage_proof"
    );
    hostile.root.components.policy_coverage_proof_ref = weaker_coverage.content_ref.clone();
    hostile.component_closure[coverage_index] = weaker_coverage;
    hostile.root.canonical_component_closure_digest =
        fixture_closure_digest(&hostile.root.components, &hostile.component_closure);
    verifier
        .verify(&hostile)
        .expect_err("a well-formed weaker coverage proof must not become authority");

    let mut hostile = certified.document().clone();
    let exact_predicate_ref = hostile
        .root
        .components
        .certification_predicate_set_ref
        .clone();
    let predicate_index = hostile
        .component_closure
        .iter()
        .position(|object| object.content_ref == exact_predicate_ref)
        .expect("exact predicate-set object");
    let mut alternate_predicates = hostile.component_closure[predicate_index]
        .value
        .as_json()
        .clone();
    alternate_predicates
        .as_array_mut()
        .expect("predicate set array")
        .push(serde_json::Value::String(
            "alternate-reviewed-predicate-v1".to_owned(),
        ));
    let alternate_predicate = fixture_component_with_same_schema(
        &hostile.component_closure[predicate_index],
        alternate_predicates,
    );
    assert_eq!(
        alternate_predicate.object_type.as_str(),
        "structured.certification_predicate_set"
    );

    let exact_policy_ref = hostile
        .root
        .components
        .qualified_entry_point_admission_policy_ref
        .clone();
    let policy_index = hostile
        .component_closure
        .iter()
        .position(|object| object.content_ref == exact_policy_ref)
        .expect("exact admission-policy object");
    let mut alternate_policy = hostile.component_closure[policy_index]
        .value
        .as_json()
        .clone();
    alternate_policy
        .as_object_mut()
        .expect("admission-policy object")
        .insert(
            "certification_predicate_set_ref".to_owned(),
            serde_json::to_value(&alternate_predicate.content_ref)
                .expect("alternate predicate reference JSON"),
        );
    let alternate_policy = fixture_component_with_same_schema(
        &hostile.component_closure[policy_index],
        alternate_policy,
    );
    assert_eq!(
        alternate_policy.object_type.as_str(),
        "structured.admission_policy"
    );
    hostile.root.components.certification_predicate_set_ref =
        alternate_predicate.content_ref.clone();
    hostile
        .root
        .components
        .qualified_entry_point_admission_policy_ref = alternate_policy.content_ref.clone();
    hostile.component_closure[predicate_index] = alternate_predicate;
    hostile.component_closure[policy_index] = alternate_policy;
    hostile.root.canonical_component_closure_digest =
        fixture_closure_digest(&hostile.root.components, &hostile.component_closure);
    verifier
        .verify(&hostile)
        .expect_err("a valid proof cannot be evaluated under another predicate set");
}

#[test]
fn qualification_rejects_a_hostile_state_input_contract() {
    let operation_id = stable("mfm.fixture/hostile-state-input");
    let mut authored = copy_program(operation_id.clone());
    let mfm_spec::structured::AuthoredDeclaration::State(state) =
        &mut authored.root.declarations[0]
    else {
        panic!("copy fixture must contain one state");
    };
    state.inputs[0] = state.output_slot.clone();

    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<Request>()
        .expect("request contract");
    assembly
        .register_value::<Response>()
        .expect("response contract");
    assembly
        .register_fixture_state::<CopyState>(stable("mfm.fixture/copy-state-implementation"))
        .expect("copy state registration");
    assembly
        .register_entry_point(operation_id.clone(), authored, profile())
        .expect("hostile entry registration remains inert");

    let error = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect_err("a state cannot consume a slot with a foreign contract");
    assert!(error.to_string().contains("input contract"));
}

#[test]
fn qualification_rejects_a_forged_closed_sum_table() {
    let operation_id = stable("mfm.fixture/forged-closed-sum");
    let mut authored = guard_match_program(operation_id.clone());
    let mfm_spec::structured::AuthoredDeclaration::Match(binding) =
        &mut authored.root.declarations[1]
    else {
        panic!("guard fixture must contain one Match");
    };
    let mut forged_variants = binding.selector_contract.variants.clone();
    forged_variants.push(mfm_spec::structured::ClosedSumVariant {
        canonical_tag: "invented".to_owned(),
        payloads: Vec::new(),
    });
    binding.selector_contract = mfm_spec::structured::ClosedSumContract::new(
        binding.selector_contract.selector_contract_ref.clone(),
        forged_variants,
    )
    .expect("internally valid but unqualified forged table");

    let mut assembly = ProgramRegistryBuilder::new();
    for register in [
        ProgramRegistryBuilder::register_value::<Request>,
        ProgramRegistryBuilder::register_value::<Response>,
        ProgramRegistryBuilder::register_value::<StateFailure>,
        ProgramRegistryBuilder::register_closed_sum::<GuardDecision>,
    ] {
        register(&mut assembly).expect("fixture contract");
    }
    assembly
        .register_fixture_state::<GuardState>(stable("mfm.fixture/guard-state-implementation"))
        .expect("guard state registration");
    assembly
        .register_fixture_state::<CopyState>(stable("mfm.fixture/copy-state-implementation"))
        .expect("copy state registration");
    assembly
        .register_entry_point(operation_id.clone(), authored, profile())
        .expect("hostile entry registration remains inert");

    let error = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect_err("a caller-authored selector table must not become authority");
    assert!(error.to_string().contains("not process-qualified"));
}

#[test]
fn qualification_rejects_every_non_exact_match_selector() {
    for attack in [
        "future-selector",
        "non-dominating-selector",
        "structurally-similar-unregistered-sum",
    ] {
        let operation_id = stable(&format!("mfm.fixture/{attack}"));
        let mut authored = guard_match_program_with_nonlocal_selectors(operation_id.clone());
        let future_selector = match &authored.root.declarations[2] {
            AuthoredDeclaration::State(state) => state.output_slot.clone(),
            _ => panic!("fixture must define the later selector as a state"),
        };
        let branch_selector = match &authored.root.declarations[1] {
            AuthoredDeclaration::Match(binding) => match &binding.arms[0].body.declarations[0] {
                AuthoredDeclaration::State(state) => state.output_slot.clone(),
                _ => panic!("fixture must define the branch-local selector as a state"),
            },
            _ => panic!("guard fixture must contain one Match"),
        };
        let mfm_spec::structured::AuthoredDeclaration::Match(binding) =
            &mut authored.root.declarations[1]
        else {
            panic!("guard fixture must contain one Match");
        };
        match attack {
            "future-selector" => binding.selector = future_selector,
            "non-dominating-selector" => binding.selector = branch_selector,
            "structurally-similar-unregistered-sum" => {
                let mut variants = binding.selector_contract.variants.clone();
                variants.reverse();
                binding.selector_contract = mfm_spec::structured::ClosedSumContract::new(
                    binding.selector_contract.selector_contract_ref.clone(),
                    variants,
                )
                .expect("internally coherent alternate selector table");
                binding.arms.reverse();
            }
            _ => unreachable!("enumerated Match attack"),
        }

        let mut assembly = ProgramRegistryBuilder::new();
        for register in [
            ProgramRegistryBuilder::register_value::<Request>,
            ProgramRegistryBuilder::register_value::<Response>,
            ProgramRegistryBuilder::register_value::<StateFailure>,
            ProgramRegistryBuilder::register_closed_sum::<GuardDecision>,
        ] {
            register(&mut assembly).expect("fixture contract");
        }
        assembly
            .register_fixture_state::<GuardState>(stable("mfm.fixture/guard-state-implementation"))
            .expect("guard state registration");
        assembly
            .register_fixture_state::<CopyState>(stable("mfm.fixture/copy-state-implementation"))
            .expect("copy state registration");
        assembly
            .register_entry_point(operation_id.clone(), authored, profile())
            .expect("hostile entry registration remains inert");

        let result = assembly.build(std::slice::from_ref(&operation_id));
        assert!(
            result.is_err(),
            "{attack} must not qualify as a Match selector"
        );
    }
}

#[test]
fn qualification_rejects_serialized_policy_proceed_inside_fan_out() {
    let operation_id = stable("mfm.fixture/hostile-fan-out-proceed");
    let authored = copy_program(operation_id.clone());
    let recipe = identity_policy_recipe();
    let mut hostile_program = recipe.program().clone();
    let proceed = hostile_program.root.declarations.remove(0);
    let mut template = fan_out_program(stable("mfm.fixture/hostile-fan-out-template"));
    let AuthoredDeclaration::FanOut(fan_out) = &mut template.root.declarations[0] else {
        unreachable!("fan-out fixture shape")
    };
    fan_out.lanes[0].body.declarations[0] = proceed;
    hostile_program.root.declarations = template.root.declarations;
    let mut recipe_json = serde_json::to_value(recipe).expect("policy recipe JSON");
    recipe_json["program"] =
        serde_json::to_value(hostile_program).expect("hostile normalized policy program");
    let hostile_recipe: PolicyExpansionRecipe =
        serde_json::from_value(recipe_json).expect("hostile serialized policy recipe");
    let protected = state_contract::<CopyState>().expect("copy state contract");
    let policy = ExpansionPolicyContract::new(vec![PolicyExpansionBinding {
        boundary_contract_ref: protected.state_contract_ref,
        recipe_ref: hostile_recipe.content_ref().expect("hostile recipe ref"),
    }])
    .expect("hostile policy contract");
    let mut hostile_profile = profile();
    hostile_profile.policies.push(policy);

    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<Request>()
        .expect("request contract");
    assembly
        .register_value::<Response>()
        .expect("response contract");
    assembly
        .register_fixture_state::<CopyState>(stable("mfm.fixture/copy-state-implementation"))
        .expect("copy state registration");
    assembly
        .register_policy_recipe(hostile_recipe)
        .expect("hostile recipe is inert registered data");
    assembly
        .register_entry_point(operation_id.clone(), authored, hostile_profile)
        .expect("entry-point registration");

    let error = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect_err("policy proceed cannot cross a fan-out lane boundary");
    assert!(
        error
            .to_string()
            .contains("policy proceed cannot be captured inside FanOut"),
        "{error}"
    );
}

#[test]
fn policy_wraps_an_eligible_state_already_authored_inside_a_fan_out_lane() {
    let operation_id = stable("mfm.fixture/policy-in-existing-fan-out");
    let authored = fan_out_program(operation_id.clone());
    let recipe = identity_policy_recipe();
    let protected = state_contract::<LaneAState>().expect("lane state contract");
    let policy = ExpansionPolicyContract::new(vec![PolicyExpansionBinding {
        boundary_contract_ref: protected.state_contract_ref,
        recipe_ref: recipe.content_ref().expect("recipe ref"),
    }])
    .expect("policy contract");
    let mut qualified_profile = profile();
    qualified_profile.policies.push(policy);

    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<Request>()
        .expect("request contract");
    assembly
        .register_value::<Response>()
        .expect("response contract");
    assembly
        .register_fixture_state::<LaneAState>(stable("mfm.fixture/lane-a-implementation"))
        .expect("lane A state");
    assembly
        .register_fixture_state::<LaneBState>(stable("mfm.fixture/lane-b-implementation"))
        .expect("lane B state");
    assembly
        .register_policy_recipe(recipe)
        .expect("policy recipe");
    assembly
        .register_entry_point(operation_id.clone(), authored.clone(), qualified_profile)
        .expect("entry point");
    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified policy in fan-out");
    let certified = registry
        .certifier(&operation_id)
        .expect("certifier")
        .certify(authored)
        .expect("certified policy in fan-out");
    assert_eq!(
        certified
            .expansion_proof()
            .substitution_trace
            .iter()
            .filter(|entry| entry.stage == ExpansionStage::PolicyWrapping)
            .count(),
        1
    );
}

#[test]
fn declarative_policy_wraps_the_exact_eligible_boundary_once() {
    let operation_id = stable("mfm.fixture/policy");
    let authored = copy_program(operation_id.clone());
    let state_contract = state_contract::<CopyState>().expect("copy state contract");
    let recipe = identity_policy_recipe();
    let policy = ExpansionPolicyContract::new(vec![PolicyExpansionBinding {
        boundary_contract_ref: state_contract.state_contract_ref.clone(),
        recipe_ref: recipe.content_ref().expect("recipe ref"),
    }])
    .expect("policy contract");
    let mut policy_profile = profile();
    policy_profile.policies.push(policy.clone());

    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<Request>()
        .expect("request contract");
    assembly
        .register_value::<Response>()
        .expect("response contract");
    assembly
        .register_fixture_state::<CopyState>(stable("mfm.fixture/copy-state-implementation"))
        .expect("copy state registration");
    assembly
        .register_policy_recipe(recipe)
        .expect("policy recipe registration");
    assembly
        .register_entry_point(operation_id.clone(), authored.clone(), policy_profile)
        .expect("entry-point registration");
    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified registry");
    let certified = registry
        .certifier(&operation_id)
        .expect("qualified certifier")
        .certify(authored)
        .expect("certified program");

    assert_eq!(certified.policy_coverage_proof().entries.len(), 1);
    assert_eq!(
        certified.policy_coverage_proof().entries[0].policy_ref,
        policy.policy_ref
    );
    assert_eq!(certified.expansion_proof().substitution_trace.len(), 1);
    assert!(matches!(
        certified.expanded().root.declarations[0],
        ExpandedDeclaration::Fragment(_)
    ));
}

#[test]
fn typed_policy_preserves_one_eventual_failure_handler() {
    let operation_id = stable("mfm.fixture/typed-policy");
    let authored = fallible_program(operation_id.clone());
    let protected_contract = state_contract::<FallibleState>().expect("fallible state contract");
    let recipe = typed_identity_policy_recipe();
    let policy = ExpansionPolicyContract::new(vec![PolicyExpansionBinding {
        boundary_contract_ref: protected_contract.state_contract_ref.clone(),
        recipe_ref: recipe.content_ref().expect("recipe ref"),
    }])
    .expect("policy contract");
    let mut policy_profile = profile();
    policy_profile.policies.push(policy);

    let mut assembly = ProgramRegistryBuilder::new();
    register_fixture_runtime_components(&mut assembly, false);
    for register in [
        ProgramRegistryBuilder::register_value::<Request>,
        ProgramRegistryBuilder::register_value::<Response>,
        ProgramRegistryBuilder::register_value::<StateFailure>,
        ProgramRegistryBuilder::register_value::<RootFailure>,
        ProgramRegistryBuilder::register_closed_sum::<DefaultRoute>,
    ] {
        register(&mut assembly).expect("fixture value contract");
    }
    assembly
        .register_fixture_state::<FallibleState>(stable(
            "mfm.fixture/fallible-state-implementation",
        ))
        .expect("fallible state registration");
    assembly
        .register_fixture_state::<RootFailureMapperState>(stable(
            "mfm.fixture/root-failure-mapper-implementation",
        ))
        .expect("mapper registration");
    assembly
        .register_policy_recipe(recipe)
        .expect("policy recipe registration");
    assembly
        .register_entry_point(operation_id.clone(), authored, policy_profile)
        .expect("entry-point registration");

    assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("typed policy must retain one complete handler chain");
}

#[test]
fn capability_lowering_is_exact_callback_free_and_registration_order_independent() {
    let operation_id = stable("mfm.fixture/capability");
    let authored = capability_program(operation_id.clone());
    let first = certify_capability_fixture(&operation_id, &authored, false);
    let second = certify_capability_fixture(&operation_id, &authored, true);

    assert_eq!(first.document(), second.document());
    assert_eq!(first.expansion_proof().substitution_trace.len(), 1);
    assert_eq!(
        first.expansion_proof().substitution_trace[0].stage,
        ExpansionStage::CapabilityLowering
    );
    let ExpandedDeclaration::Fragment(fragment) = &first.expanded().root.declarations[0] else {
        panic!("abstract capability must lower to one ordinary fragment");
    };
    let ExpandedDeclaration::State(state) = &fragment.body.declarations[0] else {
        panic!("capability recipe must contain its concrete state");
    };
    assert_eq!(
        state.contract.state_contract_ref,
        state_contract::<ConcreteCapabilityState>()
            .expect("concrete contract")
            .state_contract_ref
    );
    assert_ne!(
        state.contract.state_contract_ref,
        state_contract::<AbstractCapabilityState>()
            .expect("abstract contract")
            .state_contract_ref
    );
}

#[test]
fn capability_support_children_preserve_namespaced_identity_trace_and_policy_exclusion() {
    let operation_id = stable("mfm.fixture/capability-support-child");
    let authored = child_capability_program(operation_id.clone(), 2);
    let policy_recipe = identity_policy_recipe();
    let policy = ExpansionPolicyContract::new(vec![PolicyExpansionBinding {
        boundary_contract_ref: state_contract::<CopyState>()
            .expect("copy state contract")
            .state_contract_ref,
        recipe_ref: policy_recipe.content_ref().expect("policy recipe ref"),
    }])
    .expect("support-state policy");
    let mut expansion_profile = profile();
    expansion_profile.policies.push(policy);

    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<Request>()
        .expect("request contract");
    assembly
        .register_value::<Response>()
        .expect("response contract");
    assembly
        .register_fixture_state::<CopyState>(stable("mfm.fixture/copy-state-implementation"))
        .expect("copy state registration");
    assembly
        .register_child(copy_child_program())
        .expect("copy child registration");
    assembly
        .register_capability_state::<ChildCapabilityState>()
        .expect("child capability state registration");
    assembly
        .register_capability_expansion::<ChildCapabilityRecipeExpansion>()
        .expect("child capability recipe registration");
    assembly
        .register_policy_recipe(policy_recipe)
        .expect("support-state policy recipe registration");
    assembly
        .register_entry_point(operation_id.clone(), authored.clone(), expansion_profile)
        .expect("entry-point registration");

    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified child support registry");
    let certified = registry
        .certifier(&operation_id)
        .expect("qualified certifier")
        .certify(authored)
        .expect("certified child support program");

    assert!(certified.policy_coverage_proof().entries.is_empty());
    assert_eq!(certified.expansion_proof().substitution_trace.len(), 4);
    assert_eq!(
        certified
            .expansion_proof()
            .substitution_trace
            .iter()
            .filter(|entry| entry.stage == ExpansionStage::ChildSubstitution)
            .count(),
        2
    );
    assert_eq!(
        certified
            .expansion_proof()
            .substitution_trace
            .iter()
            .filter(|entry| entry.stage == ExpansionStage::CapabilityLowering)
            .count(),
        2
    );
    assert!(certified
        .expansion_proof()
        .substitution_trace
        .iter()
        .all(|entry| entry.stage != ExpansionStage::PolicyWrapping));

    let capability_ref = state_contract::<ChildCapabilityState>()
        .expect("child capability state contract")
        .capability_requirement_ref
        .expect("child capability requirement");
    let child_ref = copy_child_program().content_ref().expect("copy child ref");
    let mut child_call_ids = Vec::new();
    let mut support_state_ids = Vec::new();
    for declaration in &certified.expanded().root.declarations {
        let ExpandedDeclaration::Fragment(capability) = declaration else {
            panic!("capability call must lower to a fragment");
        };
        let [ExpandedDeclaration::Fragment(child)] = capability.body.declarations.as_slice() else {
            panic!("capability support must contain one child fragment");
        };
        let [ExpandedDeclaration::State(state)] = child.body.declarations.as_slice() else {
            panic!("support child must contain one state");
        };
        child_call_ids.push(child.semantic_call_id.clone());
        support_state_ids.push(state.semantic_call_id.clone());

        assert!(certified
            .expansion_proof()
            .substitution_trace
            .iter()
            .any(|entry| entry.stage == ExpansionStage::CapabilityLowering
                && entry.semantic_call_id == capability.semantic_call_id
                && entry.expansion_ref == capability_ref
                && entry.boundary_id
                    == ExpansionBoundaryId::Fragment(capability.boundary_id.clone())));
        assert!(certified
            .expansion_proof()
            .substitution_trace
            .iter()
            .any(|entry| entry.stage == ExpansionStage::ChildSubstitution
                && entry.semantic_call_id == child.semantic_call_id
                && entry.expansion_ref == child_ref
                && entry.boundary_id == ExpansionBoundaryId::Fragment(child.boundary_id.clone())));
    }
    assert_ne!(child_call_ids[0], child_call_ids[1]);
    assert_ne!(support_state_ids[0], support_state_ids[1]);
}

#[test]
fn capability_support_allows_nested_acyclic_registered_children() {
    let operation_id = stable("mfm.fixture/nested-capability-support-child");
    let authored = nested_child_capability_program(operation_id.clone());
    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<Request>()
        .expect("request contract");
    assembly
        .register_value::<Response>()
        .expect("response contract");
    assembly
        .register_fixture_state::<CopyState>(stable("mfm.fixture/copy-state-implementation"))
        .expect("copy state registration");
    assembly
        .register_child(copy_child_program())
        .expect("copy child registration");
    assembly
        .register_child(nested_copy_child_program())
        .expect("nested child registration");
    assembly
        .register_capability_state::<NestedChildCapabilityState>()
        .expect("nested child capability state registration");
    assembly
        .register_capability_expansion::<NestedChildCapabilityRecipeExpansion>()
        .expect("nested child capability recipe registration");
    assembly
        .register_entry_point(operation_id.clone(), authored.clone(), profile())
        .expect("entry-point registration");

    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("nested child support registry");
    let certified = registry
        .certifier(&operation_id)
        .expect("qualified certifier")
        .certify(authored)
        .expect("nested child support certification");
    assert_eq!(
        certified
            .expansion_proof()
            .substitution_trace
            .iter()
            .filter(|entry| entry.stage == ExpansionStage::ChildSubstitution)
            .count(),
        2
    );
    assert_eq!(
        certified
            .expansion_proof()
            .substitution_trace
            .iter()
            .filter(|entry| entry.stage == ExpansionStage::CapabilityLowering)
            .count(),
        1
    );
    let [ExpandedDeclaration::Fragment(capability)] =
        certified.expanded().root.declarations.as_slice()
    else {
        panic!("capability lowering fragment");
    };
    let [ExpandedDeclaration::Fragment(outer_child)] = capability.body.declarations.as_slice()
    else {
        panic!("outer support child fragment");
    };
    let [ExpandedDeclaration::Fragment(inner_child)] = outer_child.body.declarations.as_slice()
    else {
        panic!("inner support child fragment");
    };
    assert!(matches!(
        inner_child.body.declarations.as_slice(),
        [ExpandedDeclaration::State(_)]
    ));
}

#[test]
fn capability_support_child_custom_recovery_rebinds_its_authored_payloads() {
    let operation_id = stable("mfm.fixture/recovering-capability-support-child");
    let authored = recovering_child_capability_program(operation_id.clone());
    let mut assembly = ProgramRegistryBuilder::new();
    register_fixture_runtime_components(&mut assembly, false);
    assembly
        .register_closed_sum::<StateFailureRoute>()
        .expect("state failure route contract");
    assembly
        .register_closed_sum::<RecoveryRoute>()
        .expect("recovery route contract");
    assembly
        .register_fixture_state::<FallibleState>(stable(
            "mfm.fixture/fallible-state-implementation",
        ))
        .expect("fallible state registration");
    assembly
        .register_fixture_state::<StateFailureMapperState>(stable(
            "mfm.fixture/state-failure-mapper-implementation",
        ))
        .expect("state failure mapper registration");
    assembly
        .register_fixture_state::<RecoveryHandlerState>(stable(
            "mfm.fixture/recovery-handler-implementation",
        ))
        .expect("recovery handler registration");
    assembly
        .register_fixture_state::<RecoveryReadState>(stable(
            "mfm.fixture/recovery-read-implementation",
        ))
        .expect("recovery read registration");
    assembly
        .register_child(recoverable_child_program())
        .expect("recoverable child registration");
    assembly
        .register_capability_state::<RecoveringChildCapabilityState>()
        .expect("recovering capability state registration");
    assembly
        .register_capability_expansion::<RecoveringChildCapabilityRecipeExpansion>()
        .expect("recovering capability recipe registration");
    assembly
        .register_entry_point(operation_id.clone(), authored.clone(), profile())
        .expect("entry-point registration");

    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("recovering child support registry");
    let certified = registry
        .certifier(&operation_id)
        .expect("qualified certifier")
        .certify(authored)
        .expect("recovering child support certification");
    assert!(certified
        .expansion_proof()
        .substitution_trace
        .iter()
        .any(|entry| entry.stage == ExpansionStage::ChildSubstitution));
    assert!(certified
        .expansion_proof()
        .substitution_trace
        .iter()
        .any(|entry| entry.stage == ExpansionStage::CapabilityLowering));
}

#[test]
fn capability_support_child_rejects_recursive_capability_and_missing_child() {
    let recursive_id = stable("mfm.fixture/recursive-capability-support-child");
    let mut recursive = ProgramRegistryBuilder::new();
    recursive
        .register_value::<Request>()
        .expect("request contract");
    recursive
        .register_value::<Response>()
        .expect("response contract");
    recursive
        .register_capability_state::<RecursiveChildCapabilityState>()
        .expect("outer capability state registration");
    recursive
        .register_capability_state::<AbstractCapabilityState>()
        .expect("nested capability state registration");
    recursive
        .register_child(recursive_support_child_program())
        .expect("recursive support child registration");
    recursive
        .register_capability_expansion::<RecursiveChildCapabilityRecipeExpansion>()
        .expect("recursive child recipe registration");
    recursive
        .register_entry_point(
            recursive_id.clone(),
            recursive_child_capability_program(recursive_id.clone()),
            profile(),
        )
        .expect("recursive entry registration");
    let error = recursive
        .build(std::slice::from_ref(&recursive_id))
        .expect_err("support child cannot request another capability expansion");
    assert!(
        error
            .to_string()
            .contains("injected support state requests recursive capability expansion"),
        "{error}"
    );

    let missing_id = stable("mfm.fixture/missing-capability-support-child");
    let mut missing = ProgramRegistryBuilder::new();
    missing
        .register_value::<Request>()
        .expect("request contract");
    missing
        .register_value::<Response>()
        .expect("response contract");
    missing
        .register_capability_state::<ChildCapabilityState>()
        .expect("child capability state registration");
    missing
        .register_capability_expansion::<ChildCapabilityRecipeExpansion>()
        .expect("child capability recipe registration");
    missing
        .register_entry_point(
            missing_id.clone(),
            child_capability_program(missing_id.clone(), 1),
            profile(),
        )
        .expect("missing-child entry registration");
    let error = missing
        .build(std::slice::from_ref(&missing_id))
        .expect_err("capability support child must be registered");
    assert!(
        error
            .to_string()
            .contains("authored child call is not exact and qualified"),
        "{error}"
    );
}

#[test]
fn policy_expansion_still_rejects_child_operations() {
    let operation_id = stable("mfm.fixture/policy-support-child");
    let authored = copy_program(operation_id.clone());
    let recipe = child_policy_recipe();
    let policy = ExpansionPolicyContract::new(vec![PolicyExpansionBinding {
        boundary_contract_ref: state_contract::<CopyState>()
            .expect("copy state contract")
            .state_contract_ref,
        recipe_ref: recipe.content_ref().expect("policy recipe ref"),
    }])
    .expect("child policy");
    let mut expansion_profile = profile();
    expansion_profile.policies.push(policy);

    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<Request>()
        .expect("request contract");
    assembly
        .register_value::<Response>()
        .expect("response contract");
    assembly
        .register_fixture_state::<CopyState>(stable("mfm.fixture/copy-state-implementation"))
        .expect("copy state registration");
    assembly
        .register_child(copy_child_program())
        .expect("copy child registration");
    assembly
        .register_policy_recipe(recipe)
        .expect("child policy recipe registration");
    assembly
        .register_entry_point(operation_id.clone(), authored, expansion_profile)
        .expect("entry-point registration");
    let error = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect_err("policy support must not contain child operations");
    assert!(
        error
            .to_string()
            .contains("expansion support cannot contain child operations"),
        "{error}"
    );
}

#[test]
fn capability_expansion_rejects_recursive_support_requirements() {
    let operation_id = stable("mfm.fixture/recursive-capability-expansion");
    let authored = recursive_capability_program(operation_id.clone());
    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<Request>()
        .expect("request contract");
    assembly
        .register_value::<Response>()
        .expect("response contract");
    assembly
        .register_capability_state::<RecursiveCapabilityState>()
        .expect("outer abstract state");
    assembly
        .register_capability_state::<AbstractCapabilityState>()
        .expect("nested abstract state");
    assembly
        .register_capability_expansion::<RecursiveCapabilityRecipeExpansion>()
        .expect("recursive recipe remains inert until expansion");
    assembly
        .register_entry_point(operation_id.clone(), authored, profile())
        .expect("recursive entry remains inert");

    let error = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect_err("support states cannot recursively request capability expansion");
    assert!(
        error
            .to_string()
            .contains("injected support state requests recursive capability expansion"),
        "{error}"
    );
}

#[test]
fn qualification_rejects_each_missing_live_component_dependency() {
    let runtime_components = [
        fixture_capability_contract().expect("capability contract"),
        fixture_adapter_contract().expect("adapter contract"),
        fixture_signer_contract().expect("signer contract"),
        fixture_resource_contract().expect("resource contract"),
    ];
    for available_components in 0..runtime_components.len() {
        let operation_id = stable(&format!(
            "mfm.fixture/missing-live-component-{available_components}"
        ));
        let mut assembly = ProgramRegistryBuilder::new();
        for register in [
            ProgramRegistryBuilder::register_value::<Request>,
            ProgramRegistryBuilder::register_value::<Response>,
            ProgramRegistryBuilder::register_value::<StateFailure>,
            ProgramRegistryBuilder::register_value::<RootFailure>,
            ProgramRegistryBuilder::register_closed_sum::<DefaultRoute>,
        ] {
            register(&mut assembly).expect("fixture value contract");
        }
        assembly
            .register_fixture_state::<FallibleState>(stable(
                "mfm.fixture/fallible-state-implementation",
            ))
            .expect("fallible state registration");
        assembly
            .register_fixture_state::<RootFailureMapperState>(stable(
                "mfm.fixture/root-failure-mapper-implementation",
            ))
            .expect("failure mapper registration");
        for component_index in 0..available_components {
            match component_index {
                0 => register_fixture_capability(&mut assembly),
                1 => register_fixture_adapter(&mut assembly),
                2 => register_fixture_signer(&mut assembly),
                3 => register_fixture_resource(&mut assembly),
                _ => unreachable!("fixture has exactly four live components"),
            }
        }
        assembly
            .register_entry_point(
                operation_id.clone(),
                fallible_program(operation_id.clone()),
                profile(),
            )
            .expect("entry-point registration");
        let error = assembly
            .build(std::slice::from_ref(&operation_id))
            .expect_err("the live dependency graph must be complete");
        assert!(error
            .to_string()
            .contains("live semantic component dependency is not process-qualified"));
    }
}

#[test]
fn child_substitution_requires_the_exact_root_bijection() {
    let operation_id = stable("mfm.fixture/child-parent");
    let authored = child_parent_program(operation_id.clone(), stable("child-request"));
    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<Request>()
        .expect("request contract");
    assembly
        .register_value::<Response>()
        .expect("response contract");
    assembly
        .register_fixture_state::<CopyState>(stable("mfm.fixture/copy-state-implementation"))
        .expect("copy state registration");
    assembly
        .register_child(copy_child_program())
        .expect("child registration");
    assembly
        .register_entry_point(operation_id.clone(), authored.clone(), profile())
        .expect("entry-point registration");
    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified registry");
    let certified = registry
        .certifier(&operation_id)
        .expect("qualified certifier")
        .certify(authored)
        .expect("certified child program");
    assert_eq!(certified.expansion_proof().substitution_trace.len(), 1);
    assert_eq!(
        certified.expansion_proof().substitution_trace[0].stage,
        ExpansionStage::ChildSubstitution
    );
    let ExpandedDeclaration::Fragment(fragment) = &certified.expanded().root.declarations[0] else {
        panic!("child substitution must produce one fragment");
    };
    assert!(fragment
        .boundary_id
        .as_str()
        .starts_with("fragment-boundary:"));
    let ExpandedDeclaration::State(child_state) = &fragment.body.declarations[0] else {
        panic!("copy child must contain one executable state");
    };
    assert!(child_state
        .occurrence_id
        .as_str()
        .starts_with("occurrence:"));
    assert_eq!(
        certified.expansion_proof().substitution_trace[0].boundary_id,
        ExpansionBoundaryId::Fragment(fragment.boundary_id.clone())
    );

    for hostile_kind in [
        "missing",
        "duplicate",
        "extra",
        "foreign_root",
        "inactive",
        "wrong_role",
        "input_contract",
        "success_role",
        "success_contract",
    ] {
        let hostile_id = stable(&format!("mfm.fixture/child-parent-{hostile_kind}"));
        let mut hostile = child_parent_program(hostile_id.clone(), stable("child-request"));
        let mfm_spec::structured::AuthoredDeclaration::OperationCall(call) =
            &mut hostile.root.declarations[0]
        else {
            panic!("child fixture must contain one operation call");
        };
        match hostile_kind {
            "missing" => call.input_bindings.clear(),
            "duplicate" => call.input_bindings.push(call.input_bindings[0].clone()),
            "extra" => {
                let mut extra = call.input_bindings[0].clone();
                extra.child_root_id = stable("extra-child-root");
                call.input_bindings.push(extra);
            }
            "foreign_root" => {
                call.input_bindings[0].child_root_id = stable("foreign-root");
            }
            "inactive" => {
                let mut inactive = call.output_slot.clone();
                inactive.contract_ref =
                    mfm_spec::structured::structured_value_contract_ref::<Request>()
                        .expect("request contract ref");
                call.input_bindings[0].caller_slot = inactive;
            }
            "wrong_role" => {
                call.input_bindings[0].caller_slot.producer =
                    mfm_spec::structured::LexicalProducer::AuthoredCallOutput {
                        semantic_call_id: call.semantic_call_id.clone(),
                        role: mfm_spec::structured::ResultRole::TypedFailure,
                    };
            }
            "input_contract" => {
                call.input_bindings[0].child_contract_ref =
                    mfm_spec::structured::structured_value_contract_ref::<Response>()
                        .expect("response contract ref");
            }
            "success_role" => {
                let mfm_spec::structured::LexicalProducer::AuthoredCallOutput { role, .. } =
                    &mut call.output_slot.producer
                else {
                    panic!("authored child output producer");
                };
                *role = mfm_spec::structured::ResultRole::TypedFailure;
            }
            "success_contract" => {
                call.output_slot.contract_ref =
                    mfm_spec::structured::structured_value_contract_ref::<Request>()
                        .expect("request contract ref");
            }
            _ => unreachable!(),
        }

        let mut hostile_assembly = ProgramRegistryBuilder::new();
        hostile_assembly
            .register_value::<Request>()
            .expect("request contract");
        hostile_assembly
            .register_value::<Response>()
            .expect("response contract");
        hostile_assembly
            .register_fixture_state::<CopyState>(stable("mfm.fixture/copy-state-implementation"))
            .expect("copy state registration");
        hostile_assembly
            .register_child(copy_child_program())
            .expect("child registration");
        hostile_assembly
            .register_entry_point(hostile_id, hostile, profile())
            .expect("hostile entry registration is inert data");
        hostile_assembly
            .build(std::slice::from_ref(&operation_id))
            .expect_err("hostile child substitution must fail qualification");
    }
}

#[test]
fn typed_child_failure_crosses_one_exact_fragment_boundary_into_the_parent_handler() {
    let operation_id = stable("mfm.fixture/typed-child-failure");
    let (registry, certified) = certify_typed_child_fixture(&operation_id);
    let [ExpandedDeclaration::Fragment(fragment)] =
        certified.expanded().root.declarations.as_slice()
    else {
        panic!("typed child must substitute to one fragment");
    };
    let mfm_spec::structured::BlockTail::Normal(body_success) = &fragment.body.tail else {
        panic!("child body must retain its normal tail");
    };
    assert!(matches!(
        &fragment.success_slot.producer,
        mfm_spec::structured::LexicalProducer::FragmentBoundary {
            boundary_id,
            role: mfm_spec::structured::ResultRole::SuccessOutput,
            source,
        } if boundary_id == &fragment.boundary_id && source.as_ref() == body_success
    ));
    let mfm_spec::structured::CertifiedFailureBoundary::Typed {
        failure_contract,
        source_slot,
        ..
    } = &fragment.failure_boundary
    else {
        panic!("typed child must retain one failure boundary");
    };
    assert_eq!(
        source_slot.contract_ref,
        failure_contract.contract_ref().unwrap()
    );
    assert!(matches!(
        &source_slot.producer,
        mfm_spec::structured::LexicalProducer::FragmentBoundary {
            boundary_id,
            role: mfm_spec::structured::ResultRole::TypedFailure,
            source,
        } if boundary_id == &fragment.boundary_id
            && matches!(
                &source.producer,
                mfm_spec::structured::LexicalProducer::ScopeFailureMerge {
                    declaration_ordered_failure_slots,
                    ..
                } if declaration_ordered_failure_slots == &fragment.body.failure_exits
            )
    ));

    let mut oracle = ExecutionOracle::new(true, true);
    assert_eq!(
        oracle
            .execute(certified.expanded(), Request { value: 4 })
            .expect("nested child failure execution"),
        OracleOutcome::Failure(RootFailure { code: 7 })
    );
    assert_eq!(
        oracle.calls.last().map(String::as_str),
        Some("mfm.fixture/root-failure-mapper")
    );
    registry
        .admission_verifier(&operation_id)
        .expect("typed child verifier")
        .verify(certified.document())
        .expect("exact typed child document");

    for hostile_kind in ["failure_contract", "failure_without_plan"] {
        let hostile_id = stable(&format!("mfm.fixture/typed-child-{hostile_kind}"));
        let mut hostile = typed_child_parent_program(hostile_id.clone());
        let mfm_spec::structured::AuthoredDeclaration::OperationCall(call) =
            &mut hostile.root.declarations[0]
        else {
            panic!("typed child fixture must contain one operation call");
        };
        match hostile_kind {
            "failure_contract" => {
                call.failure_contract = mfm_spec::structured::StructuredFailureContract::Never;
            }
            "failure_without_plan" => {
                call.failure_directive = mfm_spec::structured::AuthoredFailureDirective::NoFailure;
            }
            _ => unreachable!(),
        }

        let mut assembly = ProgramRegistryBuilder::new();
        register_fixture_runtime_components(&mut assembly, false);
        for register in [
            ProgramRegistryBuilder::register_value::<Request>,
            ProgramRegistryBuilder::register_value::<Response>,
            ProgramRegistryBuilder::register_value::<StateFailure>,
            ProgramRegistryBuilder::register_value::<RootFailure>,
            ProgramRegistryBuilder::register_closed_sum::<StateFailureRoute>,
            ProgramRegistryBuilder::register_closed_sum::<DefaultRoute>,
        ] {
            register(&mut assembly).expect("typed child value contract");
        }
        assembly
            .register_capability_state::<AbstractFallibleCapabilityState>()
            .expect("abstract child state registration");
        assembly
            .register_fixture_state::<FallibleState>(stable(
                "mfm.fixture/fallible-state-implementation",
            ))
            .expect("fallible child state registration");
        assembly
            .register_fixture_state::<StateFailureMapperState>(stable(
                "mfm.fixture/state-failure-mapper-implementation",
            ))
            .expect("child failure mapper registration");
        assembly
            .register_fixture_state::<RootFailureMapperState>(stable(
                "mfm.fixture/root-failure-mapper-implementation",
            ))
            .expect("parent failure mapper registration");
        assembly
            .register_capability_expansion::<FallibleCapabilityRecipeExpansion>()
            .expect("child capability recipe registration");
        assembly
            .register_child(pipeline_child_program())
            .expect("typed child registration");
        assembly
            .register_entry_point(hostile_id, hostile, profile())
            .expect("hostile typed child entry remains inert");
        assembly
            .build(std::slice::from_ref(&operation_id))
            .expect_err("typed child boundary substitution must fail");
    }
}

#[test]
fn failure_post_is_bound_only_to_the_protected_affine_failure_plan() {
    let operation_id = stable("mfm.fixture/failure-post-policy");
    let authored = fallible_program(operation_id.clone());
    let protected_contract = state_contract::<FallibleState>().expect("fallible state contract");
    let recipe = typed_policy_recipe_with_failure_post();
    let policy = ExpansionPolicyContract::new(vec![PolicyExpansionBinding {
        boundary_contract_ref: protected_contract.state_contract_ref,
        recipe_ref: recipe.content_ref().expect("recipe ref"),
    }])
    .expect("policy contract");
    let mut policy_profile = profile();
    policy_profile.policies.push(policy);

    let mut assembly = ProgramRegistryBuilder::new();
    register_fixture_runtime_components(&mut assembly, false);
    for register in [
        ProgramRegistryBuilder::register_value::<Request>,
        ProgramRegistryBuilder::register_value::<Response>,
        ProgramRegistryBuilder::register_value::<StateFailure>,
        ProgramRegistryBuilder::register_value::<RootFailure>,
        ProgramRegistryBuilder::register_closed_sum::<DefaultRoute>,
    ] {
        register(&mut assembly).expect("fixture value contract");
    }
    assembly
        .register_fixture_state::<FallibleState>(stable(
            "mfm.fixture/fallible-state-implementation",
        ))
        .expect("fallible state registration");
    assembly
        .register_fixture_state::<RootFailureMapperState>(stable(
            "mfm.fixture/root-failure-mapper-implementation",
        ))
        .expect("mapper registration");
    assembly
        .register_fixture_state::<FailureAuditState>(stable(
            "mfm.fixture/failure-audit-state-implementation",
        ))
        .expect("failure audit registration");
    assembly
        .register_policy_recipe(recipe)
        .expect("policy recipe registration");
    assembly
        .register_entry_point(operation_id.clone(), authored.clone(), policy_profile)
        .expect("entry-point registration");
    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified registry");
    let certified = registry
        .certifier(&operation_id)
        .expect("qualified certifier")
        .certify(authored)
        .expect("certified program");

    let ExpandedDeclaration::Fragment(wrapper) = &certified.expanded().root.declarations[0] else {
        panic!("policy must produce one outer fragment");
    };
    let ExpandedDeclaration::Fragment(protected) = &wrapper.body.declarations[0] else {
        panic!("identity recipe must contain the protected fragment");
    };
    let CertifiedFailureBoundary::Typed { plan, .. } = &protected.failure_boundary else {
        panic!("protected boundary must remain typed");
    };
    let FailurePlan::Propagate {
        source_slot,
        before_boundary,
        mapping_chain,
        ..
    } = plan.as_ref()
    else {
        panic!("protected failure must propagate affinely");
    };
    assert_eq!(before_boundary.declarations.len(), 1);
    let ExpandedDeclaration::State(audit) = &before_boundary.declarations[0] else {
        panic!("failure post must contain its ordinary audit state directly");
    };
    assert!(mapping_chain.is_empty());
    assert_eq!(
        before_boundary.tail,
        mfm_spec::structured::BlockTail::Normal(source_slot.clone())
    );
    assert_eq!(
        audit.contract.state_contract_ref,
        state_contract::<FailureAuditState>()
            .expect("audit contract")
            .state_contract_ref
    );
}

#[test]
fn explicit_failure_post_mappers_form_one_exact_affine_chain() {
    for link_count in [1usize, 2] {
        let operation_id = stable(&format!("mfm.fixture/failure-map-chain-{link_count}"));
        let authored = fallible_program(operation_id.clone());
        let protected_contract = state_contract::<FallibleState>().expect("fallible contract");
        let recipe = if link_count == 1 {
            typed_policy_recipe_with_one_failure_mapper()
        } else {
            typed_policy_recipe_with_two_failure_mappers()
        };
        let policy = ExpansionPolicyContract::new(vec![PolicyExpansionBinding {
            boundary_contract_ref: protected_contract.state_contract_ref,
            recipe_ref: recipe.content_ref().expect("recipe ref"),
        }])
        .expect("policy contract");
        let mut policy_profile = profile();
        policy_profile.policies.push(policy);

        let mut assembly = ProgramRegistryBuilder::new();
        register_fixture_runtime_components(&mut assembly, false);
        for register in [
            ProgramRegistryBuilder::register_value::<Request>,
            ProgramRegistryBuilder::register_value::<Response>,
            ProgramRegistryBuilder::register_value::<StateFailure>,
            ProgramRegistryBuilder::register_value::<NormalizedFailure>,
            ProgramRegistryBuilder::register_value::<RootFailure>,
            ProgramRegistryBuilder::register_closed_sum::<DefaultRoute>,
        ] {
            register(&mut assembly).expect("fixture value contract");
        }
        assembly
            .register_fixture_state::<FallibleState>(stable(
                "mfm.fixture/fallible-state-implementation",
            ))
            .expect("fallible state registration");
        assembly
            .register_fixture_state::<RootFailureMapperState>(stable(
                "mfm.fixture/root-failure-mapper-implementation",
            ))
            .expect("root mapper registration");
        if link_count == 1 {
            assembly
                .register_fixture_state::<RedactStateFailure>(stable(
                    "mfm.fixture/redact-state-failure-implementation",
                ))
                .expect("redact mapper registration");
        } else {
            assembly
                .register_fixture_state::<NormalizeStateFailure>(stable(
                    "mfm.fixture/normalize-state-failure-implementation",
                ))
                .expect("normalize mapper registration");
            assembly
                .register_fixture_state::<RebuildStateFailure>(stable(
                    "mfm.fixture/rebuild-state-failure-implementation",
                ))
                .expect("rebuild mapper registration");
        }
        assembly
            .register_policy_recipe(recipe)
            .expect("policy recipe registration");
        assembly
            .register_entry_point(operation_id.clone(), authored.clone(), policy_profile)
            .expect("entry-point registration");
        let registry = assembly
            .build(std::slice::from_ref(&operation_id))
            .expect("qualified registry");
        let certified = registry
            .certifier(&operation_id)
            .expect("qualified certifier")
            .certify(authored)
            .expect("certified program");

        let ExpandedDeclaration::Fragment(wrapper) = &certified.expanded().root.declarations[0]
        else {
            panic!("policy must produce one outer fragment");
        };
        let ExpandedDeclaration::Fragment(protected) = &wrapper.body.declarations[0] else {
            panic!("policy recipe must retain the protected fragment");
        };
        let CertifiedFailureBoundary::Typed { plan, .. } = &protected.failure_boundary else {
            panic!("protected boundary must remain typed");
        };
        let FailurePlan::Propagate {
            plan_id,
            source_slot,
            before_boundary,
            mapping_chain,
            boundary_id,
            boundary_slot,
            ..
        } = plan.as_ref()
        else {
            panic!("protected failure must propagate affinely");
        };
        assert!(before_boundary.declarations.is_empty());
        assert_eq!(mapping_chain.len(), link_count);
        assert_eq!(mapping_chain[0].input_slot, *source_slot);
        assert!(mapping_chain.iter().all(|link| &link.plan_id == plan_id));
        assert!(mapping_chain
            .windows(2)
            .all(|links| links[0].output_slot == links[1].input_slot));
        let mapped_target = &mapping_chain[link_count - 1].output_slot;
        assert_eq!(
            before_boundary.tail,
            mfm_spec::structured::BlockTail::Normal(mapped_target.clone())
        );
        assert_eq!(mapped_target.contract_ref, source_slot.contract_ref);
        let mfm_spec::structured::LexicalProducer::FragmentBoundary {
            boundary_id: producer_boundary_id,
            role: mfm_spec::structured::ResultRole::TypedFailure,
            source,
        } = &boundary_slot.producer
        else {
            panic!("mapped target must rebind one typed fragment boundary");
        };
        assert_eq!(producer_boundary_id, boundary_id);
        assert!(fixture_propagation_source_contains(source, mapped_target));

        let mut oracle = ExecutionOracle::new(true, true);
        let expected_code = if link_count == 1 { 7 } else { 117 };
        assert_eq!(
            oracle
                .execute(certified.expanded(), Request { value: 1 })
                .expect("mapped failure execution"),
            OracleOutcome::Failure(RootFailure {
                code: expected_code,
            })
        );
        assert_eq!(
            oracle.calls.last().map(String::as_str),
            Some("mfm.fixture/root-failure-mapper")
        );
    }
}

#[test]
fn policy_profile_order_is_outer_to_inner_on_entry_and_reversed_on_exit() {
    let operation_id = stable("mfm.fixture/policy-order");
    let authored = copy_program(operation_id.clone());
    let protected = state_contract::<CopyState>().expect("copy contract");
    let outer_recipe = outer_policy_recipe();
    let inner_recipe = inner_policy_recipe();
    let outer = ExpansionPolicyContract::new(vec![PolicyExpansionBinding {
        boundary_contract_ref: protected.state_contract_ref.clone(),
        recipe_ref: outer_recipe.content_ref().expect("outer recipe ref"),
    }])
    .expect("outer policy");
    let inner = ExpansionPolicyContract::new(vec![PolicyExpansionBinding {
        boundary_contract_ref: protected.state_contract_ref,
        recipe_ref: inner_recipe.content_ref().expect("inner recipe ref"),
    }])
    .expect("inner policy");
    let mut policy_profile = profile();
    policy_profile.policies = vec![outer.clone(), inner.clone()];

    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<Request>()
        .expect("request contract");
    assembly
        .register_value::<Response>()
        .expect("response contract");
    assembly
        .register_fixture_state::<CopyState>(stable("mfm.fixture/copy-state-implementation"))
        .expect("copy state registration");
    assembly
        .register_fixture_state::<OuterPreState>(stable("mfm.fixture/outer-pre-implementation"))
        .expect("outer pre registration");
    assembly
        .register_fixture_state::<OuterPostState>(stable("mfm.fixture/outer-post-implementation"))
        .expect("outer post registration");
    assembly
        .register_fixture_state::<InnerPreState>(stable("mfm.fixture/inner-pre-implementation"))
        .expect("inner pre registration");
    assembly
        .register_fixture_state::<InnerPostState>(stable("mfm.fixture/inner-post-implementation"))
        .expect("inner post registration");
    assembly
        .register_policy_recipe(inner_recipe)
        .expect("inner recipe registration");
    assembly
        .register_policy_recipe(outer_recipe)
        .expect("outer recipe registration");
    assembly
        .register_entry_point(operation_id.clone(), authored.clone(), policy_profile)
        .expect("entry-point registration");
    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified registry");
    let certified = registry
        .certifier(&operation_id)
        .expect("qualified certifier")
        .certify(authored)
        .expect("certified program");

    let mut labels = Vec::new();
    collect_state_labels(&certified.expanded().root, &mut labels);
    assert_eq!(
        labels,
        ["outer-pre", "inner-pre", "copy", "inner-post", "outer-post"]
    );
    assert_eq!(
        certified
            .policy_coverage_proof()
            .entries
            .iter()
            .map(|entry| &entry.policy_ref)
            .collect::<Vec<_>>(),
        vec![&outer.policy_ref, &inner.policy_ref]
    );
    assert_eq!(
        certified
            .expansion_proof()
            .substitution_trace
            .iter()
            .map(|entry| &entry.expansion_ref)
            .collect::<Vec<_>>(),
        vec![&outer.policy_ref, &inner.policy_ref]
    );
}

#[test]
fn policy_guard_can_skip_but_cannot_duplicate_the_protected_boundary() {
    let operation_id = stable("mfm.fixture/guarded-policy");
    let certified = certify_guarded_fixture(&operation_id);

    let ExpandedDeclaration::Fragment(wrapper) = &certified.expanded().root.declarations[0] else {
        panic!("policy must produce one outer fragment");
    };
    assert!(matches!(
        wrapper.body.declarations.as_slice(),
        [ExpandedDeclaration::State(_), ExpandedDeclaration::Match(_)]
    ));
    let ExpandedDeclaration::Match(guard) = &wrapper.body.declarations[1] else {
        unreachable!("shape asserted above");
    };
    assert_eq!(
        guard
            .arms
            .iter()
            .map(|arm| arm.canonical_tag.as_str())
            .collect::<Vec<_>>(),
        ["allow", "deny"]
    );
    assert!(matches!(
        guard.arms[0].body.declarations.as_slice(),
        [ExpandedDeclaration::Fragment(_)]
    ));
    assert!(guard.arms[1].body.declarations.is_empty());
}

#[test]
fn execution_oracle_proves_guard_and_failure_post_callback_semantics() {
    let operation_id = stable("mfm.fixture/guarded-policy-oracle");
    let certified = certify_guarded_fixture(&operation_id);

    let mut denied = ExecutionOracle::new(false, false);
    assert_eq!(
        denied
            .execute(certified.expanded(), Request { value: 9 })
            .expect("denied execution"),
        OracleOutcome::Failure(RootFailure { code: 403 })
    );
    assert_eq!(
        denied.calls,
        ["mfm.fixture/guard-state", "mfm.fixture/root-failure-mapper"]
    );

    let mut allowed = ExecutionOracle::new(true, false);
    assert_eq!(
        allowed
            .execute(certified.expanded(), Request { value: 9 })
            .expect("allowed execution"),
        OracleOutcome::Success(Response { value: 10 })
    );
    assert_eq!(
        allowed.calls,
        ["mfm.fixture/guard-state", "mfm.fixture/fallible-state"]
    );

    let mut failed = ExecutionOracle::new(true, true);
    assert_eq!(
        failed
            .execute(certified.expanded(), Request { value: 9 })
            .expect("protected failure execution"),
        OracleOutcome::Failure(RootFailure { code: 7 })
    );
    assert_eq!(
        failed.calls,
        [
            "mfm.fixture/guard-state",
            "mfm.fixture/fallible-state",
            "mfm.fixture/failure-audit-state",
            "mfm.fixture/root-failure-mapper",
        ]
    );
}

#[test]
fn complete_structured_pipeline_has_stable_golden_bytes_and_execution() {
    let operation_id = stable("mfm.fixture/complete-pipeline-golden");
    let certified = certify_full_pipeline_fixture(&operation_id, false);
    let reversed = certify_full_pipeline_fixture(&operation_id, true);
    assert_eq!(certified.document(), reversed.document());

    let stages = certified
        .expansion_proof()
        .substitution_trace
        .iter()
        .map(|entry| entry.stage)
        .collect::<Vec<_>>();
    assert!(stages.windows(2).all(|pair| pair[0] <= pair[1]));
    assert_eq!(
        stages
            .iter()
            .filter(|stage| **stage == ExpansionStage::ChildSubstitution)
            .count(),
        1
    );
    assert_eq!(
        stages
            .iter()
            .filter(|stage| **stage == ExpansionStage::CapabilityLowering)
            .count(),
        1
    );
    assert_eq!(
        stages
            .iter()
            .filter(|stage| **stage == ExpansionStage::PolicyWrapping)
            .count(),
        2
    );
    assert_eq!(certified.policy_coverage_proof().entries.len(), 2);

    let expanded = certified
        .expanded()
        .canonical_json()
        .expect("expanded canonical bytes");
    let proceed_ref =
        mfm_spec::structured::policy_proceed_program_ref().expect("reserved proceed reference");
    assert!(!expanded
        .as_str()
        .contains(proceed_ref.content_digest().as_str()));

    let mut oracle = ExecutionOracle::new(true, false);
    assert_eq!(
        oracle
            .execute(certified.expanded(), Request { value: 5 })
            .expect("full pipeline execution"),
        OracleOutcome::Success(Response { value: 2 })
    );
    assert_eq!(
        oracle.calls,
        [
            "mfm.fixture/outer-pre",
            "mfm.fixture/guard-state",
            "mfm.fixture/fallible-state",
            "mfm.fixture/outer-post",
            "mfm.fixture/lane-a",
            "mfm.fixture/lane-b",
            "mfm.fixture/lane-b",
            "mfm.fixture/pipeline-aggregate",
        ]
    );

    let authored = certified
        .authored()
        .canonical_json()
        .expect("authored canonical bytes");
    let profile = certified
        .expansion_profile()
        .canonical_json()
        .expect("profile canonical bytes");
    let proof = canonical_fixture_bytes(certified.expansion_proof());
    let coverage = canonical_fixture_bytes(certified.policy_coverage_proof());
    let components = &certified.document().root.components;
    let component_manifest_object = certified
        .document()
        .component_closure
        .iter()
        .find(|object| {
            object.content_ref
                == components.state_capability_adapter_signer_resource_manifest_closure_ref
        })
        .expect("semantic manifest closure object");
    let component_manifest: StateCapabilityAdapterSignerResourceManifest =
        serde_json::from_value(component_manifest_object.value.as_json().clone())
            .expect("semantic manifest schema");
    let fallible_state_ref = state_contract::<FallibleState>()
        .expect("fallible state contract")
        .state_contract_ref;
    let fallible_index = component_manifest
        .entries
        .iter()
        .position(|entry| entry.semantic_contract_ref == fallible_state_ref)
        .expect("fallible state manifest entry");
    assert_eq!(
        component_manifest.entries[fallible_index..fallible_index + 5]
            .iter()
            .map(|entry| entry.component_kind)
            .collect::<Vec<_>>(),
        [
            StructuredComponentKind::State,
            StructuredComponentKind::Capability,
            StructuredComponentKind::Adapter,
            StructuredComponentKind::Signer,
            StructuredComponentKind::Resource,
        ]
    );
    let abstract_state_ref = state_contract::<AbstractFallibleCapabilityState>()
        .expect("abstract state contract")
        .state_contract_ref;
    assert!(component_manifest
        .entries
        .iter()
        .all(|entry| entry.semantic_contract_ref != abstract_state_ref));
    let implementation_manifest_object = certified
        .document()
        .component_closure
        .iter()
        .find(|object| {
            object.content_ref == components.secret_free_implementation_manifest_closure_ref
        })
        .expect("implementation manifest closure object");
    let implementation_manifest: SecretFreeImplementationManifest =
        serde_json::from_value(implementation_manifest_object.value.as_json().clone())
            .expect("implementation manifest schema");
    assert_eq!(
        component_manifest
            .entries
            .iter()
            .map(|entry| (entry.component_kind, &entry.semantic_contract_ref))
            .collect::<Vec<_>>(),
        implementation_manifest
            .entries
            .iter()
            .map(|entry| (entry.component_kind, &entry.semantic_contract_ref))
            .collect::<Vec<_>>()
    );
    let component_manifest_bytes = component_manifest_object
        .value
        .canonical_json()
        .expect("semantic manifest canonical bytes");
    let implementation_manifest_bytes = implementation_manifest_object
        .value
        .canonical_json()
        .expect("implementation manifest canonical bytes");
    let root = certified
        .document()
        .root
        .canonical_json()
        .expect("certification root canonical bytes");
    let actual = format!(
        concat!(
            "authored_sha256={}\nexpanded_sha256={}\nprofile_sha256={}\n",
            "proof_sha256={}\ncoverage_sha256={}\ncomponent_manifest_sha256={}\n",
            "implementation_manifest_sha256={}\n",
            "root_sha256={}\ncertified_ref={:?}\nclosure_digest={}"
        ),
        sha256_digest_bytes(authored.as_bytes()),
        sha256_digest_bytes(expanded.as_bytes()),
        sha256_digest_bytes(profile.as_bytes()),
        sha256_digest_bytes(proof.as_bytes()),
        sha256_digest_bytes(coverage.as_bytes()),
        sha256_digest_bytes(component_manifest_bytes.as_bytes()),
        sha256_digest_bytes(implementation_manifest_bytes.as_bytes()),
        sha256_digest_bytes(root.as_bytes()),
        certified.reference().expect("certified reference"),
        certified.document().root.canonical_component_closure_digest,
    );
    assert_eq!(
        actual,
        concat!(
            "authored_sha256=c88cbfadb438e8cdb087c4ca1a05fa0873df3d25e9bfc3918961abd586ea9ec1\n",
            "expanded_sha256=673424ee901ab26f9b28c7edbd7cc16df143deff218795989ec34f4eee141a4c\n",
            "profile_sha256=ece4250d787dcc91c2a646ec5e78490d487ac1e3cdc9fbb4eb14f9331e9cb60b\n",
            "proof_sha256=0f77d5ad24a3b02694836b2bf1a268f19d4118a914fb3500254b5729a7b6adb9\n",
            "coverage_sha256=34c64ede64588319bd6a4ff105757cffab545345313642eb68a0a3ab860a1d85\n",
            "component_manifest_sha256=532e066ea4b6baca3b758c1fc1b072e39f6d1c26c910e3cf5955571b14f40ff2\n",
            "implementation_manifest_sha256=a4973d3abaf3088110571cfc3b8da047b06fae9f5d6a4023d45d3e4c9cbea9fb\n",
            "root_sha256=4b95a5f535d0b5f032c88ea19d83a7915567eb28150a7869e3749239b57b0275\n",
            "certified_ref=ContentRef { schema_id: Identity(\"schema:mfm.certified-program:1:sha256-jcs-v1:3fb6529deb28e23b49f2f978846d0051070f189478e301210233c019b0646b19\"), content_digest: Identity(\"content:sha256-v1:6f233dcf6d029c455c3acef4901aa7557811d48db7deda6c12f41e81db6a8d4d\") }\n",
            "closure_digest=content:sha256-v1:75911bf40c9c8a829e685f5661822297dac9f41bd22b72956f390f665ef9d15a",
        )
    );
}

#[test]
fn certified_fan_out_join_is_declaration_ordered_under_completion_permutations() {
    let operation_id = stable("mfm.fixture/fan-out-oracle");
    let authored = fan_out_program(operation_id.clone());
    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<Request>()
        .expect("request contract");
    assembly
        .register_value::<Response>()
        .expect("response contract");
    assembly
        .register_fixture_state::<LaneAState>(stable("mfm.fixture/lane-a-implementation"))
        .expect("lane A registration");
    assembly
        .register_fixture_state::<LaneBState>(stable("mfm.fixture/lane-b-implementation"))
        .expect("lane B registration");
    assembly
        .register_entry_point(operation_id.clone(), authored.clone(), profile())
        .expect("entry-point registration");
    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified registry");
    let certified = registry
        .certifier(&operation_id)
        .expect("qualified certifier")
        .certify(authored)
        .expect("certified fan-out");

    let mut forward = ExecutionOracle::new(false, false);
    let forward_result = forward
        .execute_raw(certified.expanded(), Request { value: 5 })
        .expect("forward fan-out execution");
    let mut reverse = ExecutionOracle::new(false, false);
    reverse.reverse_fan_out = true;
    let reverse_result = reverse
        .execute_raw(certified.expanded(), Request { value: 5 })
        .expect("reverse fan-out execution");

    assert_eq!(forward_result, reverse_result);
    assert_eq!(
        forward_result,
        serde_json::json!({
            "head": {"Success": {"value": 15}},
            "tail": [
                {"Success": {"value": 25}},
            ]
        })
    );
    assert_eq!(forward.calls, ["mfm.fixture/lane-a", "mfm.fixture/lane-b"]);
    assert_eq!(reverse.calls, ["mfm.fixture/lane-b", "mfm.fixture/lane-a"]);
}

#[test]
fn qualification_rejects_duplicate_same_scope_labels() {
    // Duplicate declaration labels in one block.
    let operation_id = stable("mfm.fixture/duplicate-declaration-labels");
    let mut duplicate_decl = copy_program(operation_id.clone());
    let AuthoredDeclaration::State(first) = &duplicate_decl.root.declarations[0] else {
        panic!("copy fixture");
    };
    let mut second = (**first).clone();
    second.label = first.label.clone();
    second.semantic_path = first.semantic_path.clone();
    second.semantic_call_id = first.semantic_call_id.clone();
    duplicate_decl
        .root
        .declarations
        .push(AuthoredDeclaration::State(Box::new(second)));
    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<Request>()
        .expect("request contract");
    assembly
        .register_value::<Response>()
        .expect("response contract");
    assembly
        .register_fixture_state::<CopyState>(stable("mfm.fixture/copy-state-implementation"))
        .expect("copy state");
    assembly
        .register_entry_point(operation_id.clone(), duplicate_decl, profile())
        .expect("hostile duplicate declaration remains inert until build");
    let error = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect_err("duplicate declaration labels must fail qualification");
    assert!(
        error
            .to_string()
            .contains("duplicate authored declaration label"),
        "got {error}"
    );

    // Duplicate Match arm labels with distinct tags.
    let match_id = stable("mfm.fixture/duplicate-match-labels");
    let mut match_program = guard_match_program(match_id.clone());
    let binding = match_program
        .root
        .declarations
        .iter_mut()
        .find_map(|declaration| match declaration {
            AuthoredDeclaration::Match(binding) => Some(binding),
            _ => None,
        })
        .expect("guard Match fixture");
    binding.arms[1].label = binding.arms[0].label.clone();
    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<Request>()
        .expect("request contract");
    assembly
        .register_value::<Response>()
        .expect("response contract");
    assembly
        .register_value::<StateFailure>()
        .expect("failure contract");
    assembly
        .register_closed_sum::<GuardDecision>()
        .expect("selector");
    assembly
        .register_fixture_state::<CopyState>(stable("mfm.fixture/copy-state-implementation"))
        .expect("copy state");
    assembly
        .register_fixture_state::<GuardState>(stable("mfm.fixture/guard-state-implementation"))
        .expect("guard state");
    assembly
        .register_entry_point(match_id.clone(), match_program, profile())
        .expect("hostile match remains inert");
    let error = assembly
        .build(std::slice::from_ref(&match_id))
        .expect_err("duplicate Match arm labels must fail qualification");
    assert!(
        error.to_string().contains("duplicate stable arm label"),
        "got {error}"
    );
}

#[test]
fn qualification_rejects_a_caller_substituted_fan_out_join_contract() {
    let operation_id = stable("mfm.fixture/hostile-fan-out-join");
    let mut authored = fan_out_program(operation_id.clone());
    let foreign = state_contract::<CopyState>()
        .expect("foreign state contract")
        .state_contract_ref;
    let AuthoredDeclaration::FanOut(fan_out) = &mut authored.root.declarations[0] else {
        unreachable!("fan-out fixture shape")
    };
    fan_out.output_slot.contract_ref = foreign.clone();
    let mfm_spec::structured::BlockTail::Normal(tail) = &mut authored.root.tail else {
        unreachable!("fan-out root normal tail")
    };
    tail.contract_ref = foreign;

    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<Request>()
        .expect("request contract");
    assembly
        .register_value::<Response>()
        .expect("response contract");
    assembly
        .register_fixture_state::<LaneAState>(stable("mfm.fixture/lane-a-implementation"))
        .expect("lane A state");
    assembly
        .register_fixture_state::<LaneBState>(stable("mfm.fixture/lane-b-implementation"))
        .expect("lane B state");
    assembly
        .register_entry_point(operation_id.clone(), authored, profile())
        .expect("hostile entry remains inert");
    let error = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect_err("caller-selected fan-out join contract must fail");
    assert!(error.to_string().contains("authored fan-out join"));
}

#[test]
fn typed_fan_out_wraps_success_and_failure_and_joins_in_declaration_order() {
    let operation_id = stable("mfm.fixture/typed-fan-out-oracle");
    let authored = typed_fan_out_program(operation_id.clone());
    let registry = qualify_typed_fan_out_fixture(&operation_id);
    let certified = registry
        .certifier(&operation_id)
        .expect("typed fan-out certifier")
        .certify(authored)
        .expect("typed fan-out certification");
    let [ExpandedDeclaration::FanOut(group)] = certified.expanded().root.declarations.as_slice()
    else {
        panic!("typed fan-out declaration");
    };
    assert_eq!(group.lanes.len(), 2);
    let mfm_spec::structured::LexicalProducer::LaneOutcome {
        success_slot,
        failure_slot,
        ..
    } = &group.lanes[0].outcome_slot.producer
    else {
        panic!("fallible lane outcome");
    };
    assert!(success_slot.is_some());
    assert!(failure_slot.is_some());
    let mfm_spec::structured::LexicalProducer::LaneOutcome {
        success_slot,
        failure_slot,
        ..
    } = &group.lanes[1].outcome_slot.producer
    else {
        panic!("successful lane outcome");
    };
    assert!(success_slot.is_some());
    assert!(failure_slot.is_none());

    let mut forward = ExecutionOracle::new(false, true);
    let forward_result = forward
        .execute_raw(certified.expanded(), Request { value: 5 })
        .expect("typed fan-out execution");
    let mut reverse = ExecutionOracle::new(false, true);
    reverse.reverse_fan_out = true;
    let reverse_result = reverse
        .execute_raw(certified.expanded(), Request { value: 5 })
        .expect("reverse typed fan-out execution");
    assert_eq!(forward_result, reverse_result);
    assert_eq!(
        forward_result,
        serde_json::json!({
            "head": {"Failure": {"code": 7}},
            "tail": [
                {"Success": {"value": 15}},
            ]
        })
    );
    assert_eq!(
        forward.calls,
        [
            "mfm.fixture/fallible-state",
            "mfm.fixture/state-failure-mapper",
            "mfm.fixture/lane-a",
        ]
    );
    assert_eq!(
        reverse.calls,
        [
            "mfm.fixture/lane-a",
            "mfm.fixture/fallible-state",
            "mfm.fixture/state-failure-mapper",
        ]
    );
}

#[test]
fn certified_depth_two_fan_out_executes_without_structural_transitions() {
    let operation_id = stable("mfm.fixture/depth-two-fan-out-oracle");
    let authored = depth_two_fan_out_program(operation_id.clone());
    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<Request>()
        .expect("request contract");
    assembly
        .register_value::<Response>()
        .expect("response contract");
    assembly
        .register_fixture_state::<LaneAState>(stable("mfm.fixture/lane-a-implementation"))
        .expect("lane A registration");
    assembly
        .register_fixture_state::<LaneBState>(stable("mfm.fixture/lane-b-implementation"))
        .expect("lane B registration");
    assembly
        .register_entry_point(operation_id.clone(), authored.clone(), profile())
        .expect("entry-point registration");
    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified registry");
    let certified = registry
        .certifier(&operation_id)
        .expect("qualified certifier")
        .certify(authored)
        .expect("certified depth-two fan-out");

    let mut oracle = ExecutionOracle::new(false, false);
    oracle.reverse_fan_out = true;
    let outcome = oracle
        .execute_raw(certified.expanded(), Request { value: 1 })
        .expect("depth-two execution");
    assert_eq!(
        outcome,
        serde_json::json!({
            "head": {"Success": {
                "head": {"Success": {"value": 11}},
                "tail": [
                    {"Success": {"value": 21}},
                ]
            }},
            "tail": [
                {"Success": {
                    "head": {"Success": {"value": 21}},
                    "tail": []
                }},
            ]
        })
    );
    assert_eq!(oracle.calls.len(), 3);
}

#[test]
fn custom_recovery_is_exhaustive_and_totally_recovers_in_a_never_scope() {
    let operation_id = stable("mfm.fixture/custom-recovery");
    let authored = custom_recovery_program(operation_id.clone());
    let mut assembly = ProgramRegistryBuilder::new();
    register_fixture_runtime_components(&mut assembly, false);
    for register in [
        ProgramRegistryBuilder::register_value::<Request>,
        ProgramRegistryBuilder::register_value::<Response>,
        ProgramRegistryBuilder::register_value::<StateFailure>,
        ProgramRegistryBuilder::register_closed_sum::<RecoveryRoute>,
    ] {
        register(&mut assembly).expect("fixture value contract");
    }
    assembly
        .register_fixture_state::<FallibleState>(stable(
            "mfm.fixture/fallible-state-implementation",
        ))
        .expect("fallible state registration");
    assembly
        .register_fixture_state::<RecoveryHandlerState>(stable(
            "mfm.fixture/recovery-handler-implementation",
        ))
        .expect("recovery handler registration");
    assembly
        .register_fixture_state::<RecoveryReadState>(stable(
            "mfm.fixture/recovery-read-implementation",
        ))
        .expect("recovery read registration");
    assembly
        .register_entry_point(operation_id.clone(), authored.clone(), profile())
        .expect("entry-point registration");
    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified registry");
    let certified = registry
        .certifier(&operation_id)
        .expect("qualified certifier")
        .certify(authored)
        .expect("certified recovery program");

    let mut oracle = ExecutionOracle::new(true, true);
    assert_eq!(
        oracle
            .execute(certified.expanded(), Request { value: 3 })
            .expect("recovered execution"),
        OracleOutcome::Success(Response { value: 77 })
    );
    assert_eq!(
        oracle.calls,
        [
            "mfm.fixture/fallible-state",
            "mfm.fixture/recovery-handler",
            "mfm.fixture/recovery-read",
        ]
    );
}

#[test]
fn custom_recovery_can_author_reads_under_every_sealed_policy() {
    let authored = recovery_read_policy_program(stable("mfm.fixture/recovery-read-policies"));
    let mut counts = BTreeMap::<String, usize>::new();
    count_authored_state_ids(&authored.root, &mut counts);
    assert_eq!(counts.get("mfm.fixture/recovery-read"), Some(&3));
}

#[test]
fn a_failure_post_state_failure_causally_supersedes_the_protected_failure() {
    let operation_id = stable("mfm.fixture/failing-failure-post-policy");
    let authored = fallible_program(operation_id.clone());
    let protected = state_contract::<FallibleState>().expect("fallible contract");
    let recipe = typed_policy_recipe_with_fallible_failure_post();
    let policy = ExpansionPolicyContract::new(vec![PolicyExpansionBinding {
        boundary_contract_ref: protected.state_contract_ref,
        recipe_ref: recipe.content_ref().expect("recipe ref"),
    }])
    .expect("policy contract");
    let mut policy_profile = profile();
    policy_profile.policies.push(policy);
    let mut assembly = ProgramRegistryBuilder::new();
    register_fixture_runtime_components(&mut assembly, false);
    for register in [
        ProgramRegistryBuilder::register_value::<Request>,
        ProgramRegistryBuilder::register_value::<Response>,
        ProgramRegistryBuilder::register_value::<StateFailure>,
        ProgramRegistryBuilder::register_value::<RootFailure>,
        ProgramRegistryBuilder::register_value::<PostFailure>,
        ProgramRegistryBuilder::register_closed_sum::<DefaultRoute>,
        ProgramRegistryBuilder::register_closed_sum::<PostFailureRoute>,
    ] {
        register(&mut assembly).expect("fixture value contract");
    }
    assembly
        .register_fixture_state::<FallibleState>(stable(
            "mfm.fixture/fallible-state-implementation",
        ))
        .expect("fallible state registration");
    assembly
        .register_fixture_state::<RootFailureMapperState>(stable(
            "mfm.fixture/root-failure-mapper-implementation",
        ))
        .expect("root mapper registration");
    assembly
        .register_fixture_state::<FailingFailurePostState>(stable(
            "mfm.fixture/failing-failure-post-implementation",
        ))
        .expect("fallible post registration");
    assembly
        .register_fixture_state::<PostFailureMapperState>(stable(
            "mfm.fixture/post-failure-mapper-implementation",
        ))
        .expect("post mapper registration");
    assembly
        .register_policy_recipe(recipe)
        .expect("policy recipe registration");
    assembly
        .register_entry_point(operation_id.clone(), authored.clone(), policy_profile)
        .expect("entry-point registration");
    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified registry");
    let certified = registry
        .certifier(&operation_id)
        .expect("qualified certifier")
        .certify(authored)
        .expect("certified program");

    let mut oracle = ExecutionOracle::new(true, true);
    oracle.failure_post_fails = true;
    assert_eq!(
        oracle
            .execute(certified.expanded(), Request { value: 4 })
            .expect("superseding failure execution"),
        OracleOutcome::Failure(RootFailure { code: 91 })
    );
    assert_eq!(
        oracle.calls,
        [
            "mfm.fixture/fallible-state",
            "mfm.fixture/failing-failure-post",
            "mfm.fixture/post-failure-mapper",
            "mfm.fixture/root-failure-mapper",
        ]
    );
}

fn copy_program(operation_id: StableId) -> mfm_spec::structured::AuthoredStructuredProgram {
    copy_program_with_label(operation_id, "copy")
}

fn copy_program_with_label(
    operation_id: StableId,
    label: &str,
) -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder =
        OperationBuilder::<Response, Never>::new(operation_id, stable("root")).expect("builder");
    let input = builder.input::<Request>(stable("request")).expect("input");
    let output = builder
        .root()
        .state::<CopyState>(stable(label), &input)
        .expect("copy declaration")
        .infallible()
        .expect("infallible copy");
    let completion = builder.succeed(&output).expect("success");
    builder.finish(completion).expect("authored program")
}

fn lane_a_program(operation_id: StableId) -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder =
        OperationBuilder::<Response, Never>::new(operation_id, stable("root")).expect("builder");
    let input = builder.input::<Request>(stable("request")).expect("input");
    let output = builder
        .root()
        .state::<LaneAState>(stable("lane-a"), &input)
        .expect("lane A declaration")
        .infallible()
        .expect("infallible lane A");
    let completion = builder.succeed(&output).expect("success");
    builder.finish(completion).expect("authored program")
}

fn guard_match_program(operation_id: StableId) -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder = OperationBuilder::<Response, StateFailure>::new(operation_id, stable("root"))
        .expect("builder");
    let input = builder.input::<Request>(stable("request")).expect("input");
    let decision = builder
        .root()
        .state::<GuardState>(stable("guard"), &input)
        .expect("guard state")
        .infallible()
        .expect("guard completion");
    let output = builder
        .root()
        .match_value(stable("decision"), &decision, |arms| {
            arms.arm("allow", stable("allow"), |block, payloads| {
                let request = payloads.value::<Request>(&[stable("request")])?;
                let response = block
                    .state::<CopyState>(stable("copy"), &request)?
                    .infallible()?;
                block.normal(&response)
            })?;
            arms.arm("deny", stable("deny"), |block, payloads| {
                let failure = payloads.value::<StateFailure>(&[stable("failure")])?;
                block.scope_failure::<Response>(&failure)
            })
        })
        .expect("exhaustive guard Match");
    let completion = builder.succeed(&output).expect("success");
    builder.finish(completion).expect("guard Match program")
}

fn guard_match_program_with_nonlocal_selectors(
    operation_id: StableId,
) -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder = OperationBuilder::<Response, StateFailure>::new(operation_id, stable("root"))
        .expect("builder");
    let input = builder.input::<Request>(stable("request")).expect("input");
    let decision = builder
        .root()
        .state::<GuardState>(stable("guard"), &input)
        .expect("guard state")
        .infallible()
        .expect("guard completion");
    let output = builder
        .root()
        .match_value(stable("decision"), &decision, |arms| {
            arms.arm("allow", stable("allow"), |block, payloads| {
                let request = payloads.value::<Request>(&[stable("request")])?;
                let _branch_selector = block
                    .state::<GuardState>(stable("branch-selector"), &request)?
                    .infallible()?;
                let response = block
                    .state::<CopyState>(stable("copy"), &request)?
                    .infallible()?;
                block.normal(&response)
            })?;
            arms.arm("deny", stable("deny"), |block, payloads| {
                let failure = payloads.value::<StateFailure>(&[stable("failure")])?;
                block.scope_failure::<Response>(&failure)
            })
        })
        .expect("exhaustive guard Match");
    let _future_selector = builder
        .root()
        .state::<GuardState>(stable("future-selector"), &input)
        .expect("future guard state")
        .infallible()
        .expect("future guard completion");
    let completion = builder.succeed(&output).expect("success");
    builder
        .finish(completion)
        .expect("guard Match program with non-local selectors")
}

fn fallible_program(operation_id: StableId) -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder = OperationBuilder::<Response, RootFailure>::new(operation_id, stable("root"))
        .expect("builder");
    builder
        .root()
        .failure_map::<StateFailure, RootFailureMapper>()
        .expect("failure mapper");
    let input = builder.input::<Request>(stable("request")).expect("input");
    let output = builder
        .root()
        .state::<FallibleState>(stable("read"), &input)
        .expect("fallible declaration")
        .or_default()
        .expect("default handler");
    let completion = builder.succeed(&output).expect("success");
    builder.finish(completion).expect("authored program")
}

fn custom_recovery_program(
    operation_id: StableId,
) -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder =
        OperationBuilder::<Response, Never>::new(operation_id, stable("root")).expect("builder");
    let input = builder.input::<Request>(stable("request")).expect("input");
    let output = builder
        .root()
        .state::<FallibleState>(stable("read"), &input)
        .expect("fallible state")
        .on_failure::<RecoveryHandler>()
        .expect("custom recovery");
    let completion = builder.succeed(&output).expect("success");
    builder.finish(completion).expect("recovery program")
}

fn recovery_read_policy_program(
    operation_id: StableId,
) -> mfm_spec::structured::AuthoredStructuredProgram {
    type InnerJoin = FanOutResults<Response, Never>;
    type OuterJoin = FanOutResults<InnerJoin, Never>;

    let mut builder =
        OperationBuilder::<OuterJoin, Never>::new(operation_id, stable("root")).expect("builder");
    let input = builder.input::<Request>(stable("request")).expect("input");
    let _sequential = builder
        .root()
        .state::<FallibleState>(stable("sequential-read"), &input)
        .expect("sequential fallible state")
        .on_failure::<RecoveryHandler>()
        .expect("sequential recovery read");

    let mut first = builder
        .root()
        .fan_out::<Response, Never>(stable("first-fan-out"))
        .expect("first fan-out");
    first
        .lane(stable("first-lane"), |lane| {
            let output = lane
                .state::<FallibleState>(stable("first-fallible-read"), &input)?
                .on_failure::<RecoveryHandler>()?;
            lane.normal(&output)
        })
        .expect("first fan-out recovery read");
    let _first_join = first.finish().expect("first fan-out join");

    let mut outer = builder
        .root()
        .fan_out::<InnerJoin, Never>(stable("outer-fan-out"))
        .expect("outer fan-out");
    outer
        .lane(stable("outer-lane"), |lane| {
            let mut inner = lane.fan_out::<Response, Never>(stable("inner-fan-out"))?;
            inner.lane(stable("inner-lane"), |lane| {
                let output = lane
                    .state::<FallibleState>(stable("depth-two-fallible-read"), &input)?
                    .on_failure::<RecoveryHandler>()?;
                lane.normal(&output)
            })?;
            let joined = inner.finish()?;
            lane.normal(&joined)
        })
        .expect("depth-two recovery read");
    let joined = outer.finish().expect("outer join");
    let completion = builder.succeed(&joined).expect("success");
    builder.finish(completion).expect("recovery read policies")
}

fn count_authored_state_ids(block: &AuthoredBlock, counts: &mut BTreeMap<String, usize>) {
    for declaration in &block.declarations {
        match declaration {
            AuthoredDeclaration::State(state) => {
                *counts
                    .entry(state.contract.semantic_state_id.as_str().to_owned())
                    .or_default() += 1;
                count_authored_failure_state_ids(&state.failure_directive, counts);
            }
            AuthoredDeclaration::Match(binding) => {
                for arm in &binding.arms {
                    count_authored_state_ids(&arm.body, counts);
                }
            }
            AuthoredDeclaration::FanOut(group) => {
                for lane in &group.lanes {
                    count_authored_state_ids(&lane.body, counts);
                }
            }
            AuthoredDeclaration::OperationCall(call) => {
                count_authored_failure_state_ids(&call.failure_directive, counts);
            }
        }
    }
}

fn count_authored_failure_state_ids(
    directive: &AuthoredFailureDirective,
    counts: &mut BTreeMap<String, usize>,
) {
    if let AuthoredFailureDirective::Custom { arms, .. } = directive {
        for arm in arms {
            count_authored_state_ids(&arm.body, counts);
        }
    }
}

fn capability_program(operation_id: StableId) -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder =
        OperationBuilder::<Response, Never>::new(operation_id, stable("root")).expect("builder");
    let input = builder.input::<Request>(stable("request")).expect("input");
    let output = builder
        .root()
        .state::<AbstractCapabilityState>(stable("abstract-read"), &input)
        .expect("abstract state")
        .infallible()
        .expect("abstract completion");
    let completion = builder.succeed(&output).expect("success");
    builder.finish(completion).expect("authored program")
}

fn recursive_capability_program(
    operation_id: StableId,
) -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder =
        OperationBuilder::<Response, Never>::new(operation_id, stable("root")).expect("builder");
    let input = builder.input::<Request>(stable("request")).expect("input");
    let output = builder
        .root()
        .state::<RecursiveCapabilityState>(stable("recursive-read"), &input)
        .expect("recursive abstract state")
        .infallible()
        .expect("recursive abstract completion");
    let completion = builder.succeed(&output).expect("success");
    builder.finish(completion).expect("recursive program")
}

fn child_capability_program(
    operation_id: StableId,
    calls: usize,
) -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder =
        OperationBuilder::<Response, Never>::new(operation_id, stable("root")).expect("builder");
    let input = builder.input::<Request>(stable("request")).expect("input");
    let mut output = None;
    for call in 0..calls {
        output = Some(
            builder
                .root()
                .state::<ChildCapabilityState>(stable(&format!("child-capability-{call}")), &input)
                .expect("child capability state")
                .infallible()
                .expect("child capability completion"),
        );
    }
    let completion = builder
        .succeed(&output.expect("at least one child capability call"))
        .expect("success");
    builder
        .finish(completion)
        .expect("child capability program")
}

fn nested_child_capability_program(
    operation_id: StableId,
) -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder =
        OperationBuilder::<Response, Never>::new(operation_id, stable("root")).expect("builder");
    let input = builder.input::<Request>(stable("request")).expect("input");
    let output = builder
        .root()
        .state::<NestedChildCapabilityState>(stable("nested-child-capability"), &input)
        .expect("nested child capability state")
        .infallible()
        .expect("nested child capability completion");
    let completion = builder.succeed(&output).expect("success");
    builder
        .finish(completion)
        .expect("nested child capability program")
}

fn recursive_child_capability_program(
    operation_id: StableId,
) -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder =
        OperationBuilder::<Response, Never>::new(operation_id, stable("root")).expect("builder");
    let input = builder.input::<Request>(stable("request")).expect("input");
    let output = builder
        .root()
        .state::<RecursiveChildCapabilityState>(stable("recursive-child-capability"), &input)
        .expect("recursive child capability state")
        .infallible()
        .expect("recursive child capability completion");
    let completion = builder.succeed(&output).expect("success");
    builder
        .finish(completion)
        .expect("recursive child capability program")
}

fn recovering_child_capability_program(
    operation_id: StableId,
) -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder =
        OperationBuilder::<Response, Never>::new(operation_id, stable("root")).expect("builder");
    let input = builder.input::<Request>(stable("request")).expect("input");
    let output = builder
        .root()
        .state::<RecoveringChildCapabilityState>(stable("recovering-child-capability"), &input)
        .expect("recovering child capability state")
        .infallible()
        .expect("recovering child capability completion");
    let completion = builder.succeed(&output).expect("success");
    builder
        .finish(completion)
        .expect("recovering child capability program")
}

fn copy_child_program() -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder = OperationBuilder::<Response, Never>::new(
        stable("mfm.fixture/copy-child"),
        stable("child-root"),
    )
    .expect("child builder");
    let input = builder
        .input::<Request>(stable("child-request"))
        .expect("child input");
    let output = builder
        .root()
        .state::<CopyState>(stable("copy"), &input)
        .expect("child state")
        .infallible()
        .expect("child completion");
    let completion = builder.succeed(&output).expect("child success");
    builder.finish(completion).expect("child program")
}

fn nested_copy_child_program() -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder = OperationBuilder::<Response, Never>::new(
        stable("mfm.fixture/nested-copy-child"),
        stable("nested-child-root"),
    )
    .expect("nested child builder");
    let input = builder
        .input::<Request>(stable("nested-child-request"))
        .expect("nested child input");
    let output = builder
        .root()
        .child::<CopyChild>(
            stable("copy-child"),
            vec![input.bind_child(stable("child-request"))],
        )
        .expect("nested child call")
        .infallible()
        .expect("nested child completion");
    let completion = builder.succeed(&output).expect("nested child success");
    builder.finish(completion).expect("nested child program")
}

fn recursive_support_child_program() -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder = OperationBuilder::<Response, Never>::new(
        stable("mfm.fixture/recursive-support-child"),
        stable("recursive-support-child-root"),
    )
    .expect("recursive support child builder");
    let input = builder
        .input::<Request>(stable("recursive-support-child-request"))
        .expect("recursive support child input");
    let output = builder
        .root()
        .state::<AbstractCapabilityState>(stable("nested-capability"), &input)
        .expect("nested capability state")
        .infallible()
        .expect("nested capability completion");
    let completion = builder
        .succeed(&output)
        .expect("recursive support child success");
    builder
        .finish(completion)
        .expect("recursive support child program")
}

fn recoverable_child_program() -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder = OperationBuilder::<Response, StateFailure>::new(
        stable("mfm.fixture/recoverable-child"),
        stable("recoverable-child-root"),
    )
    .expect("recoverable child builder");
    builder
        .root()
        .failure_map::<StateFailure, StateFailureMapper>()
        .expect("recoverable child failure mapper");
    let input = builder
        .input::<Request>(stable("recoverable-child-input"))
        .expect("recoverable child input");
    let output = builder
        .root()
        .state::<FallibleState>(stable("fallible-read"), &input)
        .expect("fallible child state")
        .or_default()
        .expect("fallible child propagation");
    let completion = builder.succeed(&output).expect("recoverable child success");
    builder
        .finish(completion)
        .expect("recoverable child program")
}

fn child_parent_program(
    operation_id: StableId,
    child_root_id: StableId,
) -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder =
        OperationBuilder::<Response, Never>::new(operation_id, stable("root")).expect("builder");
    let input = builder.input::<Request>(stable("request")).expect("input");
    let output = builder
        .root()
        .child::<CopyChild>(stable("child"), vec![input.bind_child(child_root_id)])
        .expect("child call")
        .infallible()
        .expect("child completion");
    let completion = builder.succeed(&output).expect("success");
    builder.finish(completion).expect("parent program")
}

fn typed_child_parent_program(
    operation_id: StableId,
) -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder = OperationBuilder::<Response, RootFailure>::new(operation_id, stable("root"))
        .expect("typed child parent builder");
    builder
        .root()
        .failure_map::<StateFailure, RootFailureMapper>()
        .expect("root failure mapper");
    let input = builder.input::<Request>(stable("request")).expect("input");
    let output = builder
        .root()
        .child::<PipelineChild>(
            stable("pipeline-child"),
            vec![input.bind_child(stable("pipeline-child-input"))],
        )
        .expect("typed child call")
        .or_default()
        .expect("typed child failure completion");
    let completion = builder.succeed(&output).expect("success");
    builder.finish(completion).expect("typed child parent")
}

fn fan_out_program(operation_id: StableId) -> mfm_spec::structured::AuthoredStructuredProgram {
    type Join = FanOutResults<Response, Never>;

    let mut builder =
        OperationBuilder::<Join, Never>::new(operation_id, stable("root")).expect("builder");
    let input = builder.input::<Request>(stable("request")).expect("input");
    let mut fan_out = builder
        .root()
        .fan_out::<Response, Never>(stable("reads"))
        .expect("fan-out");
    fan_out
        .lane(stable("lane-a"), |lane| {
            let output = lane
                .state::<LaneAState>(stable("read-a"), &input)?
                .infallible()?;
            lane.normal(&output)
        })
        .expect("lane A");
    fan_out
        .lane(stable("lane-b"), |lane| {
            let output = lane
                .state::<LaneBState>(stable("read-b"), &input)?
                .infallible()?;
            lane.normal(&output)
        })
        .expect("lane B");
    let joined = fan_out.finish().expect("fan-out join");
    let completion = builder.succeed(&joined).expect("success");
    builder.finish(completion).expect("fan-out program")
}

fn typed_fan_out_program(
    operation_id: StableId,
) -> mfm_spec::structured::AuthoredStructuredProgram {
    type Join = FanOutResults<Response, StateFailure>;

    let mut builder =
        OperationBuilder::<Join, Never>::new(operation_id, stable("root")).expect("builder");
    let input = builder.input::<Request>(stable("request")).expect("input");
    let mut fan_out = builder
        .root()
        .fan_out::<Response, StateFailure>(stable("typed-reads"))
        .expect("typed fan-out");
    fan_out
        .lane(stable("fallible"), |lane| {
            lane.failure_map::<StateFailure, StateFailureMapper>()?;
            let output = lane
                .state::<FallibleState>(stable("fallible-read"), &input)?
                .or_default()?;
            lane.normal(&output)
        })
        .expect("fallible lane");
    fan_out
        .lane(stable("successful"), |lane| {
            let output = lane
                .state::<LaneAState>(stable("successful-read"), &input)?
                .infallible()?;
            lane.normal(&output)
        })
        .expect("successful lane");
    let joined = fan_out.finish().expect("typed fan-out join");
    let completion = builder.succeed(&joined).expect("success");
    builder.finish(completion).expect("typed fan-out program")
}

fn depth_two_fan_out_program(
    operation_id: StableId,
) -> mfm_spec::structured::AuthoredStructuredProgram {
    type InnerJoin = FanOutResults<Response, Never>;
    type OuterJoin = FanOutResults<InnerJoin, Never>;

    let mut builder =
        OperationBuilder::<OuterJoin, Never>::new(operation_id, stable("root")).expect("builder");
    let input = builder.input::<Request>(stable("request")).expect("input");
    let mut outer = builder
        .root()
        .fan_out::<InnerJoin, Never>(stable("outer"))
        .expect("outer fan-out");
    outer
        .lane(stable("outer-a"), |lane| {
            let mut inner = lane.fan_out::<Response, Never>(stable("inner-a"))?;
            inner.lane(stable("a"), |lane| {
                let value = lane
                    .state::<LaneAState>(stable("read-a"), &input)?
                    .infallible()?;
                lane.normal(&value)
            })?;
            inner.lane(stable("b"), |lane| {
                let value = lane
                    .state::<LaneBState>(stable("read-b"), &input)?
                    .infallible()?;
                lane.normal(&value)
            })?;
            let joined = inner.finish()?;
            lane.normal(&joined)
        })
        .expect("outer lane A");
    outer
        .lane(stable("outer-b"), |lane| {
            let mut inner = lane.fan_out::<Response, Never>(stable("inner-b"))?;
            inner.lane(stable("b"), |lane| {
                let value = lane
                    .state::<LaneBState>(stable("read-b"), &input)?
                    .infallible()?;
                lane.normal(&value)
            })?;
            let joined = inner.finish()?;
            lane.normal(&joined)
        })
        .expect("outer lane B");
    let joined = outer.finish().expect("outer join");
    let completion = builder.succeed(&joined).expect("success");
    builder.finish(completion).expect("depth-two program")
}

fn capability_recipe() -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder = OperationBuilder::<Response, Never>::new(
        stable("mfm.fixture/capability-recipe"),
        stable("capability-root"),
    )
    .expect("recipe builder");
    let input = builder
        .input::<Request>(stable("capability-input"))
        .expect("recipe input");
    let output = builder
        .root()
        .state::<ConcreteCapabilityState>(stable("qualified-read"), &input)
        .expect("concrete state")
        .infallible()
        .expect("concrete completion");
    let completion = builder.succeed(&output).expect("recipe success");
    builder.finish(completion).expect("capability recipe")
}

fn child_capability_recipe() -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder = OperationBuilder::<Response, Never>::new(
        stable("mfm.fixture/child-capability-recipe"),
        stable("capability-root"),
    )
    .expect("child capability recipe builder");
    let input = builder
        .input::<Request>(stable("capability-input"))
        .expect("child capability recipe input");
    let output = builder
        .root()
        .child::<CopyChild>(
            stable("qualified-child"),
            vec![input.bind_child(stable("child-request"))],
        )
        .expect("qualified child")
        .infallible()
        .expect("qualified child completion");
    let completion = builder
        .succeed(&output)
        .expect("child capability recipe success");
    builder.finish(completion).expect("child capability recipe")
}

fn nested_child_capability_recipe() -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder = OperationBuilder::<Response, Never>::new(
        stable("mfm.fixture/nested-child-capability-recipe"),
        stable("capability-root"),
    )
    .expect("nested child capability recipe builder");
    let input = builder
        .input::<Request>(stable("capability-input"))
        .expect("nested child capability recipe input");
    let output = builder
        .root()
        .child::<NestedCopyChild>(
            stable("qualified-nested-child"),
            vec![input.bind_child(stable("nested-child-request"))],
        )
        .expect("qualified nested child")
        .infallible()
        .expect("qualified nested child completion");
    let completion = builder
        .succeed(&output)
        .expect("nested child capability recipe success");
    builder
        .finish(completion)
        .expect("nested child capability recipe")
}

fn recursive_child_capability_recipe() -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder = OperationBuilder::<Response, Never>::new(
        stable("mfm.fixture/recursive-child-capability-recipe"),
        stable("capability-root"),
    )
    .expect("recursive child capability recipe builder");
    let input = builder
        .input::<Request>(stable("capability-input"))
        .expect("recursive child capability recipe input");
    let output = builder
        .root()
        .child::<RecursiveSupportChild>(
            stable("recursive-support-child"),
            vec![input.bind_child(stable("recursive-support-child-request"))],
        )
        .expect("recursive support child")
        .infallible()
        .expect("recursive support child completion");
    let completion = builder
        .succeed(&output)
        .expect("recursive child capability recipe success");
    builder
        .finish(completion)
        .expect("recursive child capability recipe")
}

fn recovering_child_capability_recipe() -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder = OperationBuilder::<Response, Never>::new(
        stable("mfm.fixture/recovering-child-capability-recipe"),
        stable("capability-root"),
    )
    .expect("recovering child capability recipe builder");
    let input = builder
        .input::<Request>(stable("capability-input"))
        .expect("recovering child capability recipe input");
    let output = builder
        .root()
        .child::<RecoverableChild>(
            stable("recoverable-child"),
            vec![input.bind_child(stable("recoverable-child-input"))],
        )
        .expect("recoverable support child")
        .on_failure::<RecoveryHandler>()
        .expect("recoverable support child handler");
    let completion = builder
        .succeed(&output)
        .expect("recovering child capability recipe success");
    builder
        .finish(completion)
        .expect("recovering child capability recipe")
}

fn recursive_capability_recipe() -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder = OperationBuilder::<Response, Never>::new(
        stable("mfm.fixture/recursive-capability-recipe"),
        stable("capability-root"),
    )
    .expect("recursive recipe builder");
    let input = builder
        .input::<Request>(stable("capability-input"))
        .expect("recursive recipe input");
    let output = builder
        .root()
        .state::<AbstractCapabilityState>(stable("recursive-abstract-read"), &input)
        .expect("nested abstract state")
        .infallible()
        .expect("nested abstract completion");
    let completion = builder.succeed(&output).expect("recursive recipe success");
    builder
        .finish(completion)
        .expect("recursive capability recipe")
}

fn fallible_capability_recipe() -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder = OperationBuilder::<Response, StateFailure>::new(
        stable("mfm.fixture/fallible-capability-recipe"),
        stable("capability-root"),
    )
    .expect("recipe builder");
    builder
        .root()
        .failure_map::<StateFailure, StateFailureMapper>()
        .expect("state-failure mapper");
    let input = builder
        .input::<Request>(stable("pipeline-child-input"))
        .expect("recipe input");
    let output = builder
        .root()
        .state::<FallibleState>(stable("qualified-read"), &input)
        .expect("concrete fallible state")
        .or_default()
        .expect("concrete failure completion");
    let completion = builder.succeed(&output).expect("recipe success");
    builder.finish(completion).expect("capability recipe")
}

fn pipeline_child_program() -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder = OperationBuilder::<Response, StateFailure>::new(
        stable("mfm.fixture/pipeline-child"),
        stable("child-root"),
    )
    .expect("child builder");
    builder
        .root()
        .failure_map::<StateFailure, StateFailureMapper>()
        .expect("child failure mapper");
    let input = builder
        .input::<Request>(stable("pipeline-child-input"))
        .expect("child input");
    let output = builder
        .root()
        .state::<AbstractFallibleCapabilityState>(stable("abstract-read"), &input)
        .expect("abstract capability")
        .or_default()
        .expect("abstract completion");
    let completion = builder.succeed(&output).expect("child success");
    builder.finish(completion).expect("pipeline child")
}

fn typed_outer_policy_recipe() -> PolicyExpansionRecipe {
    let (mut builder, input, proceed) =
        PolicyRecipeBuilder::<Request, Response, StateFailure>::new(
            stable("mfm.fixture/typed-outer-policy"),
            stable("policy-root"),
        )
        .expect("outer typed policy builder");
    let checked = builder
        .root()
        .state::<OuterPreState>(stable("outer-pre"), &input)
        .expect("outer pre")
        .infallible()
        .expect("outer pre completion");
    let protected = proceed
        .call(builder.root(), stable("proceed"), &checked)
        .expect("outer proceed");
    let output = builder
        .root()
        .state::<OuterPostState>(stable("outer-post"), &protected)
        .expect("outer post")
        .infallible()
        .expect("outer post completion");
    let completion = builder.succeed(&output).expect("outer success");
    builder.finish(completion).expect("outer policy")
}

fn full_pipeline_program(
    operation_id: StableId,
) -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder = OperationBuilder::<Response, RootFailure>::new(operation_id, stable("root"))
        .expect("pipeline builder");
    builder
        .root()
        .failure_map::<StateFailure, RootFailureMapper>()
        .expect("root failure mapper");
    let input = builder.input::<Request>(stable("request")).expect("input");
    let _child = builder
        .root()
        .child::<PipelineChild>(
            stable("pipeline-child"),
            vec![input.bind_child(stable("pipeline-child-input"))],
        )
        .expect("pipeline child call")
        .or_default()
        .expect("child failure completion");

    let mut outer = builder
        .root()
        .fan_out::<PipelineInnerJoin, Never>(stable("outer-fan-out"))
        .expect("outer fan-out");
    outer
        .lane(stable("outer-a"), |lane| {
            let mut inner = lane.fan_out::<Response, Never>(stable("inner-a"))?;
            inner.lane(stable("inner-a-0"), |inner_lane| {
                let output = inner_lane
                    .state::<LaneAState>(stable("lane-a"), &input)?
                    .infallible()?;
                inner_lane.normal(&output)
            })?;
            inner.lane(stable("inner-a-1"), |inner_lane| {
                let output = inner_lane
                    .state::<LaneBState>(stable("lane-b"), &input)?
                    .infallible()?;
                inner_lane.normal(&output)
            })?;
            let joined = inner.finish()?;
            lane.normal(&joined)
        })
        .expect("outer lane A");
    outer
        .lane(stable("outer-b"), |lane| {
            let mut inner = lane.fan_out::<Response, Never>(stable("inner-b"))?;
            inner.lane(stable("inner-b-0"), |inner_lane| {
                let output = inner_lane
                    .state::<LaneBState>(stable("lane-b"), &input)?
                    .infallible()?;
                inner_lane.normal(&output)
            })?;
            let joined = inner.finish()?;
            lane.normal(&joined)
        })
        .expect("outer lane B");
    let joined = outer.finish().expect("outer join");
    let aggregate = builder
        .root()
        .state::<PipelineAggregateState>(stable("aggregate"), &joined)
        .expect("aggregate")
        .infallible()
        .expect("aggregate completion");
    let completion = builder.succeed(&aggregate).expect("pipeline success");
    builder.finish(completion).expect("full pipeline")
}

fn certify_capability_fixture(
    operation_id: &StableId,
    authored: &mfm_spec::structured::AuthoredStructuredProgram,
    reverse_registration: bool,
) -> mfm_certify::structured::CertifiedProgram {
    let expected_requirement = state_contract::<AbstractCapabilityState>()
        .expect("abstract state contract")
        .capability_requirement_ref
        .expect("capability requirement");
    let mut assembly = ProgramRegistryBuilder::new();
    register_fixture_runtime_components(&mut assembly, reverse_registration);
    assembly
        .register_value::<Request>()
        .expect("request contract");
    assembly
        .register_value::<Response>()
        .expect("response contract");
    if reverse_registration {
        assembly
            .register_fixture_state::<ConcreteCapabilityState>(stable(
                "mfm.fixture/concrete-capability-implementation",
            ))
            .expect("concrete state registration");
        assembly
            .register_capability_state::<AbstractCapabilityState>()
            .expect("abstract state registration");
    } else {
        assembly
            .register_capability_state::<AbstractCapabilityState>()
            .expect("abstract state registration");
        assembly
            .register_fixture_state::<ConcreteCapabilityState>(stable(
                "mfm.fixture/concrete-capability-implementation",
            ))
            .expect("concrete state registration");
    }
    assert_eq!(
        assembly
            .register_capability_expansion::<CapabilityRecipeExpansion>()
            .expect("capability recipe registration"),
        expected_requirement
    );
    assembly
        .register_entry_point(operation_id.clone(), authored.clone(), profile())
        .expect("entry-point registration");
    let registry = assembly
        .build(std::slice::from_ref(operation_id))
        .expect("qualified registry");
    registry
        .certifier(operation_id)
        .expect("qualified certifier")
        .certify(authored.clone())
        .expect("certified capability program")
}

// InvalidEvidence and Failure are unrepresentable on the SuccessOnly safe-failure
// callback: see program compile-fail tests
// `success_only_safe_failure_cannot_fail` and
// `success_only_safe_failure_cannot_invalid_evidence`.

fn certify_full_pipeline_fixture(
    operation_id: &StableId,
    reverse_registration: bool,
) -> mfm_certify::structured::CertifiedProgram {
    let authored = full_pipeline_program(operation_id.clone());
    let protected =
        state_contract::<AbstractFallibleCapabilityState>().expect("abstract state contract");
    let outer_recipe = typed_outer_policy_recipe();
    let inner_recipe = guarded_policy_recipe();
    let outer = ExpansionPolicyContract::new(vec![PolicyExpansionBinding {
        boundary_contract_ref: protected.state_contract_ref.clone(),
        recipe_ref: outer_recipe.content_ref().expect("outer recipe ref"),
    }])
    .expect("outer policy");
    let inner = ExpansionPolicyContract::new(vec![PolicyExpansionBinding {
        boundary_contract_ref: protected.state_contract_ref,
        recipe_ref: inner_recipe.content_ref().expect("inner recipe ref"),
    }])
    .expect("inner policy");
    let mut pipeline_profile = profile();
    pipeline_profile.policies = vec![outer, inner];

    let mut assembly = ProgramRegistryBuilder::new();
    register_fixture_runtime_components(&mut assembly, reverse_registration);
    for register in [
        ProgramRegistryBuilder::register_value::<Request>,
        ProgramRegistryBuilder::register_value::<Response>,
        ProgramRegistryBuilder::register_value::<StateFailure>,
        ProgramRegistryBuilder::register_value::<RootFailure>,
        ProgramRegistryBuilder::register_closed_sum::<StateFailureRoute>,
        ProgramRegistryBuilder::register_closed_sum::<DefaultRoute>,
        ProgramRegistryBuilder::register_closed_sum::<GuardDecision>,
    ] {
        register(&mut assembly).expect("pipeline value contract");
    }
    assembly
        .register_capability_state::<AbstractFallibleCapabilityState>()
        .expect("abstract capability registration");

    let register_states = |assembly: &mut ProgramRegistryBuilder| {
        assembly.register_fixture_state::<FallibleState>(stable(
            "mfm.fixture/fallible-state-implementation",
        ))?;
        assembly.register_fixture_state::<StateFailureMapperState>(stable(
            "mfm.fixture/state-failure-mapper-implementation",
        ))?;
        assembly.register_fixture_state::<RootFailureMapperState>(stable(
            "mfm.fixture/root-failure-mapper-implementation",
        ))?;
        assembly.register_fixture_state::<OuterPreState>(stable(
            "mfm.fixture/outer-pre-implementation",
        ))?;
        assembly.register_fixture_state::<OuterPostState>(stable(
            "mfm.fixture/outer-post-implementation",
        ))?;
        assembly.register_fixture_state::<GuardState>(stable(
            "mfm.fixture/guard-state-implementation",
        ))?;
        assembly.register_fixture_state::<FailureAuditState>(stable(
            "mfm.fixture/failure-audit-state-implementation",
        ))?;
        assembly
            .register_fixture_state::<LaneAState>(stable("mfm.fixture/lane-a-implementation"))?;
        assembly
            .register_fixture_state::<LaneBState>(stable("mfm.fixture/lane-b-implementation"))?;
        assembly.register_fixture_state::<PipelineAggregateState>(stable(
            "mfm.fixture/pipeline-aggregate-implementation",
        ))?;
        Ok::<(), mfm_certify::CertifyError>(())
    };
    register_states(&mut assembly).expect("pipeline state registration");

    if reverse_registration {
        assembly
            .register_policy_recipe(inner_recipe)
            .expect("inner recipe registration");
        assembly
            .register_policy_recipe(outer_recipe)
            .expect("outer recipe registration");
        assembly
            .register_capability_expansion::<FallibleCapabilityRecipeExpansion>()
            .expect("capability recipe registration");
        assembly
            .register_child(pipeline_child_program())
            .expect("pipeline child registration");
    } else {
        assembly
            .register_child(pipeline_child_program())
            .expect("pipeline child registration");
        assembly
            .register_capability_expansion::<FallibleCapabilityRecipeExpansion>()
            .expect("capability recipe registration");
        assembly
            .register_policy_recipe(outer_recipe)
            .expect("outer recipe registration");
        assembly
            .register_policy_recipe(inner_recipe)
            .expect("inner recipe registration");
    }
    assembly
        .register_entry_point(operation_id.clone(), authored.clone(), pipeline_profile)
        .expect("pipeline entry registration");
    let registry = assembly
        .build(std::slice::from_ref(operation_id))
        .expect("pipeline registry");
    registry
        .certifier(operation_id)
        .expect("pipeline certifier")
        .certify(authored)
        .expect("full pipeline certification")
}

fn certify_typed_child_fixture(
    operation_id: &StableId,
) -> (
    mfm_certify::structured::QualifiedProgramRegistry,
    mfm_certify::structured::CertifiedProgram,
) {
    let mut assembly = ProgramRegistryBuilder::new();
    register_fixture_runtime_components(&mut assembly, false);
    for register in [
        ProgramRegistryBuilder::register_value::<Request>,
        ProgramRegistryBuilder::register_value::<Response>,
        ProgramRegistryBuilder::register_value::<StateFailure>,
        ProgramRegistryBuilder::register_value::<RootFailure>,
        ProgramRegistryBuilder::register_closed_sum::<StateFailureRoute>,
        ProgramRegistryBuilder::register_closed_sum::<DefaultRoute>,
    ] {
        register(&mut assembly).expect("typed child value contract");
    }
    assembly
        .register_capability_state::<AbstractFallibleCapabilityState>()
        .expect("abstract child state registration");
    assembly
        .register_fixture_state::<FallibleState>(stable(
            "mfm.fixture/fallible-state-implementation",
        ))
        .expect("fallible child state registration");
    assembly
        .register_fixture_state::<StateFailureMapperState>(stable(
            "mfm.fixture/state-failure-mapper-implementation",
        ))
        .expect("child failure mapper registration");
    assembly
        .register_fixture_state::<RootFailureMapperState>(stable(
            "mfm.fixture/root-failure-mapper-implementation",
        ))
        .expect("parent failure mapper registration");
    assembly
        .register_capability_expansion::<FallibleCapabilityRecipeExpansion>()
        .expect("child capability recipe registration");
    assembly
        .register_child(pipeline_child_program())
        .expect("typed child registration");
    let authored = typed_child_parent_program(operation_id.clone());
    assembly
        .register_entry_point(operation_id.clone(), authored.clone(), profile())
        .expect("typed child entry registration");
    let registry = assembly
        .build(std::slice::from_ref(operation_id))
        .expect("typed child registry");
    let certified = registry
        .certifier(operation_id)
        .expect("typed child certifier")
        .certify(authored)
        .expect("typed child certification");
    (registry, certified)
}

fn qualify_typed_fan_out_fixture(
    operation_id: &StableId,
) -> mfm_certify::structured::QualifiedProgramRegistry {
    let mut assembly = ProgramRegistryBuilder::new();
    register_fixture_runtime_components(&mut assembly, false);
    for register in [
        ProgramRegistryBuilder::register_value::<Request>,
        ProgramRegistryBuilder::register_value::<Response>,
        ProgramRegistryBuilder::register_value::<StateFailure>,
        ProgramRegistryBuilder::register_closed_sum::<StateFailureRoute>,
    ] {
        register(&mut assembly).expect("typed fan-out value contract");
    }
    assembly
        .register_fixture_state::<FallibleState>(stable(
            "mfm.fixture/fallible-state-implementation",
        ))
        .expect("fallible lane state registration");
    assembly
        .register_fixture_state::<StateFailureMapperState>(stable(
            "mfm.fixture/state-failure-mapper-implementation",
        ))
        .expect("lane failure mapper registration");
    assembly
        .register_fixture_state::<LaneAState>(stable("mfm.fixture/lane-a-implementation"))
        .expect("successful lane state registration");
    assembly
        .register_entry_point(
            operation_id.clone(),
            typed_fan_out_program(operation_id.clone()),
            profile(),
        )
        .expect("typed fan-out entry registration");
    assembly
        .build(std::slice::from_ref(operation_id))
        .expect("typed fan-out registry")
}

fn certify_guarded_fixture(operation_id: &StableId) -> mfm_certify::structured::CertifiedProgram {
    let authored = fallible_program(operation_id.clone());
    let protected = state_contract::<FallibleState>().expect("fallible contract");
    let recipe = guarded_policy_recipe();
    let policy = ExpansionPolicyContract::new(vec![PolicyExpansionBinding {
        boundary_contract_ref: protected.state_contract_ref,
        recipe_ref: recipe.content_ref().expect("guarded recipe ref"),
    }])
    .expect("guard policy");
    let mut policy_profile = profile();
    policy_profile.policies.push(policy);

    let mut assembly = ProgramRegistryBuilder::new();
    register_fixture_runtime_components(&mut assembly, false);
    for register in [
        ProgramRegistryBuilder::register_value::<Request>,
        ProgramRegistryBuilder::register_value::<Response>,
        ProgramRegistryBuilder::register_value::<StateFailure>,
        ProgramRegistryBuilder::register_value::<RootFailure>,
        ProgramRegistryBuilder::register_closed_sum::<DefaultRoute>,
        ProgramRegistryBuilder::register_closed_sum::<GuardDecision>,
    ] {
        register(&mut assembly).expect("fixture value contract");
    }
    assembly
        .register_fixture_state::<FallibleState>(stable(
            "mfm.fixture/fallible-state-implementation",
        ))
        .expect("fallible state registration");
    assembly
        .register_fixture_state::<RootFailureMapperState>(stable(
            "mfm.fixture/root-failure-mapper-implementation",
        ))
        .expect("mapper registration");
    assembly
        .register_fixture_state::<GuardState>(stable("mfm.fixture/guard-state-implementation"))
        .expect("guard registration");
    assembly
        .register_fixture_state::<FailureAuditState>(stable(
            "mfm.fixture/failure-audit-state-implementation",
        ))
        .expect("failure audit registration");
    assembly
        .register_policy_recipe(recipe)
        .expect("policy recipe registration");
    assembly
        .register_entry_point(operation_id.clone(), authored.clone(), policy_profile)
        .expect("entry-point registration");
    let registry = assembly
        .build(std::slice::from_ref(operation_id))
        .expect("qualified registry");
    registry
        .certifier(operation_id)
        .expect("qualified certifier")
        .certify(authored)
        .expect("certified guarded program")
}

fn identity_policy_recipe() -> PolicyExpansionRecipe {
    let (mut builder, input, proceed) = PolicyRecipeBuilder::<Request, Response, Never>::new(
        stable("mfm.fixture/identity-policy-recipe"),
        stable("policy-root"),
    )
    .expect("policy recipe builder");
    let output = proceed
        .call(builder.root(), stable("proceed"), &input)
        .expect("protected boundary");
    let completion = builder.succeed(&output).expect("policy success");
    builder.finish(completion).expect("policy recipe")
}

fn child_policy_recipe() -> PolicyExpansionRecipe {
    let (mut builder, input, proceed) = PolicyRecipeBuilder::<Request, Response, Never>::new(
        stable("mfm.fixture/child-policy-recipe"),
        stable("policy-root"),
    )
    .expect("child policy recipe builder");
    let _copied = builder
        .root()
        .child::<CopyChild>(
            stable("policy-child"),
            vec![input.bind_child(stable("child-request"))],
        )
        .expect("policy child")
        .infallible()
        .expect("policy child completion");
    let output = proceed
        .call(builder.root(), stable("proceed"), &input)
        .expect("protected boundary");
    let completion = builder.succeed(&output).expect("policy success");
    builder.finish(completion).expect("child policy recipe")
}

fn typed_identity_policy_recipe() -> PolicyExpansionRecipe {
    let (mut builder, input, proceed) =
        PolicyRecipeBuilder::<Request, Response, StateFailure>::new(
            stable("mfm.fixture/typed-identity-policy-recipe"),
            stable("policy-root"),
        )
        .expect("policy recipe builder");
    let output = proceed
        .call(builder.root(), stable("proceed"), &input)
        .expect("protected boundary");
    let completion = builder.succeed(&output).expect("policy success");
    builder.finish(completion).expect("policy recipe")
}

fn typed_policy_recipe_with_failure_post() -> PolicyExpansionRecipe {
    let (mut post, failure) = PolicyFailurePostBuilder::<StateFailure, Never>::new(
        stable("mfm.fixture/failure-post-recipe"),
        stable("failure-post-root"),
    )
    .expect("failure-post builder");
    post.root()
        .state::<FailureAuditState>(stable("audit"), &failure)
        .expect("audit state")
        .infallible()
        .expect("audit completion");
    let post = post.finish().expect("failure-post recipe");

    let (mut builder, input, proceed) =
        PolicyRecipeBuilder::<Request, Response, StateFailure>::new(
            stable("mfm.fixture/failure-post-policy-recipe"),
            stable("policy-root"),
        )
        .expect("policy recipe builder");
    let output = proceed
        .call(builder.root(), stable("proceed"), &input)
        .expect("protected boundary");
    let completion = builder.succeed(&output).expect("policy success");
    builder
        .finish_with_failure_post(completion, post)
        .expect("policy recipe")
}

fn typed_policy_recipe_with_one_failure_mapper() -> PolicyExpansionRecipe {
    let (mut post, failure) = PolicyFailurePostBuilder::<StateFailure, Never>::new(
        stable("mfm.fixture/one-link-failure-post-recipe"),
        stable("one-link-failure-post-root"),
    )
    .expect("failure-post builder");
    let mapped = post
        .root()
        .state::<RedactStateFailure>(stable("redact"), &failure)
        .expect("redact mapper")
        .infallible()
        .expect("redact mapper completion");
    let post = post
        .finish_with_mapped_failure(&mapped)
        .expect("one-link failure-post recipe");

    let (mut builder, input, proceed) =
        PolicyRecipeBuilder::<Request, Response, StateFailure>::new(
            stable("mfm.fixture/one-link-failure-post-policy-recipe"),
            stable("policy-root"),
        )
        .expect("policy recipe builder");
    let output = proceed
        .call(builder.root(), stable("proceed"), &input)
        .expect("protected boundary");
    let completion = builder.succeed(&output).expect("policy success");
    builder
        .finish_with_failure_post(completion, post)
        .expect("policy recipe")
}

fn typed_policy_recipe_with_two_failure_mappers() -> PolicyExpansionRecipe {
    let (mut post, failure) = PolicyFailurePostBuilder::<StateFailure, Never>::new(
        stable("mfm.fixture/two-link-failure-post-recipe"),
        stable("two-link-failure-post-root"),
    )
    .expect("failure-post builder");
    let normalized = post
        .root()
        .state::<NormalizeStateFailure>(stable("normalize"), &failure)
        .expect("normalize mapper")
        .infallible()
        .expect("normalize mapper completion");
    let rebuilt = post
        .root()
        .state::<RebuildStateFailure>(stable("rebuild"), &normalized)
        .expect("rebuild mapper")
        .infallible()
        .expect("rebuild mapper completion");
    let post = post
        .finish_with_mapped_failure(&rebuilt)
        .expect("two-link failure-post recipe");

    let (mut builder, input, proceed) =
        PolicyRecipeBuilder::<Request, Response, StateFailure>::new(
            stable("mfm.fixture/two-link-failure-post-policy-recipe"),
            stable("policy-root"),
        )
        .expect("policy recipe builder");
    let output = proceed
        .call(builder.root(), stable("proceed"), &input)
        .expect("protected boundary");
    let completion = builder.succeed(&output).expect("policy success");
    builder
        .finish_with_failure_post(completion, post)
        .expect("policy recipe")
}

fn typed_policy_recipe_with_fallible_failure_post() -> PolicyExpansionRecipe {
    let (mut post, failure) = PolicyFailurePostBuilder::<StateFailure, StateFailure>::new(
        stable("mfm.fixture/fallible-failure-post-recipe"),
        stable("fallible-failure-post-root"),
    )
    .expect("failure-post builder");
    post.root()
        .failure_map::<PostFailure, PostFailureMapper>()
        .expect("post failure mapper");
    post.root()
        .state::<FailingFailurePostState>(stable("fallible-post"), &failure)
        .expect("fallible post state")
        .or_default()
        .expect("fallible post handler");
    let post = post.finish().expect("failure-post recipe");

    let (mut builder, input, proceed) =
        PolicyRecipeBuilder::<Request, Response, StateFailure>::new(
            stable("mfm.fixture/fallible-failure-post-policy-recipe"),
            stable("policy-root"),
        )
        .expect("policy recipe builder");
    let output = proceed
        .call(builder.root(), stable("proceed"), &input)
        .expect("protected boundary");
    let completion = builder.succeed(&output).expect("policy success");
    builder
        .finish_with_failure_post(completion, post)
        .expect("policy recipe")
}

fn outer_policy_recipe() -> PolicyExpansionRecipe {
    let (mut builder, input, proceed) = PolicyRecipeBuilder::<Request, Response, Never>::new(
        stable("mfm.fixture/outer-policy-recipe"),
        stable("outer-policy-root"),
    )
    .expect("outer recipe builder");
    let checked = builder
        .root()
        .state::<OuterPreState>(stable("outer-pre"), &input)
        .expect("outer pre")
        .infallible()
        .expect("outer pre completion");
    let output = proceed
        .call(builder.root(), stable("proceed"), &checked)
        .expect("outer proceed");
    let output = builder
        .root()
        .state::<OuterPostState>(stable("outer-post"), &output)
        .expect("outer post")
        .infallible()
        .expect("outer post completion");
    let completion = builder.succeed(&output).expect("outer success");
    builder.finish(completion).expect("outer recipe")
}

fn inner_policy_recipe() -> PolicyExpansionRecipe {
    let (mut builder, input, proceed) = PolicyRecipeBuilder::<Request, Response, Never>::new(
        stable("mfm.fixture/inner-policy-recipe"),
        stable("inner-policy-root"),
    )
    .expect("inner recipe builder");
    let checked = builder
        .root()
        .state::<InnerPreState>(stable("inner-pre"), &input)
        .expect("inner pre")
        .infallible()
        .expect("inner pre completion");
    let output = proceed
        .call(builder.root(), stable("proceed"), &checked)
        .expect("inner proceed");
    let output = builder
        .root()
        .state::<InnerPostState>(stable("inner-post"), &output)
        .expect("inner post")
        .infallible()
        .expect("inner post completion");
    let completion = builder.succeed(&output).expect("inner success");
    builder.finish(completion).expect("inner recipe")
}

fn guarded_policy_recipe() -> PolicyExpansionRecipe {
    let (mut post, failure) = PolicyFailurePostBuilder::<StateFailure, Never>::new(
        stable("mfm.fixture/guarded-failure-post"),
        stable("guarded-failure-post-root"),
    )
    .expect("failure-post builder");
    post.root()
        .state::<FailureAuditState>(stable("audit"), &failure)
        .expect("audit state")
        .infallible()
        .expect("audit completion");
    let post = post.finish().expect("failure-post recipe");

    let (mut builder, input, proceed) =
        PolicyRecipeBuilder::<Request, Response, StateFailure>::new(
            stable("mfm.fixture/guarded-policy-recipe"),
            stable("guarded-policy-root"),
        )
        .expect("guarded policy builder");
    let decision = builder
        .root()
        .state::<GuardState>(stable("guard"), &input)
        .expect("guard state")
        .infallible()
        .expect("guard completion");
    let mut proceed = Some(proceed);
    let output = builder
        .root()
        .match_value(stable("decision"), &decision, |arms| {
            arms.arm("allow", stable("allow"), |block, payloads| {
                let request = payloads.value::<Request>(&[stable("request")])?;
                let proceed = proceed.take().ok_or_else(|| {
                    mfm_program::ProgramError::Authoring("guard recipe reused proceed".to_owned())
                })?;
                let output = proceed.call(block, stable("proceed"), &request)?;
                block.normal(&output)
            })?;
            arms.arm("deny", stable("deny"), |block, payloads| {
                let failure = payloads.value::<StateFailure>(&[stable("failure")])?;
                block.scope_failure::<Response>(&failure)
            })
        })
        .expect("exhaustive guard");
    let completion = builder.succeed(&output).expect("guarded success");
    builder
        .finish_with_failure_post(completion, post)
        .expect("guarded policy recipe")
}

fn collect_state_labels<'a>(
    block: &'a mfm_spec::structured::ExpandedBlock,
    labels: &mut Vec<&'a str>,
) {
    for declaration in &block.declarations {
        match declaration {
            ExpandedDeclaration::State(state) => labels.push(state.label.as_str()),
            ExpandedDeclaration::Match(binding) => {
                for arm in &binding.arms {
                    collect_state_labels(&arm.body, labels);
                }
            }
            ExpandedDeclaration::FanOut(group) => {
                for lane in &group.lanes {
                    collect_state_labels(&lane.body, labels);
                }
            }
            ExpandedDeclaration::Fragment(fragment) => {
                collect_state_labels(&fragment.body, labels);
            }
        }
    }
}

fn fixture_propagation_source_contains(
    source: &mfm_spec::structured::LexicalSlot,
    target: &mfm_spec::structured::LexicalSlot,
) -> bool {
    if source == target {
        return true;
    }
    match &source.producer {
        mfm_spec::structured::LexicalProducer::FragmentBoundary {
            role: mfm_spec::structured::ResultRole::TypedFailure,
            source,
            ..
        } => fixture_propagation_source_contains(source, target),
        mfm_spec::structured::LexicalProducer::ScopeFailureMerge {
            declaration_ordered_failure_slots,
            ..
        } => declaration_ordered_failure_slots
            .iter()
            .any(|slot| fixture_propagation_source_contains(slot, target)),
        _ => false,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum OracleOutcome {
    Success(Response),
    Failure(RootFailure),
}

enum OracleFlow {
    Normal(serde_json::Value),
    ScopeFailure(serde_json::Value),
}

enum OracleStateOutcome {
    Success(serde_json::Value),
    Failure(serde_json::Value),
}

enum FailureResolution {
    Recovered(serde_json::Value),
    ScopeFailure(serde_json::Value),
}

struct ExecutionOracle {
    guard_allows: bool,
    protected_fails: bool,
    failure_post_fails: bool,
    reverse_fan_out: bool,
    values: BTreeMap<String, serde_json::Value>,
    calls: Vec<String>,
}

impl ExecutionOracle {
    fn new(guard_allows: bool, protected_fails: bool) -> Self {
        Self {
            guard_allows,
            protected_fails,
            failure_post_fails: false,
            reverse_fan_out: false,
            values: BTreeMap::new(),
            calls: Vec::new(),
        }
    }

    fn execute(
        &mut self,
        program: &mfm_spec::structured::ExpandedStructuredProgram,
        request: Request,
    ) -> Result<OracleOutcome, String> {
        match self.execute_flow(program, request)? {
            OracleFlow::Normal(value) => serde_json::from_value(value)
                .map(OracleOutcome::Success)
                .map_err(|error| error.to_string()),
            OracleFlow::ScopeFailure(value) => serde_json::from_value(value)
                .map(OracleOutcome::Failure)
                .map_err(|error| error.to_string()),
        }
    }

    fn execute_raw(
        &mut self,
        program: &mfm_spec::structured::ExpandedStructuredProgram,
        request: Request,
    ) -> Result<serde_json::Value, String> {
        match self.execute_flow(program, request)? {
            OracleFlow::Normal(value) => Ok(value),
            OracleFlow::ScopeFailure(_) => {
                Err("raw oracle fixture unexpectedly reached root failure".to_owned())
            }
        }
    }

    fn execute_flow(
        &mut self,
        program: &mfm_spec::structured::ExpandedStructuredProgram,
        request: Request,
    ) -> Result<OracleFlow, String> {
        let [root] = program.input_roots.as_slice() else {
            return Err("oracle fixture requires one admission root".to_owned());
        };
        self.insert(
            root,
            serde_json::to_value(request).map_err(|error| error.to_string())?,
        )?;
        self.execute_block(&program.root)
    }

    fn execute_block(
        &mut self,
        block: &mfm_spec::structured::ExpandedBlock,
    ) -> Result<OracleFlow, String> {
        for declaration in &block.declarations {
            if let Some(failure) = self.execute_declaration(declaration)? {
                return Ok(OracleFlow::ScopeFailure(failure));
            }
        }
        match &block.tail {
            mfm_spec::structured::BlockTail::Normal(slot) => {
                Ok(OracleFlow::Normal(self.value(slot)?))
            }
            mfm_spec::structured::BlockTail::ScopeFailure(slot) => {
                Ok(OracleFlow::ScopeFailure(self.value(slot)?))
            }
        }
    }

    fn execute_declaration(
        &mut self,
        declaration: &ExpandedDeclaration,
    ) -> Result<Option<serde_json::Value>, String> {
        match declaration {
            ExpandedDeclaration::State(state) => {
                let input = self.one_input(&state.inputs)?;
                match self.invoke(state, input)? {
                    OracleStateOutcome::Success(value) => {
                        self.insert(&state.output_slot, value)?;
                        Ok(None)
                    }
                    OracleStateOutcome::Failure(value) => {
                        match self.resolve_failure(&state.failure_boundary, value)? {
                            FailureResolution::Recovered(value) => {
                                self.insert(&state.output_slot, value)?;
                                Ok(None)
                            }
                            FailureResolution::ScopeFailure(value) => Ok(Some(value)),
                        }
                    }
                }
            }
            ExpandedDeclaration::Match(binding) => self.execute_match(binding),
            ExpandedDeclaration::Fragment(fragment) => self.execute_fragment(fragment),
            ExpandedDeclaration::FanOut(group) => {
                self.execute_fan_out(group)?;
                Ok(None)
            }
        }
    }

    fn execute_fan_out(
        &mut self,
        group: &mfm_spec::structured::ExpandedFanOut,
    ) -> Result<(), String> {
        let mut order = (0..group.lanes.len()).collect::<Vec<_>>();
        if self.reverse_fan_out {
            order.reverse();
        }
        let mut outcomes = vec![None; group.lanes.len()];
        for index in order {
            let lane = &group.lanes[index];
            let outcome = match self.execute_block(&lane.body)? {
                OracleFlow::Normal(value) => {
                    serde_json::to_value(mfm_spec::structured::LaneOutcome::<
                        serde_json::Value,
                        serde_json::Value,
                    >::Success(value))
                    .map_err(|error| error.to_string())?
                }
                OracleFlow::ScopeFailure(value) => {
                    serde_json::to_value(mfm_spec::structured::LaneOutcome::<
                        serde_json::Value,
                        serde_json::Value,
                    >::Failure(value))
                    .map_err(|error| error.to_string())?
                }
            };
            self.insert(&lane.outcome_slot, outcome.clone())?;
            outcomes[index] = Some(outcome);
        }
        let joined = outcomes
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| "fan-out oracle did not complete every lane".to_owned())?;
        self.insert(
            &group.output_slot,
            serde_json::json!({
                "head": joined[0],
                "tail": joined[1..].to_vec(),
            }),
        )
    }

    fn execute_match(
        &mut self,
        binding: &mfm_spec::structured::ExpandedMatch,
    ) -> Result<Option<serde_json::Value>, String> {
        let selector = self.value(&binding.selector)?;
        let tag = selector
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "closed-sum selector has no canonical kind tag".to_owned())?;
        let arm = binding
            .arms
            .iter()
            .find(|arm| arm.canonical_tag == tag)
            .ok_or_else(|| "closed-sum selector tag is not certified".to_owned())?;
        match self.execute_block(&arm.body)? {
            OracleFlow::Normal(value) => {
                self.insert(&binding.output_slot, value)?;
                Ok(None)
            }
            OracleFlow::ScopeFailure(value) => Ok(Some(value)),
        }
    }

    fn execute_fragment(
        &mut self,
        fragment: &mfm_spec::structured::ExpandedFragment,
    ) -> Result<Option<serde_json::Value>, String> {
        for input in &fragment.input_bindings {
            let value = self.value(&input.caller_slot)?;
            let rebound = mfm_spec::structured::LexicalSlot {
                lexical_path: fragment.path.clone(),
                contract_ref: input.child_contract_ref.clone(),
                producer: mfm_spec::structured::LexicalProducer::FragmentInput {
                    boundary_id: fragment.boundary_id.clone(),
                    child_root_id: input.child_root_id.clone(),
                    source: Box::new(input.caller_slot.clone()),
                },
            };
            self.insert(&rebound, value)?;
        }
        match self.execute_block(&fragment.body)? {
            OracleFlow::Normal(value) => {
                self.insert(&fragment.success_slot, value)?;
                Ok(None)
            }
            OracleFlow::ScopeFailure(value) => {
                match self.resolve_failure(&fragment.failure_boundary, value)? {
                    FailureResolution::Recovered(value) => {
                        self.insert(&fragment.success_slot, value)?;
                        Ok(None)
                    }
                    FailureResolution::ScopeFailure(value) => Ok(Some(value)),
                }
            }
        }
    }

    fn resolve_failure(
        &mut self,
        boundary: &CertifiedFailureBoundary,
        failure: serde_json::Value,
    ) -> Result<FailureResolution, String> {
        let CertifiedFailureBoundary::Typed {
            source_slot, plan, ..
        } = boundary
        else {
            return Err("oracle observed Failure for a certified Never boundary".to_owned());
        };
        self.insert(source_slot, failure)?;
        match plan.as_ref() {
            FailurePlan::Propagate {
                before_boundary,
                mapping_chain,
                ..
            } => {
                let mut prefix = before_boundary.as_ref().clone();
                prefix.tail = mfm_spec::structured::BlockTail::Normal(source_slot.clone());
                let mut mapped = match self.execute_block(&prefix)? {
                    OracleFlow::Normal(value) => value,
                    OracleFlow::ScopeFailure(value) => {
                        return Ok(FailureResolution::ScopeFailure(value));
                    }
                };
                for link in mapping_chain {
                    mapped = match self.invoke(&link.mapper, mapped)? {
                        OracleStateOutcome::Success(value) => value,
                        OracleStateOutcome::Failure(_) => {
                            return Err("certified Pure + Never failure mapper failed".to_owned());
                        }
                    };
                    self.insert(&link.output_slot, mapped.clone())?;
                }
                Ok(FailureResolution::ScopeFailure(mapped))
            }
            FailurePlan::Handled {
                before_handler,
                handler,
                continuation,
                ..
            } => {
                let mapped = match self.execute_block(before_handler)? {
                    OracleFlow::Normal(value) => value,
                    OracleFlow::ScopeFailure(value) => {
                        return Ok(FailureResolution::ScopeFailure(value));
                    }
                };
                let handled = match self.invoke(handler, mapped)? {
                    OracleStateOutcome::Success(value) => value,
                    OracleStateOutcome::Failure(_) => {
                        return Err("certified Pure + Never handler failed".to_owned());
                    }
                };
                self.insert(&handler.output_slot, handled)?;
                match continuation.as_ref() {
                    mfm_spec::structured::HandlerContinuation::DefaultPropagation {
                        payload_slot,
                        ..
                    } => Ok(FailureResolution::ScopeFailure(self.value(payload_slot)?)),
                    mfm_spec::structured::HandlerContinuation::CustomRecovery { arms, .. } => {
                        let selector = self.value(&handler.output_slot)?;
                        let tag = selector
                            .get("kind")
                            .and_then(serde_json::Value::as_str)
                            .ok_or_else(|| "custom handler route has no tag".to_owned())?;
                        let arm = arms
                            .iter()
                            .find(|arm| arm.canonical_tag == tag)
                            .ok_or_else(|| "custom handler route is not exhaustive".to_owned())?;
                        match self.execute_block(&arm.body)? {
                            OracleFlow::Normal(value) => Ok(FailureResolution::Recovered(value)),
                            OracleFlow::ScopeFailure(value) => {
                                Ok(FailureResolution::ScopeFailure(value))
                            }
                        }
                    }
                }
            }
        }
    }

    fn invoke(
        &mut self,
        state: &mfm_spec::structured::ExpandedStateBinding,
        input: serde_json::Value,
    ) -> Result<OracleStateOutcome, String> {
        let id = state.contract.semantic_state_id.as_str();
        self.calls.push(id.to_owned());
        match id {
            "mfm.fixture/guard-state" => {
                let request: Request =
                    serde_json::from_value(input).map_err(|error| error.to_string())?;
                let decision = if self.guard_allows {
                    GuardDecision::Allow { request }
                } else {
                    GuardDecision::Deny {
                        failure: StateFailure { code: 403 },
                    }
                };
                Ok(OracleStateOutcome::Success(
                    serde_json::to_value(decision).map_err(|error| error.to_string())?,
                ))
            }
            "mfm.fixture/fallible-state" if self.protected_fails => {
                Ok(OracleStateOutcome::Failure(
                    serde_json::to_value(StateFailure { code: 7 })
                        .map_err(|error| error.to_string())?,
                ))
            }
            "mfm.fixture/fallible-state" => {
                let request: Request =
                    serde_json::from_value(input).map_err(|error| error.to_string())?;
                Ok(OracleStateOutcome::Success(
                    serde_json::to_value(Response {
                        value: request.value + 1,
                    })
                    .map_err(|error| error.to_string())?,
                ))
            }
            "mfm.fixture/outer-pre" | "mfm.fixture/outer-post" => {
                Ok(OracleStateOutcome::Success(input))
            }
            "mfm.fixture/failure-audit-state" => Ok(OracleStateOutcome::Success(input)),
            "mfm.fixture/redact-state-failure" => {
                let failure: StateFailure =
                    serde_json::from_value(input).map_err(|error| error.to_string())?;
                Ok(OracleStateOutcome::Success(
                    serde_json::to_value(StateFailure {
                        code: failure.code.min(999),
                    })
                    .map_err(|error| error.to_string())?,
                ))
            }
            "mfm.fixture/normalize-state-failure" => {
                let failure: StateFailure =
                    serde_json::from_value(input).map_err(|error| error.to_string())?;
                Ok(OracleStateOutcome::Success(
                    serde_json::to_value(NormalizedFailure {
                        code: failure.code + 10,
                    })
                    .map_err(|error| error.to_string())?,
                ))
            }
            "mfm.fixture/rebuild-state-failure" => {
                let failure: NormalizedFailure =
                    serde_json::from_value(input).map_err(|error| error.to_string())?;
                Ok(OracleStateOutcome::Success(
                    serde_json::to_value(StateFailure {
                        code: failure.code + 100,
                    })
                    .map_err(|error| error.to_string())?,
                ))
            }
            "mfm.fixture/root-failure-mapper" => {
                let failure: StateFailure =
                    serde_json::from_value(input).map_err(|error| error.to_string())?;
                Ok(OracleStateOutcome::Success(
                    serde_json::to_value(DefaultRoute::Propagate {
                        failure: RootFailure { code: failure.code },
                    })
                    .map_err(|error| error.to_string())?,
                ))
            }
            "mfm.fixture/state-failure-mapper" => {
                let failure: StateFailure =
                    serde_json::from_value(input).map_err(|error| error.to_string())?;
                Ok(OracleStateOutcome::Success(
                    serde_json::to_value(StateFailureRoute::Propagate { failure })
                        .map_err(|error| error.to_string())?,
                ))
            }
            "mfm.fixture/lane-a" | "mfm.fixture/lane-b" => {
                let request: Request =
                    serde_json::from_value(input).map_err(|error| error.to_string())?;
                let increment = if id == "mfm.fixture/lane-a" { 10 } else { 20 };
                Ok(OracleStateOutcome::Success(
                    serde_json::to_value(Response {
                        value: request.value + increment,
                    })
                    .map_err(|error| error.to_string())?,
                ))
            }
            "mfm.fixture/pipeline-aggregate" => {
                let joined: PipelineOuterJoin =
                    serde_json::from_value(input).map_err(|error| error.to_string())?;
                Ok(OracleStateOutcome::Success(
                    serde_json::to_value(Response {
                        value: joined.len() as u64,
                    })
                    .map_err(|error| error.to_string())?,
                ))
            }
            "mfm.fixture/recovery-handler" => Ok(OracleStateOutcome::Success(
                serde_json::to_value(RecoveryRoute::Recover {
                    response: Response { value: 77 },
                })
                .map_err(|error| error.to_string())?,
            )),
            "mfm.fixture/recovery-read" => Ok(OracleStateOutcome::Success(input)),
            "mfm.fixture/failing-failure-post" if self.failure_post_fails => {
                Ok(OracleStateOutcome::Failure(
                    serde_json::to_value(PostFailure { code: 91 })
                        .map_err(|error| error.to_string())?,
                ))
            }
            "mfm.fixture/failing-failure-post" => Ok(OracleStateOutcome::Success(input)),
            "mfm.fixture/post-failure-mapper" => {
                let failure: PostFailure =
                    serde_json::from_value(input).map_err(|error| error.to_string())?;
                Ok(OracleStateOutcome::Success(
                    serde_json::to_value(PostFailureRoute::Propagate {
                        failure: StateFailure { code: failure.code },
                    })
                    .map_err(|error| error.to_string())?,
                ))
            }
            other => Err(format!("oracle has no callback for {other}")),
        }
    }

    fn one_input(
        &self,
        inputs: &[mfm_spec::structured::LexicalSlot],
    ) -> Result<serde_json::Value, String> {
        let [input] = inputs else {
            return Err("oracle fixture state does not have one input".to_owned());
        };
        self.value(input)
    }

    fn insert(
        &mut self,
        slot: &mfm_spec::structured::LexicalSlot,
        value: serde_json::Value,
    ) -> Result<(), String> {
        let key = slot_key(slot)?;
        if let Some(existing) = self.values.insert(key, value.clone()) {
            if existing != value {
                return Err("oracle slot was rebound to different bytes".to_owned());
            }
        }
        Ok(())
    }

    fn value(&self, slot: &mfm_spec::structured::LexicalSlot) -> Result<serde_json::Value, String> {
        if let Some(value) = self.values.get(&slot_key(slot)?) {
            return Ok(value.clone());
        }
        match &slot.producer {
            mfm_spec::structured::LexicalProducer::FragmentInput { source, .. }
            | mfm_spec::structured::LexicalProducer::FragmentBoundary { source, .. }
            | mfm_spec::structured::LexicalProducer::ArmValue { source, .. } => self.value(source),
            mfm_spec::structured::LexicalProducer::VariantPayload {
                selector,
                canonical_tag,
                payload_path,
            } => {
                let selected = self.value(selector)?;
                if selected.get("kind").and_then(serde_json::Value::as_str)
                    != Some(canonical_tag.as_str())
                {
                    return Err("inactive closed-sum payload was requested".to_owned());
                }
                let mut payload = &selected;
                for segment in payload_path {
                    payload = payload.get(segment.as_str()).ok_or_else(|| {
                        "closed-sum payload path is absent from canonical value".to_owned()
                    })?;
                }
                Ok(payload.clone())
            }
            _ => Err("oracle value is not committed for this exact slot".to_owned()),
        }
    }
}

fn slot_key(slot: &mfm_spec::structured::LexicalSlot) -> Result<String, String> {
    let reference = slot.content_ref().map_err(|error| error.to_string())?;
    serde_json::to_string(&reference).map_err(|error| error.to_string())
}

fn canonical_fixture_bytes<T: Serialize>(value: &T) -> mfm_canonical::PlainCanonicalJsonBytes {
    let json = serde_json::to_string(value).expect("fixture JSON serialization");
    mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&json).expect("fixture canonical JSON")
}

fn fixture_component_with_same_schema(
    template: &CertifiedComponentObject,
    value: serde_json::Value,
) -> CertifiedComponentObject {
    let value = mfm_spec::CanonicalJsonValue::new(value).expect("canonical component value");
    let canonical = value.canonical_json().expect("canonical component bytes");
    let content_ref = ContentRef::new(
        template.content_ref.schema_id().clone(),
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            sha256_digest_bytes(canonical.as_bytes()),
        ),
    )
    .expect("component content reference");
    CertifiedComponentObject {
        object_type: template.object_type.clone(),
        content_ref,
        value,
    }
}

fn mutate_component_value(
    document: &mut mfm_spec::structured::CertifiedProgramDocument,
    current_ref: &ContentRef,
    mutate: impl FnOnce(&mut serde_json::Value),
) -> ContentRef {
    let index = document
        .component_closure
        .iter()
        .position(|object| &object.content_ref == current_ref)
        .expect("component selected for mutation");
    let template = document.component_closure[index].clone();
    let mut value = template.value.as_json().clone();
    mutate(&mut value);
    let replacement = fixture_component_with_same_schema(&template, value);
    let replacement_ref = replacement.content_ref.clone();
    document.component_closure[index] = replacement;
    replacement_ref
}

fn repoint_program_evidence(
    document: &mut mfm_spec::structured::CertifiedProgramDocument,
    field: &str,
    program_ref: &ContentRef,
) {
    let proof_ref = document.root.components.expansion_proof_ref.clone();
    let replacement_proof = mutate_component_value(document, &proof_ref, |value| {
        value[field] = serde_json::to_value(program_ref).expect("program reference JSON");
    });
    document.root.components.expansion_proof_ref = replacement_proof;

    let coverage_ref = document.root.components.policy_coverage_proof_ref.clone();
    let replacement_coverage = mutate_component_value(document, &coverage_ref, |value| {
        value[field] = serde_json::to_value(program_ref).expect("program reference JSON");
    });
    document.root.components.policy_coverage_proof_ref = replacement_coverage;
}

fn refresh_fixture_closure_digest(document: &mut mfm_spec::structured::CertifiedProgramDocument) {
    document.root.canonical_component_closure_digest =
        fixture_closure_digest(&document.root.components, &document.component_closure);
}

fn fixture_closure_digest(
    components: &CertifiedProgramComponents,
    closure: &[CertifiedComponentObject],
) -> ContentDigest {
    let closure_items = closure
        .iter()
        .map(|object| {
            (
                &object.object_type,
                &object.content_ref,
                object
                    .value
                    .canonical_json()
                    .expect("component canonical bytes")
                    .to_vec(),
            )
        })
        .collect::<Vec<_>>();
    let canonical = canonical_fixture_bytes(&(components, closure_items));
    let mut preimage = b"mfm.certified-program-closure.v1\0".to_vec();
    preimage.extend_from_slice(canonical.as_bytes());
    ContentDigest::from_digest(DigestAlgorithm::Sha256V1, sha256_digest_bytes(&preimage))
}

fn profile() -> StructuredExpansionProfile {
    StructuredExpansionProfile {
        policies: Vec::new(),
        max_occurrences: 32,
        max_declarations: 64,
        max_lanes: 16,
        max_fan_out_depth: 2,
        max_branch_depth: 8,
    }
}

fn register_fixture_resource(assembly: &mut ProgramRegistryBuilder) {
    let semantic_contract_ref = fixture_resource_contract()
        .expect("resource contract")
        .content_ref()
        .expect("resource ref");
    let descriptor = fixture_implementation_descriptor(
        assembly,
        StructuredComponentKind::Resource,
        semantic_contract_ref,
        stable("mfm.fixture/read-resource-implementation"),
    )
    .expect("resource descriptor");
    assembly
        .register_resource_authority::<FixtureResource, _>(
            descriptor,
            Arc::new(FixtureResourceInvoker),
        )
        .expect("resource registration");
}

fn register_fixture_signer(assembly: &mut ProgramRegistryBuilder) {
    let semantic_contract_ref = fixture_signer_contract()
        .expect("signer contract")
        .content_ref()
        .expect("signer ref");
    let descriptor = fixture_implementation_descriptor(
        assembly,
        StructuredComponentKind::Signer,
        semantic_contract_ref,
        stable("mfm.fixture/read-signer-implementation"),
    )
    .expect("signer descriptor");
    assembly
        .register_signer::<FixtureSigner, _>(descriptor, Arc::new(FixtureSignerInvoker))
        .expect("signer registration");
}

fn register_fixture_adapter(assembly: &mut ProgramRegistryBuilder) {
    let semantic_contract_ref = fixture_adapter_contract()
        .expect("adapter contract")
        .content_ref()
        .expect("adapter ref");
    let descriptor = fixture_implementation_descriptor(
        assembly,
        StructuredComponentKind::Adapter,
        semantic_contract_ref,
        stable("mfm.fixture/read-adapter-implementation"),
    )
    .expect("adapter descriptor");
    let certificate = HistoryObject::new(
        stable("mfm.fixture/read-physical-binding"),
        SchemaId::new(
            "mfm.fixture/read-physical-binding",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.fixture/read-physical-binding"),
        )
        .expect("physical-binding schema"),
        "{\"target\":1}",
    )
    .expect("physical-binding certificate");
    assembly
        .register_read_adapter::<FixtureReadCapability, _>(
            descriptor,
            Arc::new(FixtureReadBindingSource {
                binding: Arc::new(FixtureReadAdapter { certificate }),
            }),
        )
        .expect("adapter registration");
}

fn register_fixture_capability(assembly: &mut ProgramRegistryBuilder) {
    let semantic_contract_ref = fixture_capability_contract()
        .expect("capability contract")
        .content_ref()
        .expect("capability ref");
    let descriptor = fixture_implementation_descriptor(
        assembly,
        StructuredComponentKind::Capability,
        semantic_contract_ref,
        stable("mfm.fixture/read-capability-implementation"),
    )
    .expect("capability descriptor");
    assembly
        .register_read_capability::<FixtureReadCapability, _>(
            descriptor,
            Arc::new(FixtureReadCapabilityImplementation),
        )
        .expect("capability registration");
}

fn register_fixture_runtime_components(
    assembly: &mut ProgramRegistryBuilder,
    reverse_registration: bool,
) {
    assembly
        .register_value::<Request>()
        .expect("request contract");
    assembly
        .register_value::<Response>()
        .expect("response contract");
    assembly
        .register_value::<StateFailure>()
        .expect("state-failure contract");
    if reverse_registration {
        register_fixture_capability(assembly);
        register_fixture_adapter(assembly);
        register_fixture_signer(assembly);
        register_fixture_resource(assembly);
    } else {
        register_fixture_resource(assembly);
        register_fixture_signer(assembly);
        register_fixture_adapter(assembly);
        register_fixture_capability(assembly);
    }
}

fn fixture_capability_contract() -> mfm_program::Result<StructuredLiveComponentContract> {
    let adapter = fixture_adapter_contract()?.content_ref()?;
    StructuredLiveComponentContract::new_read_capability(
        stable("mfm.fixture/read-capability"),
        mfm_spec::structured::structured_value_contract_ref::<Request>()?,
        mfm_spec::structured::structured_value_contract_ref::<Response>()?,
        mfm_spec::structured::structured_value_contract_ref::<StateFailure>()?,
        adapter,
    )
    .map_err(Into::into)
}

fn fixture_adapter_contract() -> mfm_program::Result<StructuredLiveComponentContract> {
    let signer = fixture_signer_contract()?.content_ref()?;
    let resource = fixture_resource_contract()?.content_ref()?;
    StructuredLiveComponentContract::new(
        StructuredComponentKind::Adapter,
        stable("mfm.fixture/read-adapter"),
        vec![
            StructuredComponentDependency {
                component_kind: StructuredComponentKind::Signer,
                contract_ref: signer,
            },
            StructuredComponentDependency {
                component_kind: StructuredComponentKind::Resource,
                contract_ref: resource,
            },
        ],
    )
    .map_err(Into::into)
}

fn fixture_signer_contract() -> mfm_program::Result<StructuredLiveComponentContract> {
    StructuredLiveComponentContract::new(
        StructuredComponentKind::Signer,
        stable("mfm.fixture/read-signer"),
        Vec::new(),
    )
    .map_err(Into::into)
}

fn fixture_resource_contract() -> mfm_program::Result<StructuredLiveComponentContract> {
    StructuredLiveComponentContract::new(
        StructuredComponentKind::Resource,
        stable("mfm.fixture/read-resource"),
        Vec::new(),
    )
    .map_err(Into::into)
}

fn stable(value: &str) -> StableId {
    StableId::new(value).expect("stable fixture id")
}
