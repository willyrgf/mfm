//! Authoritative legal recoverability-v1 fixtures for backend and runtime conformance tests.

use mfm_canonical::{
    sha256_digest_bytes, CanonicalValue, PlainCanonicalJsonBytes, RecoverabilityContractV1,
};
use mfm_ids::{
    AppendRequestId, ContentRef, DigestAlgorithm, EntryPointId, FieldPath, InvocationIdentity,
    ObjectEvidenceDigest, SemanticTypeId, StableId, StoreEpoch, StoreScopeId, TenantScopeId,
};
use mfm_journal::v1::{
    ArtifactIdPreimage, ConfiguredValueBinding, ConfiguredValueKey, CrossRunSourceRef,
    ObjectEvidencePreimage, OutputRef, ProducerBinding, RunPhase, ValueRef,
};
use mfm_spec::v1::{
    AuthoredBaseKind, AuthoredInputBinding, AuthoredNode, AuthoredPublicOutputBinding,
    AuthoredSourceSelector, CanonicalAuthoredProgram, CanonicalExpansionPath,
    CanonicalExpansionStep, CanonicalJsonValue, CapabilityBindingManifest, Certificate,
    CertificateProofEntry, CertifiedAdmissionArtifacts, CertifiedFrameBinding,
    CertifiedInputBinding, CertifiedInputDestination, CertifiedJournalProtocolContracts,
    CertifiedNodeContract, CertifiedOutputBinding, CertifiedOutputSlot,
    CertifiedSettlementContract, CertifiedSourceSelector, CertifiedStateExecution,
    EntryPointContract, PlanningProfile, PublicOutputContract, RetainedValueContract,
    RunTerminalContract, StateImplementationManifest, StateImplementationManifestEntry,
};

use super::objects::validate_value_contract;
use super::{
    AdmissionMaterial, AdmissionSourceBackend, AdmissionSourceStore, Admit, AdmitRun,
    AsyncInMemoryRunStore, ConfiguredValueBackend, ConfiguredValueStore, ProposedAdmissionInput,
    ProposedAdmissionSourceRoot, ProposedAdmissionSources, QualifiedDeploymentAuthority,
    QualifiedSupportGraph, QualifiedSupportMember, Result, RunAccessAuthority,
    RunAccessAuthorityIssuer, RunJournalBackend, RunJournalStore, StoreError, StoreIdentity,
    SupportBackend, SupportStore, VerifiedRunView,
};

const CONFIG_TARGET: &str = "fixture-config";
const OUTPUT_PATH: &str = "output";
const SOURCE_PATH: &str = "source";
const STATE_KEY: &str = "fixture-state";

/// Deterministic inputs for one legal source-free admission.
///
/// The fixture owns no sealed store authority. Every sealed support, configuration, source, and
/// admission value is obtained from the backend under an issuer supplied by the test.
pub struct LegalAdmissionFixture {
    discriminator: u8,
    store_identity: StoreIdentity,
    tenant_scope_id: TenantScopeId,
    entry_point_id: EntryPointId,
    entry_point_operation_id: StableId,
    invocation_identity: InvocationIdentity,
    qualification_scope_id: SemanticTypeId,
    configured_target: StableId,
    configured_contract: RetainedValueContract,
    configured_binding: ConfiguredValueBinding,
    configured_bytes: PlainCanonicalJsonBytes,
    failure_contract: RetainedValueContract,
    output_contract: RetainedValueContract,
    nested_public_output_bindings: bool,
    requires_effective_output_source: bool,
    append_request_id: AppendRequestId,
    retry_append_request_id: AppendRequestId,
    successor_append_request_id: AppendRequestId,
}

/// One store-prepared legal admission and the exact authority that prepared it.
pub struct PreparedLegalAdmission {
    authority: RunAccessAuthority<Admit>,
    append: AdmitRun,
}

impl PreparedLegalAdmission {
    /// Returns the exact admission-purpose authority.
    pub const fn authority(&self) -> &RunAccessAuthority<Admit> {
        &self.authority
    }

    /// Returns the complete store-prepared admission append.
    pub const fn append(&self) -> &AdmitRun {
        &self.append
    }

    /// Consumes the fixture result into its authority and prepared append.
    pub fn into_parts(self) -> (RunAccessAuthority<Admit>, AdmitRun) {
        (self.authority, self.append)
    }
}

impl LegalAdmissionFixture {
    /// Constructs one deterministic legal fixture namespace.
    pub fn new(discriminator: u8) -> Result<Self> {
        let hex = format!("{discriminator:02x}").repeat(16);
        let store_identity = StoreIdentity::new(
            StoreScopeId::new(format!("{}{}", StoreScopeId::PREFIX, hex))?,
            StoreEpoch::new(1),
        );
        Self::for_store(store_identity, discriminator)
    }

    /// Constructs one legal fixture bound to an already bootstrapped backend identity.
    pub fn for_store(store_identity: StoreIdentity, discriminator: u8) -> Result<Self> {
        let tenant_scope_id = TenantScopeId::new(format!(
            "{}{}",
            TenantScopeId::PREFIX,
            format!("{:02x}", discriminator.wrapping_add(1)).repeat(16)
        ))?;
        Self::for_store_in_tenant(store_identity, tenant_scope_id, discriminator)
    }

    /// Constructs one distinct legal run in an existing fixture tenant.
    ///
    /// The discriminator still gives the run its own invocation, support namespace, configured
    /// target, and append request identities. This permits legal recursive source chains without
    /// introducing tenant or configured-value conflicts.
    pub fn for_store_in_tenant(
        store_identity: StoreIdentity,
        tenant_scope_id: TenantScopeId,
        discriminator: u8,
    ) -> Result<Self> {
        let entry_point_id = EntryPointId::new("mfm.fixture/legal-run@1")?;
        let entry_point_operation_id = stable_id("mfm.fixture/legal-run")?;
        let invocation_identity =
            InvocationIdentity::new(format!("00000000-0000-4000-8000-{discriminator:012x}"))?;
        let qualification_scope_id = semantic_type("qualification-scope", discriminator)?;
        let configured_target = stable_id(&format!("{CONFIG_TARGET}/{discriminator:02x}"))?;
        let configured_contract = retained_contract(
            "configured-value",
            "mfm.fixture.configured-value",
            discriminator,
        )?;
        let configured_bytes = canonical_json(&format!(
            r#"{{"fixture":{discriminator},"kind":"configured"}}"#
        ))?;
        let producer = ProducerBinding::configured_value(
            store_identity.store_scope_id(),
            &tenant_scope_id,
            &entry_point_id,
            &configured_target,
        )?;
        let configured_ref =
            derive_value_ref(&configured_contract, &producer, configured_bytes.as_bytes())?;
        let configured_key = ConfiguredValueKey::new(
            store_identity.store_scope_id(),
            &tenant_scope_id,
            &entry_point_id,
            &configured_target,
        )?;
        let configured_binding = ConfiguredValueBinding::new(&configured_key, &configured_ref)?;
        let failure_contract = retained_contract("state-failure", "mfm.fixture.state-failure", 0)?;
        let output_contract = retained_contract("state-output", "mfm.fixture.state-output", 0)?;

        Ok(Self {
            discriminator,
            store_identity,
            tenant_scope_id,
            entry_point_id,
            entry_point_operation_id,
            invocation_identity,
            qualification_scope_id,
            configured_target,
            configured_contract,
            configured_binding,
            configured_bytes,
            failure_contract,
            output_contract,
            nested_public_output_bindings: false,
            requires_effective_output_source: false,
            append_request_id: append_request_id("fixture-admission", discriminator)?,
            retry_append_request_id: append_request_id("fixture-admission-retry", discriminator)?,
            successor_append_request_id: append_request_id("fixture-successor", discriminator)?,
        })
    }

    /// Makes this fixture certify one whole effective-output source at `source`.
    pub fn with_effective_output_source(mut self) -> Self {
        self.requires_effective_output_source = true;
        self
    }

    /// Makes this fixture project two nested fields from its single successful output.
    pub fn with_nested_public_output_bindings(mut self) -> Self {
        self.nested_public_output_bindings = true;
        self
    }

    /// Returns the authoritative store lineage selected by this fixture.
    pub const fn store_identity(&self) -> &StoreIdentity {
        &self.store_identity
    }

    /// Returns the admitted tenant.
    pub const fn tenant_scope_id(&self) -> &TenantScopeId {
        &self.tenant_scope_id
    }

    /// Returns the exact versioned entry point.
    pub const fn entry_point_id(&self) -> &EntryPointId {
        &self.entry_point_id
    }

    /// Returns the stable admission operation.
    pub const fn entry_point_operation_id(&self) -> &StableId {
        &self.entry_point_operation_id
    }

    /// Returns the logical invocation identity.
    pub const fn invocation_identity(&self) -> &InvocationIdentity {
        &self.invocation_identity
    }

    /// Returns the exact qualified-deployment scope.
    pub const fn qualification_scope_id(&self) -> &SemanticTypeId {
        &self.qualification_scope_id
    }

    /// Returns the configured-value lookup target.
    pub const fn configured_target(&self) -> &StableId {
        &self.configured_target
    }

    /// Returns the certified configured-value contract.
    pub const fn configured_contract(&self) -> &RetainedValueContract {
        &self.configured_contract
    }

    /// Returns the fixture-wide contract used for a successful state output.
    ///
    /// The contract is deliberately stable across fixture run discriminators so one fixture run
    /// can legally consume another fixture run's effective output.
    pub const fn output_contract(&self) -> &RetainedValueContract {
        &self.output_contract
    }

    /// Returns the fixture-wide contract used for a failed state settlement.
    pub const fn failure_contract(&self) -> &RetainedValueContract {
        &self.failure_contract
    }

    /// Returns the exact immutable configured binding to provision in a test backend.
    pub const fn configured_binding(&self) -> &ConfiguredValueBinding {
        &self.configured_binding
    }

    /// Returns exact configured bytes to provision in a test backend.
    pub fn configured_bytes(&self) -> &[u8] {
        self.configured_bytes.as_bytes()
    }

    /// Returns the caller id for the first admission attempt.
    pub const fn append_request_id(&self) -> &AppendRequestId {
        &self.append_request_id
    }

    /// Returns a distinct caller id for an immutable-root retry.
    pub const fn retry_append_request_id(&self) -> &AppendRequestId {
        &self.retry_append_request_id
    }

    /// Returns a distinct caller id for the first legal successor.
    pub const fn successor_append_request_id(&self) -> &AppendRequestId {
        &self.successor_append_request_id
    }

    /// Mints the exact admission authority from a backend's paired issuer.
    pub fn authorize_admission(
        &self,
        issuer: &RunAccessAuthorityIssuer,
    ) -> RunAccessAuthority<Admit> {
        issuer.authorize_admit(
            self.tenant_scope_id.clone(),
            self.entry_point_id.clone(),
            self.entry_point_operation_id.clone(),
            self.invocation_identity.clone(),
        )
    }

    /// Mints exact qualified-deployment authority from a backend's paired issuer.
    pub fn authorize_qualified_deployment(
        &self,
        issuer: &RunAccessAuthorityIssuer,
    ) -> QualifiedDeploymentAuthority {
        issuer.authorize_qualified_deployment(self.qualification_scope_id.clone())
    }

    /// Rebuilds the complete producer-free qualified support graph.
    pub fn qualified_support_graph(&self) -> Result<QualifiedSupportGraph> {
        Ok(self.certification_closure()?.support)
    }

    /// Rebuilds the complete coherent certified admission artifacts.
    pub fn certified_artifacts(&self) -> Result<CertifiedAdmissionArtifacts> {
        Ok(self.certification_closure()?.artifacts)
    }

    /// Rebuilds the exact producer-free admitted input.
    pub fn proposed_input(&self) -> Result<ProposedAdmissionInput> {
        let contract = retained_contract(
            "admission-input",
            "mfm.fixture.admission-input",
            self.discriminator,
        )?;
        Ok(ProposedAdmissionInput::new(canonical_json("{}")?, contract))
    }

    /// Provisions this fixture's immutable configured value in the in-memory test backend.
    pub fn provision_in_memory(&self, store: &AsyncInMemoryRunStore) -> Result<()> {
        store.provision_configured_value(
            self.configured_binding.clone(),
            self.configured_bytes.to_vec(),
        )
    }

    /// Derives one exact effective-output reference from a verified closed source fixture run.
    ///
    /// The helper accepts no raw rows or caller-authored lineage. It requires the source to be a
    /// successful, single-node closed view in the same store and tenant, with the fixture's exact
    /// output contract at ordinal zero.
    pub fn effective_output_source_ref(
        &self,
        source_view: &VerifiedRunView,
    ) -> Result<CrossRunSourceRef> {
        if source_view.store_identity() != &self.store_identity
            || source_view.tenant_scope_id() != &self.tenant_scope_id
            || source_view.run_phase() != RunPhase::Closed
            || !source_view.terminal_succeeded()
        {
            return Err(StoreError::InvalidSourceClosure);
        }
        let [node] = source_view.certified_spec().nodes() else {
            return Err(StoreError::InvalidSourceClosure);
        };
        let transition_ref = source_view
            .node_terminal_transition_ref(node.node_id())
            .ok_or(StoreError::InvalidSourceClosure)?;
        let closure_ref = source_view
            .transition_entries()
            .find(|entry| entry.transition_ref() == transition_ref)
            .and_then(|entry| entry.closure_ref())
            .ok_or(StoreError::InvalidSourceClosure)?;
        let output_ref = OutputRef::new(transition_ref, 0)?;
        let output_binding = source_view.output_binding(&output_ref)?.fields()?;
        validate_value_contract(&self.output_contract, &output_binding.value_ref)?;
        let admission_ref = source_view
            .journal()
            .commits()
            .first()
            .and_then(|commit| commit.records().first())
            .ok_or(StoreError::InvalidSourceClosure)?
            .record_ref(source_view.run_id(), 1)?;

        let source_ref = CanonicalValue::object([
            (
                "kind",
                CanonicalValue::String("effective_output".to_owned()),
            ),
            (
                "source_store_scope_id",
                CanonicalValue::String(
                    source_view
                        .store_identity()
                        .store_scope_id()
                        .as_str()
                        .to_owned(),
                ),
            ),
            (
                "source_store_epoch",
                CanonicalValue::String(
                    source_view.store_identity().store_epoch().get().to_string(),
                ),
            ),
            ("source_admission_ref", admission_ref.canonical_value()?),
            (
                "source_run_id",
                CanonicalValue::String(source_view.run_id().as_str().to_owned()),
            ),
            (
                "source_spec_hash",
                CanonicalValue::String(
                    source_view
                        .certified_spec()
                        .spec_hash()?
                        .as_str()
                        .to_owned(),
                ),
            ),
            (
                "source_node_id",
                CanonicalValue::String(node.node_id().as_str().to_owned()),
            ),
            (
                "effective_transition_ref",
                transition_ref.canonical_value()?,
            ),
            ("effective_output_ref", output_ref.canonical_value()?),
            ("source_closure_ref", closure_ref.canonical_value()?),
        ])
        .map_err(|_| fixture_error("fixture source reference is not canonical"))?;
        CrossRunSourceRef::from_canonical_value(source_ref).map_err(Into::into)
    }

    /// Proposes the exact whole-output root required by a source-consuming fixture.
    pub fn proposed_effective_output_sources(
        &self,
        source_view: &VerifiedRunView,
    ) -> Result<ProposedAdmissionSources> {
        if !self.requires_effective_output_source {
            return Err(StoreError::InvalidSourceClosure);
        }
        let source_ref = self.effective_output_source_ref(source_view)?;
        ProposedAdmissionSources::new(vec![ProposedAdmissionSourceRoot::new(
            field_path(SOURCE_PATH)?,
            source_ref,
            self.output_contract.clone(),
            self.output_contract.evidence_contract_ref().clone(),
        )?])
    }

    /// Resolves all sealed prerequisites and asks the store to prepare the legal admission.
    pub async fn prepare_on<B>(
        &self,
        store: &B,
        issuer: &RunAccessAuthorityIssuer,
    ) -> std::result::Result<PreparedLegalAdmission, B::Error>
    where
        B: RunJournalBackend + SupportBackend + ConfiguredValueBackend + AdmissionSourceBackend,
    {
        self.prepare_on_with_append_request_id(store, issuer, self.append_request_id.clone())
            .await
    }

    /// Verifies a proposed direct source set and prepares this fixture's legal admission.
    pub async fn prepare_on_with_sources<B>(
        &self,
        store: &B,
        issuer: &RunAccessAuthorityIssuer,
        proposed_sources: ProposedAdmissionSources,
    ) -> std::result::Result<PreparedLegalAdmission, B::Error>
    where
        B: RunJournalBackend + SupportBackend + ConfiguredValueBackend + AdmissionSourceBackend,
    {
        self.prepare_on_with_request_and_sources(
            store,
            issuer,
            self.append_request_id.clone(),
            Some(proposed_sources),
        )
        .await
    }

    /// Prepares the same immutable root with the fixture's distinct retry request identity.
    pub async fn prepare_retry_on<B>(
        &self,
        store: &B,
        issuer: &RunAccessAuthorityIssuer,
    ) -> std::result::Result<PreparedLegalAdmission, B::Error>
    where
        B: RunJournalBackend + SupportBackend + ConfiguredValueBackend + AdmissionSourceBackend,
    {
        self.prepare_on_with_append_request_id(store, issuer, self.retry_append_request_id.clone())
            .await
    }

    /// Resolves sealed prerequisites and prepares the root under an explicit request identity.
    pub async fn prepare_on_with_append_request_id<B>(
        &self,
        store: &B,
        issuer: &RunAccessAuthorityIssuer,
        append_request_id: AppendRequestId,
    ) -> std::result::Result<PreparedLegalAdmission, B::Error>
    where
        B: RunJournalBackend + SupportBackend + ConfiguredValueBackend + AdmissionSourceBackend,
    {
        self.prepare_on_with_request_and_sources(store, issuer, append_request_id, None)
            .await
    }

    async fn prepare_on_with_request_and_sources<B>(
        &self,
        store: &B,
        issuer: &RunAccessAuthorityIssuer,
        append_request_id: AppendRequestId,
        proposed_sources: Option<ProposedAdmissionSources>,
    ) -> std::result::Result<PreparedLegalAdmission, B::Error>
    where
        B: RunJournalBackend + SupportBackend + ConfiguredValueBackend + AdmissionSourceBackend,
    {
        let authority = self.authorize_admission(issuer);
        let deployment_authority = self.authorize_qualified_deployment(issuer);
        let closure = self.certification_closure().map_err(B::Error::from)?;
        let support =
            SupportStore::admit_support_graph(store, &deployment_authority, closure.support)
                .await?;
        let configured = ConfiguredValueStore::resolve_configured_value(
            store,
            &authority,
            &self.entry_point_id,
            &self.configured_target,
            &self.configured_contract,
        )
        .await?;
        let sources = match proposed_sources {
            Some(proposed) => {
                AdmissionSourceStore::verify_admission_sources(store, &authority, proposed).await?
            }
            None => AdmissionSourceStore::verify_no_admission_sources(store, &authority).await?,
        };
        let input = self.proposed_input().map_err(B::Error::from)?;
        let append = RunJournalStore::prepare_admission(
            store,
            &authority,
            append_request_id,
            AdmissionMaterial::new(closure.artifacts, input, &configured, &support, &sources),
        )
        .map_err(B::Error::from)?;
        Ok(PreparedLegalAdmission { authority, append })
    }

    fn certification_closure(&self) -> Result<FixtureCertificationClosure> {
        let planner_contract = support_material(
            "planner.contract",
            "planner-contract",
            "mfm.fixture.planner-contract",
            r#"{"kind":"planner-contract"}"#,
            self.discriminator,
        )?;
        let planner_implementation = support_material(
            "planner.implementation",
            "planner-implementation",
            "mfm.fixture.planner-implementation",
            r#"{"kind":"planner-implementation"}"#,
            self.discriminator,
        )?;
        let state_contract = support_material(
            "state.contract",
            "state-contract",
            "mfm.fixture.state-contract",
            r#"{"kind":"state-contract"}"#,
            self.discriminator,
        )?;
        let state_implementation = support_material(
            "state.implementation",
            "state-implementation",
            "mfm.fixture.state-implementation",
            r#"{"kind":"state-implementation"}"#,
            self.discriminator,
        )?;
        let executable_identity = support_material(
            "executable.identity",
            "executable-identity",
            "mfm.qualification.executable-identity",
            r#"{"kind":"executable-identity"}"#,
            self.discriminator,
        )?;

        let planning_profile = PlanningProfile::new(
            planner_contract.content_ref.clone(),
            planner_implementation.content_ref.clone(),
            Vec::new(),
            CanonicalJsonValue::empty_object(),
        )?;
        let input_contract = retained_contract(
            "admission-input",
            "mfm.fixture.admission-input",
            self.discriminator,
        )?;
        let output_contract = self.output_contract.clone();
        let entry_point = EntryPointContract::new(
            self.entry_point_id.clone(),
            self.entry_point_operation_id.clone(),
            planning_profile,
            input_contract.schema_id().clone(),
            output_contract.schema_id().clone(),
        )?;

        let state_key = stable_id(STATE_KEY)?;
        let authored_node = AuthoredNode::new(
            state_key.clone(),
            state_key.clone(),
            Vec::new(),
            AuthoredBaseKind::Authored,
            state_contract.content_ref.clone(),
            self.configured_binding.fields()?.value_ref,
            None,
        )?;
        let authored_input_bindings = if self.requires_effective_output_source {
            vec![AuthoredInputBinding::new(
                state_key.clone(),
                0,
                0,
                AuthoredSourceSelector::CrossRunEffectiveOutput {
                    source_field_path: None,
                },
            )]
        } else {
            Vec::new()
        };
        let authored_public_outputs = if self.nested_public_output_bindings {
            vec![
                AuthoredPublicOutputBinding::new(
                    stable_id("alpha")?,
                    state_key.clone(),
                    0,
                    Some(field_path("nested.first")?),
                ),
                AuthoredPublicOutputBinding::new(
                    stable_id("beta")?,
                    state_key.clone(),
                    0,
                    Some(field_path("nested.second")?),
                ),
            ]
        } else {
            vec![AuthoredPublicOutputBinding::new(
                stable_id(OUTPUT_PATH)?,
                state_key.clone(),
                0,
                None,
            )]
        };
        let authored_program = CanonicalAuthoredProgram::new(
            self.entry_point_operation_id.clone(),
            vec![authored_node],
            authored_input_bindings,
            authored_public_outputs,
            vec![state_key.clone()],
        )?;

        let path = CanonicalExpansionPath::new(vec![
            CanonicalExpansionStep::EntryPoint,
            CanonicalExpansionStep::Authored {
                stable_key: state_key,
                ordinal: 0,
            },
        ])?;
        let config_binding = CertifiedFrameBinding::new(
            self.configured_contract.clone(),
            CertifiedSourceSelector::Config {
                source_field_path: None,
            },
        );
        let settlement = CertifiedSettlementContract::new(
            Some(self.failure_contract.clone()),
            vec![CertifiedOutputSlot::new(
                0,
                field_path(OUTPUT_PATH)?,
                output_contract.clone(),
            )],
            Vec::new(),
        )?;
        let input_bindings = if self.requires_effective_output_source {
            vec![CertifiedInputBinding::new(
                field_path(SOURCE_PATH)?,
                CertifiedInputDestination::OrdinaryValue,
                self.output_contract.clone(),
                vec![CertifiedSourceSelector::CrossRunEffectiveOutput {
                    source_field_path: None,
                }],
            )?]
        } else {
            Vec::new()
        };
        let node = CertifiedNodeContract::new(
            path,
            state_contract.content_ref.clone(),
            config_binding,
            None,
            input_contract,
            input_bindings,
            CertifiedStateExecution::Pure,
            settlement,
        )?;
        let node_id = node.node_id().clone();
        let public_bindings = if self.nested_public_output_bindings {
            vec![
                CertifiedOutputBinding::new(
                    field_path("alpha")?,
                    node_id.clone(),
                    0,
                    Some(field_path("nested.first")?),
                    output_contract.clone(),
                ),
                CertifiedOutputBinding::new(
                    field_path("beta")?,
                    node_id.clone(),
                    0,
                    Some(field_path("nested.second")?),
                    output_contract.clone(),
                ),
            ]
        } else {
            vec![CertifiedOutputBinding::new(
                field_path(OUTPUT_PATH)?,
                node_id.clone(),
                0,
                None,
                output_contract.clone(),
            )]
        };
        let public_output = PublicOutputContract::new(output_contract, public_bindings)?;
        let state_manifest =
            StateImplementationManifest::new(vec![StateImplementationManifestEntry {
                state_contract_ref: state_contract.content_ref.clone(),
                component_implementation_ref: state_implementation.content_ref.clone(),
            }])?;
        let capability_manifest = CapabilityBindingManifest::new(Vec::new())?;
        let expanded_spec = mfm_spec::v1::ExpandedCertifiedSpec::new(
            authored_program.content_ref()?,
            entry_point.planning_profile_ref().clone(),
            vec![node],
            public_output,
            RunTerminalContract::new(vec![node_id], true),
            CertifiedJournalProtocolContracts::current()?,
        )?;
        let expanded_ref = expanded_spec.content_ref()?;
        let entry_point_ref = entry_point.content_ref()?;
        let authored_ref = authored_program.content_ref()?;
        let state_manifest_ref = state_manifest.content_ref()?;
        let capability_manifest_ref = capability_manifest.content_ref()?;
        let certificate = Certificate::new(
            expanded_spec.spec_hash()?,
            expanded_ref.clone(),
            vec![
                proof(
                    "entry-point-profile",
                    entry_point_ref,
                    entry_point.planning_profile_ref().clone(),
                )?,
                proof("authored-program", expanded_ref.clone(), authored_ref)?,
                proof("state-manifest", expanded_ref.clone(), state_manifest_ref)?,
                proof(
                    "capability-manifest",
                    expanded_ref.clone(),
                    capability_manifest_ref,
                )?,
                proof("expanded-spec", expanded_ref.clone(), expanded_ref)?,
            ],
        )?;
        let artifacts = CertifiedAdmissionArtifacts::from_certification(
            entry_point,
            authored_program,
            expanded_spec,
            certificate,
            state_manifest.clone(),
            capability_manifest.clone(),
        )?;

        let state_manifest_support = QualifiedSupportMember::new(
            field_path("manifests.state_implementation")?,
            state_manifest.canonical_json()?,
            StateImplementationManifest::retained_contract()?,
        );
        let capability_manifest_support = QualifiedSupportMember::new(
            field_path("manifests.capability_binding")?,
            capability_manifest.canonical_json()?,
            CapabilityBindingManifest::retained_contract()?,
        );
        let support = QualifiedSupportGraph::new(
            self.qualification_scope_id.clone(),
            [
                planner_contract.member,
                planner_implementation.member,
                state_contract.member,
                state_implementation.member,
                executable_identity.member,
                state_manifest_support,
                capability_manifest_support,
            ],
        )?;
        Ok(FixtureCertificationClosure { artifacts, support })
    }
}

struct FixtureCertificationClosure {
    artifacts: CertifiedAdmissionArtifacts,
    support: QualifiedSupportGraph,
}

struct SupportMaterial {
    member: QualifiedSupportMember,
    content_ref: ContentRef,
}

fn support_material(
    path: &str,
    semantic_name: &str,
    role: &str,
    json: &str,
    discriminator: u8,
) -> Result<SupportMaterial> {
    let canonical = canonical_json(json)?;
    let contract = retained_contract(semantic_name, role, discriminator)?;
    let content_ref = content_ref(&contract, canonical.as_bytes())?;
    Ok(SupportMaterial {
        member: QualifiedSupportMember::new(field_path(path)?, canonical, contract),
        content_ref,
    })
}

fn retained_contract(
    semantic_name: &str,
    role: &str,
    discriminator: u8,
) -> Result<RetainedValueContract> {
    RetainedValueContract::new(
        RecoverabilityContractV1::embedded()?
            .schema_id("mfm.primitive-canonical_value.v1")?
            .clone(),
        semantic_type(semantic_name, discriminator)?,
        stable_id(role)?,
        "application/json",
        EntryPointContract::retained_contract()?
            .evidence_contract_ref()
            .clone(),
    )
    .map_err(|_| fixture_error("fixture retained-value contract is invalid"))
}

fn semantic_type(name: &str, discriminator: u8) -> Result<SemanticTypeId> {
    SemanticTypeId::new(
        "mfm.fixture",
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("mfm.fixture:{name}:{discriminator}").as_bytes()),
    )
    .map_err(Into::into)
}

fn content_ref(contract: &RetainedValueContract, bytes: &[u8]) -> Result<ContentRef> {
    ContentRef::new(
        contract.schema_id().clone(),
        RecoverabilityContractV1::embedded()?.raw_content_digest(bytes),
    )
    .map_err(Into::into)
}

fn derive_value_ref(
    contract: &RetainedValueContract,
    producer: &ProducerBinding,
    bytes: &[u8],
) -> Result<ValueRef> {
    let recoverability = RecoverabilityContractV1::embedded()?;
    let content_digest = recoverability.raw_content_digest(bytes);
    let artifact_id = ArtifactIdPreimage::new(
        contract.schema_id(),
        &content_digest,
        contract.semantic_type_id(),
    )?
    .artifact_id()?;
    let byte_length = u64::try_from(bytes.len())
        .map_err(|_| fixture_error("fixture retained bytes exceed u64"))?;
    let evidence_hash: ObjectEvidenceDigest = ObjectEvidencePreimage::new(
        &artifact_id,
        &content_digest,
        contract.schema_id(),
        byte_length,
        contract.media_type(),
        contract.evidence_contract_ref(),
    )?
    .evidence_hash()?;
    ValueRef::new(
        &artifact_id,
        &content_digest,
        &evidence_hash,
        contract.schema_id(),
        contract.semantic_type_id(),
        contract.role(),
        byte_length,
        contract.media_type(),
        contract.evidence_contract_ref(),
        producer,
    )
    .map_err(Into::into)
}

fn proof(
    kind: &str,
    subject_ref: ContentRef,
    evidence_ref: ContentRef,
) -> Result<CertificateProofEntry> {
    Ok(CertificateProofEntry {
        proof_kind: stable_id(kind)?,
        subject_ref,
        evidence_ref,
    })
}

fn canonical_json(value: &str) -> Result<PlainCanonicalJsonBytes> {
    PlainCanonicalJsonBytes::from_json_str(value)
        .map_err(|_| fixture_error("fixture JSON is not canonicalizable"))
}

fn append_request_id(prefix: &str, discriminator: u8) -> Result<AppendRequestId> {
    AppendRequestId::new(format!("{prefix}/{discriminator:02x}")).map_err(Into::into)
}

fn field_path(value: &str) -> Result<FieldPath> {
    FieldPath::new(value).map_err(Into::into)
}

fn stable_id(value: &str) -> Result<StableId> {
    StableId::new(value).map_err(Into::into)
}

const fn fixture_error(message: &'static str) -> StoreError {
    StoreError::InvalidPreparedAppend {
        purpose: "legal_test_fixture",
        message,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use mfm_canonical::{CanonicalJsonBytes, PlainCanonicalJsonBytes, RecoverabilityContractV1};
    use mfm_ids::TenantScopeId;

    use super::{LegalAdmissionFixture, PreparedLegalAdmission, OUTPUT_PATH};
    use crate::v1::{
        AppendOutcome, AsyncInMemoryRunStore, AsyncStoreFuture, CommittedRunJournal,
        ComparisonSettlementKind, ComparisonTransitionKind, ExistingRunAppendMaterial,
        JournalAppendVerifier, JournalLoadVerifier, NewlyAppended, ObjectGraphProposal,
        ProducedObjectRoot, ProducedOutputSlot, RunAccessAuthorityIssuer, RunJournalBackend,
        RunJournalStore, SettlementMaterial, StoreAuthorityContext, StoreError, TransitionMaterial,
        TransitionTracePageRequest, VerifiedRunView, VerifiedTransitionTracePage,
    };

    struct CountingRunBackend {
        store: AsyncInMemoryRunStore,
        loads: AtomicUsize,
    }

    impl RunJournalBackend for CountingRunBackend {
        type Error = StoreError;

        fn store_authority_context(&self) -> &StoreAuthorityContext {
            RunJournalBackend::store_authority_context(&self.store)
        }

        fn backend_append<'a>(
            &'a self,
            verifier: JournalAppendVerifier,
        ) -> AsyncStoreFuture<'a, AppendOutcome, Self::Error> {
            RunJournalBackend::backend_append(&self.store, verifier)
        }

        fn backend_load<'a>(
            &'a self,
            verifier: JournalLoadVerifier,
        ) -> AsyncStoreFuture<'a, CommittedRunJournal, Self::Error> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            RunJournalBackend::backend_load(&self.store, verifier)
        }
    }

    #[test]
    fn legal_fixture_builds_one_coherent_certification_closure() {
        let fixture = LegalAdmissionFixture::new(7).expect("fixture identifiers");
        fixture
            .certified_artifacts()
            .expect("coherent certified artifacts");
        fixture
            .qualified_support_graph()
            .expect("coherent qualified support");
        fixture.proposed_input().expect("coherent admission input");
    }

    #[tokio::test]
    async fn public_read_performs_exactly_one_backend_load() {
        let fixture = LegalAdmissionFixture::new(13).expect("fixture identifiers");
        let (store, issuer) = AsyncInMemoryRunStore::new(fixture.store_identity().clone());
        fixture
            .provision_in_memory(&store)
            .expect("provision configured value");
        let prepared = fixture
            .prepare_on(&store, &issuer)
            .await
            .expect("prepare legal admission");
        let (admit, append) = prepared.into_parts();
        let outcome = store
            .append_admission(&admit, append)
            .await
            .expect("append fixture admission");
        let AppendOutcome::NewlyAppended(NewlyAppended::RunAdmitted(admitted)) = outcome else {
            panic!("fixture admission must be new");
        };
        let counting = CountingRunBackend {
            store: store.clone(),
            loads: AtomicUsize::new(0),
        };
        let authority = issuer
            .authorize_read_public(fixture.tenant_scope_id().clone(), admitted.run_id().clone());

        counting
            .read_public_run(&authority)
            .await
            .expect("read public run");
        assert_eq!(counting.loads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn source_fixture_shares_tenant_and_output_contract_without_run_key_conflicts() {
        let source = LegalAdmissionFixture::new(9).expect("source fixture");
        let consumer = LegalAdmissionFixture::for_store_in_tenant(
            source.store_identity().clone(),
            source.tenant_scope_id().clone(),
            10,
        )
        .expect("consumer fixture")
        .with_effective_output_source();

        assert_eq!(consumer.tenant_scope_id(), source.tenant_scope_id());
        assert_eq!(consumer.output_contract(), source.output_contract());
        assert_ne!(consumer.invocation_identity(), source.invocation_identity());
        assert_ne!(consumer.configured_target(), source.configured_target());
        let artifacts = consumer
            .certified_artifacts()
            .expect("source-consuming certified artifacts");
        let [node] = artifacts.expanded_spec().nodes() else {
            panic!("fixture must certify exactly one node");
        };
        assert_eq!(node.input_bindings().len(), 1);
    }

    #[tokio::test]
    async fn legal_fixture_prepares_appends_and_projects_a_pure_settlement() {
        let fixture = LegalAdmissionFixture::new(8).expect("fixture identifiers");
        let (store, issuer) = AsyncInMemoryRunStore::new(fixture.store_identity().clone());
        fixture
            .provision_in_memory(&store)
            .expect("provision configured value");
        let prepared = fixture
            .prepare_on(&store, &issuer)
            .await
            .expect("prepare legal admission");
        let (authority, append) = prepared.into_parts();
        let outcome = store
            .append_admission(&authority, append)
            .await
            .expect("append legal admission");
        let AppendOutcome::NewlyAppended(NewlyAppended::RunAdmitted(admitted)) = outcome else {
            panic!("fixture admission was not newly committed");
        };
        let run_id = admitted.run_id().clone();
        assert_eq!(admitted.tenant_scope_id(), fixture.tenant_scope_id());
        assert_eq!(
            admitted
                .committed()
                .journal_head()
                .fields()
                .expect("assigned journal head")
                .run_sequence,
            1
        );

        let drive = issuer.authorize_drive(fixture.tenant_scope_id().clone(), run_id.clone());
        let open = store
            .load_committed_journal(&drive)
            .await
            .expect("load admitted fixture")
            .verify_recorded_history()
            .expect("verify admitted fixture");
        let [node] = open.certified_spec().nodes() else {
            panic!("fixture must certify exactly one node");
        };
        let node_id = node.node_id().clone();
        let [output_slot] = node.settlement_contract().output_slots() else {
            panic!("fixture must certify exactly one output");
        };
        let output_ordinal = output_slot.output_ordinal();
        let output_path = output_slot.field_path().clone();
        let output_contract = output_slot.value_contract().clone();
        let output_schema_id = output_contract.schema_id().clone();
        let public =
            issuer.authorize_read_public(fixture.tenant_scope_id().clone(), open.run_id().clone());
        let open_public = store
            .read_public_run(&public)
            .await
            .expect("read active public view")
            .into_validated();
        assert_eq!(open_public.schema_contract(), "mfm.public-run-view.v1");
        let open_public: serde_json::Value =
            serde_json::from_slice(open_public.as_bytes()).expect("decode active public view");
        assert_eq!(open_public["status"], "active");
        assert_eq!(
            open_public["active"]["ready_node_ids"],
            serde_json::json!([node_id.as_str()])
        );
        assert_eq!(
            open_public["active"]["pending_effects"],
            serde_json::json!([])
        );
        assert_eq!(open_public["public_outputs"], serde_json::json!([]));
        assert_eq!(
            open_public["journal_head"],
            serde_json::from_slice::<serde_json::Value>(open.journal_head().as_bytes())
                .expect("decode open physical head")
        );
        assert_eq!(open_public["journal_head"], open_public["semantic_head"]);
        let frame = store
            .prepare_frame(&drive, &open, &node_id)
            .await
            .expect("prepare pure frame");
        let output =
            PlainCanonicalJsonBytes::from_json_str(r#"{"result":"settled"}"#).expect("output");
        let append = store
            .prepare_append(
                &drive,
                &open,
                fixture.successor_append_request_id().clone(),
                ExistingRunAppendMaterial::Transition(Box::new(TransitionMaterial::PureSettled {
                    prepared_frame: Box::new(frame),
                    settlement: SettlementMaterial::Succeeded {
                        output_roots: vec![ProducedOutputSlot::new(
                            output_ordinal,
                            output_path,
                            ProducedObjectRoot::new(output_contract, output.clone()),
                        )],
                        fact_roots: Vec::new(),
                    },
                    object_graph: ObjectGraphProposal::empty(),
                })),
            )
            .expect("prepare pure settlement");
        assert!(matches!(
            store
                .append(&drive, append)
                .await
                .expect("append settlement"),
            AppendOutcome::NewlyAppended(NewlyAppended::Transition(_))
        ));

        let closed_drive = issuer.authorize_drive(fixture.tenant_scope_id().clone(), run_id);
        let closed = store
            .load_committed_journal(&closed_drive)
            .await
            .expect("load closed fixture")
            .verify_recorded_history()
            .expect("verify closed fixture");
        let public = issuer
            .authorize_read_public(fixture.tenant_scope_id().clone(), closed.run_id().clone());
        let closed_public = store
            .read_public_run(&public)
            .await
            .expect("read succeeded public view")
            .into_validated();
        let closed_public: serde_json::Value =
            serde_json::from_slice(closed_public.as_bytes()).expect("decode succeeded public view");
        assert_eq!(closed_public["status"], "succeeded");
        assert_eq!(closed_public["active"], serde_json::Value::Null);
        let [projected_public_output] = closed_public["public_outputs"]
            .as_array()
            .expect("public outputs array")
            .as_slice()
        else {
            panic!("fixture must project exactly one public output");
        };
        assert_eq!(projected_public_output["name"], OUTPUT_PATH);
        assert_eq!(
            projected_public_output["schema_id"],
            output_schema_id.as_str()
        );
        assert_eq!(
            projected_public_output["value_digest"],
            RecoverabilityContractV1::embedded()
                .expect("recoverability contract")
                .raw_content_digest(output.as_bytes())
                .as_str()
        );
        assert_eq!(
            projected_public_output["value"],
            serde_json::json!({"result": "settled"})
        );
        for forbidden in [
            "access_eligible_node_ids",
            "next_candidate_node_id",
            "waiting_reason",
            "transition_ref",
            "observation_ref",
            "fact",
            "timestamp",
        ] {
            assert!(
                !closed_public
                    .as_object()
                    .expect("public view object")
                    .contains_key(forbidden),
                "public view leaked forbidden field {forbidden}"
            );
        }
        let comparison = closed
            .comparison_frames()
            .expect("project comparison frames");
        let frames = comparison.frames().collect::<Vec<_>>();
        let [projected] = frames.as_slice() else {
            panic!("fixture must have exactly one comparison frame");
        };
        assert_eq!(projected.kind(), ComparisonTransitionKind::PureSettled);
        assert!(projected.request().is_none());
        assert!(projected.evidence().is_empty());
        assert!(projected.blocking_sources().is_empty());
        let state = projected.frame().expect("pure state frame");
        assert_eq!(
            state.config().canonical().as_bytes(),
            fixture.configured_bytes()
        );
        assert_eq!(state.input().canonical().as_bytes(), b"{}");
        assert!(state.context().is_none());
        let settlement = projected
            .recorded_settlement()
            .expect("recorded settlement");
        assert_eq!(settlement.kind(), ComparisonSettlementKind::Succeeded);
        let [projected_output] = settlement.outputs() else {
            panic!("fixture must have exactly one projected output");
        };
        assert_eq!(projected_output.value().canonical(), &output);

        let other_tenant =
            TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "f".repeat(32)))
                .expect("other tenant");
        assert_ne!(&other_tenant, fixture.tenant_scope_id());
        let cross_tenant = issuer.authorize_read_public(other_tenant, closed.run_id().clone());
        let cross_tenant_error = match store.read_public_run(&cross_tenant).await {
            Ok(_) => panic!("cross-tenant load must not reveal the run"),
            Err(error) => error,
        };
        assert_eq!(cross_tenant_error, crate::v1::StoreError::RunNotFound);

        let absent_run = mfm_ids::RunId::parse(format!("run:sha256-jcs-v1:{}", "a".repeat(64)))
            .expect("absent run");
        assert_ne!(&absent_run, closed.run_id());
        let absent = issuer.authorize_read_public(fixture.tenant_scope_id().clone(), absent_run);
        assert!(matches!(
            store.read_public_run(&absent).await,
            Err(StoreError::RunNotFound)
        ));

        let (_, other_issuer) = AsyncInMemoryRunStore::new(store.store_identity().clone());
        let foreign = other_issuer
            .authorize_read_public(fixture.tenant_scope_id().clone(), closed.run_id().clone());
        assert!(matches!(
            store.read_public_run(&foreign).await,
            Err(StoreError::AccessDenied {
                purpose: "read_public"
            })
        ));

        let trace_authority = issuer
            .authorize_inspect_trace(fixture.tenant_scope_id().clone(), closed.run_id().clone());
        let requirements = store
            .discover_transition_trace_sources(
                &trace_authority,
                TransitionTracePageRequest::new(None, 0, 1).expect("trace page request"),
            )
            .await
            .expect("discover trace source requirements");
        assert!(requirements.source_run_ids().is_empty());
        let trace_page = store
            .inspect_transition_trace(&trace_authority, requirements, &[])
            .await
            .expect("inspect transition trace");
        assert_eq!(trace_page.run_id(), closed.run_id());
        assert_eq!(trace_page.at_journal_head(), closed.journal_head());
        assert!(!trace_page.has_more());
        assert_eq!(trace_page.next_index(), None);
        let [trace] = trace_page.transitions() else {
            panic!("fixture must have exactly one transition trace");
        };
        let trace_json = CanonicalJsonBytes::from_value(trace.canonical_value());
        assert!(trace_json
            .as_str()
            .contains(r#""transition_kind":"pure_settled""#));
        assert!(trace_json.as_str().contains(r#""kind":"succeeded""#));
        assert!(trace_json.as_str().contains(r#""result":"settled""#));
    }

    #[tokio::test]
    async fn public_read_projects_nested_bindings_from_the_single_aggregate() {
        let fixture = LegalAdmissionFixture::new(11)
            .expect("fixture identifiers")
            .with_nested_public_output_bindings();
        let (store, issuer) = AsyncInMemoryRunStore::new(fixture.store_identity().clone());
        fixture
            .provision_in_memory(&store)
            .expect("provision configured value");
        let prepared = fixture
            .prepare_on(&store, &issuer)
            .await
            .expect("prepare legal admission");
        let closed = settle_fixture(
            &store,
            &issuer,
            &fixture,
            prepared,
            r#"{"nested":{"first":{"value":"one"},"second":[1,2]}}"#,
        )
        .await;
        let authority = issuer
            .authorize_read_public(fixture.tenant_scope_id().clone(), closed.run_id().clone());
        let public = store
            .read_public_run(&authority)
            .await
            .expect("read nested public view")
            .into_validated();
        let public: serde_json::Value =
            serde_json::from_slice(public.as_bytes()).expect("decode nested public view");
        let outputs = public["public_outputs"]
            .as_array()
            .expect("public outputs array");
        let [alpha, beta] = outputs.as_slice() else {
            panic!("fixture must project both certified public bindings");
        };
        assert_eq!(alpha["name"], "alpha");
        assert_eq!(alpha["value"], serde_json::json!({"value": "one"}));
        assert_eq!(beta["name"], "beta");
        assert_eq!(beta["value"], serde_json::json!([1, 2]));

        let contract = RecoverabilityContractV1::embedded().expect("recoverability contract");
        let alpha_bytes =
            PlainCanonicalJsonBytes::from_json_str(r#"{"value":"one"}"#).expect("alpha bytes");
        let beta_bytes = PlainCanonicalJsonBytes::from_json_str("[1,2]").expect("beta bytes");
        assert_eq!(
            alpha["value_digest"],
            contract.raw_content_digest(alpha_bytes.as_bytes()).as_str()
        );
        assert_eq!(
            beta["value_digest"],
            contract.raw_content_digest(beta_bytes.as_bytes()).as_str()
        );
        assert_eq!(
            alpha["schema_id"],
            fixture.output_contract().schema_id().as_str()
        );
        assert_eq!(
            beta["schema_id"],
            fixture.output_contract().schema_id().as_str()
        );
    }

    #[tokio::test]
    async fn public_read_projects_failed_closure_without_public_outputs() {
        let fixture = LegalAdmissionFixture::new(12).expect("fixture identifiers");
        let (store, issuer) = AsyncInMemoryRunStore::new(fixture.store_identity().clone());
        fixture
            .provision_in_memory(&store)
            .expect("provision configured value");
        let prepared = fixture
            .prepare_on(&store, &issuer)
            .await
            .expect("prepare legal admission");
        let (admit, append) = prepared.into_parts();
        let outcome = store
            .append_admission(&admit, append)
            .await
            .expect("append fixture admission");
        let AppendOutcome::NewlyAppended(NewlyAppended::RunAdmitted(admitted)) = outcome else {
            panic!("fixture admission must be new");
        };
        let drive =
            issuer.authorize_drive(fixture.tenant_scope_id().clone(), admitted.run_id().clone());
        let open = store
            .load_committed_journal(&drive)
            .await
            .expect("load open fixture")
            .verify_recorded_history()
            .expect("verify open fixture");
        let [node] = open.certified_spec().nodes() else {
            panic!("fixture must certify exactly one node");
        };
        let frame = store
            .prepare_frame(&drive, &open, node.node_id())
            .await
            .expect("prepare fixture frame");
        let failure =
            PlainCanonicalJsonBytes::from_json_str(r#"{"code":"expected"}"#).expect("failure");
        let append = store
            .prepare_append(
                &drive,
                &open,
                fixture.successor_append_request_id().clone(),
                ExistingRunAppendMaterial::Transition(Box::new(TransitionMaterial::PureSettled {
                    prepared_frame: Box::new(frame),
                    settlement: SettlementMaterial::Failed {
                        typed_failure_root: Box::new(ProducedObjectRoot::new(
                            fixture.failure_contract().clone(),
                            failure,
                        )),
                    },
                    object_graph: ObjectGraphProposal::empty(),
                })),
            )
            .expect("prepare failed settlement");
        assert!(matches!(
            store
                .append(&drive, append)
                .await
                .expect("append failed settlement"),
            AppendOutcome::NewlyAppended(NewlyAppended::Transition(_))
        ));

        let authority = issuer
            .authorize_read_public(fixture.tenant_scope_id().clone(), admitted.run_id().clone());
        let public = store
            .read_public_run(&authority)
            .await
            .expect("read failed public view")
            .into_validated();
        let public: serde_json::Value =
            serde_json::from_slice(public.as_bytes()).expect("decode failed public view");
        assert_eq!(public["status"], "failed");
        assert_eq!(public["active"], serde_json::Value::Null);
        assert_eq!(public["public_outputs"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn trace_requires_canonical_independent_source_authorities() {
        let (store, issuer, tenant, source_run_id, consumer_run_id) =
            closed_cross_run_consumer().await;

        let root = issuer.authorize_inspect_trace(tenant.clone(), consumer_run_id.clone());
        let requirements = trace_requirements(&store, &root).await;
        assert_eq!(
            requirements.source_run_ids(),
            std::slice::from_ref(&source_run_id)
        );
        let redacted = store
            .inspect_transition_trace(&root, requirements, &[])
            .await
            .expect("inspect redacted trace");
        let redacted_json = only_trace_json(&redacted);
        assert!(redacted_json.contains(r#""kind":"cross_run_redacted""#));
        assert!(!redacted_json.contains(source_run_id.as_str()));

        let root = issuer.authorize_inspect_trace(tenant.clone(), consumer_run_id.clone());
        let requirements = trace_requirements(&store, &root).await;
        let source = issuer.authorize_inspect_trace(tenant.clone(), source_run_id.clone());
        let authorized = store
            .inspect_transition_trace(&root, requirements, &[source])
            .await
            .expect("inspect authorized trace");
        let authorized_json = only_trace_json(&authorized);
        assert!(authorized_json.contains(r#""kind":"cross_run_value""#));
        assert!(authorized_json.contains(r#""result":"source""#));

        let root = issuer.authorize_inspect_trace(tenant.clone(), consumer_run_id.clone());
        let empty_page_requirements = store
            .discover_transition_trace_sources(
                &root,
                TransitionTracePageRequest::new(None, 1, 1).expect("second trace page"),
            )
            .await
            .expect("discover empty second page");
        assert!(
            empty_page_requirements.source_run_ids().is_empty(),
            "a source used only outside the requested page must not be discovered"
        );
        let empty_page = store
            .inspect_transition_trace(&root, empty_page_requirements, &[])
            .await
            .expect("inspect empty second page");
        assert!(empty_page.transitions().is_empty());

        let root = issuer.authorize_inspect_trace(tenant.clone(), consumer_run_id.clone());
        let requirements = trace_requirements(&store, &root).await;
        let duplicate_sources = [
            issuer.authorize_inspect_trace(tenant.clone(), source_run_id.clone()),
            issuer.authorize_inspect_trace(tenant.clone(), source_run_id.clone()),
        ];
        assert!(matches!(
            store
                .inspect_transition_trace(&root, requirements, &duplicate_sources)
                .await,
            Err(StoreError::InvalidTracePage {
                field: "source_authorities"
            })
        ));

        let root = issuer.authorize_inspect_trace(tenant.clone(), consumer_run_id.clone());
        let requirements = trace_requirements(&store, &root).await;
        let other_run = mfm_ids::RunId::parse(format!("run:sha256-jcs-v1:{}", "e".repeat(64)))
            .expect("other run");
        let extra = issuer.authorize_inspect_trace(tenant.clone(), other_run);
        assert!(matches!(
            store
                .inspect_transition_trace(&root, requirements, &[extra])
                .await,
            Err(StoreError::InvalidTracePage {
                field: "source_authorities"
            })
        ));

        let root = issuer.authorize_inspect_trace(tenant.clone(), consumer_run_id.clone());
        let requirements = trace_requirements(&store, &root).await;
        let other_tenant =
            TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "d".repeat(32)))
                .expect("other tenant");
        assert_ne!(other_tenant, tenant);
        let wrong_tenant = issuer.authorize_inspect_trace(other_tenant, source_run_id.clone());
        assert!(matches!(
            store
                .inspect_transition_trace(&root, requirements, &[wrong_tenant])
                .await,
            Err(StoreError::InvalidTracePage {
                field: "source_authorities"
            })
        ));

        let root = issuer.authorize_inspect_trace(tenant.clone(), consumer_run_id.clone());
        let requirements = trace_requirements(&store, &root).await;
        let wrong_root = issuer.authorize_inspect_trace(tenant.clone(), source_run_id.clone());
        assert!(matches!(
            store
                .inspect_transition_trace(&wrong_root, requirements, &[])
                .await,
            Err(StoreError::AccessDenied {
                purpose: "inspect_trace"
            })
        ));

        let root = issuer.authorize_inspect_trace(tenant.clone(), consumer_run_id.clone());
        let requirements = trace_requirements(&store, &root).await;
        let (_, other_issuer) = AsyncInMemoryRunStore::new(store.store_identity().clone());
        let wrong_store_root = other_issuer.authorize_inspect_trace(tenant, consumer_run_id);
        assert!(matches!(
            store
                .inspect_transition_trace(&wrong_store_root, requirements, &[])
                .await,
            Err(StoreError::AccessDenied {
                purpose: "inspect_trace"
            })
        ));
    }

    #[tokio::test]
    async fn authorized_absent_trace_source_matches_omitted_redaction() {
        let (store, issuer, tenant, source_run_id, consumer_run_id) =
            closed_cross_run_consumer().await;

        let root = issuer.authorize_inspect_trace(tenant.clone(), consumer_run_id.clone());
        let requirements = trace_requirements(&store, &root).await;
        let omitted = store
            .inspect_transition_trace(&root, requirements, &[])
            .await
            .expect("inspect omitted source");
        let omitted_json = only_trace_json(&omitted);

        let root = issuer.authorize_inspect_trace(tenant.clone(), consumer_run_id);
        let requirements = trace_requirements(&store, &root).await;
        store
            .remove_run_for_trace_test(&source_run_id)
            .expect("remove source run for authorized-absence test");
        let source = issuer.authorize_inspect_trace(tenant, source_run_id);
        let absent = store
            .inspect_transition_trace(&root, requirements, &[source])
            .await
            .expect("authorized absent source remains redacted");
        assert_eq!(only_trace_json(&absent), omitted_json);
    }

    #[tokio::test]
    async fn corrupt_authorized_trace_source_fails_instead_of_redacting() {
        let (store, issuer, tenant, source_run_id, consumer_run_id) =
            closed_cross_run_consumer().await;
        let root = issuer.authorize_inspect_trace(tenant.clone(), consumer_run_id);
        let requirements = trace_requirements(&store, &root).await;
        store
            .remove_transition_output_for_trace_test(&source_run_id)
            .expect("corrupt source object authority");
        let source = issuer.authorize_inspect_trace(tenant, source_run_id);
        assert!(matches!(
            store
                .inspect_transition_trace(&root, requirements, &[source])
                .await,
            Err(StoreError::MissingObjectAuthority { .. })
        ));
    }

    async fn closed_cross_run_consumer() -> (
        AsyncInMemoryRunStore,
        RunAccessAuthorityIssuer,
        TenantScopeId,
        mfm_ids::RunId,
        mfm_ids::RunId,
    ) {
        let source_fixture = LegalAdmissionFixture::new(20).expect("source fixture");
        let (store, issuer) = AsyncInMemoryRunStore::new(source_fixture.store_identity().clone());
        source_fixture
            .provision_in_memory(&store)
            .expect("provision source");
        let source_prepared = source_fixture
            .prepare_on(&store, &issuer)
            .await
            .expect("prepare source");
        let source_view = settle_fixture(
            &store,
            &issuer,
            &source_fixture,
            source_prepared,
            r#"{"result":"source"}"#,
        )
        .await;
        let source_run_id = source_view.run_id().clone();

        let consumer_fixture = LegalAdmissionFixture::for_store_in_tenant(
            source_fixture.store_identity().clone(),
            source_fixture.tenant_scope_id().clone(),
            21,
        )
        .expect("consumer fixture")
        .with_effective_output_source();
        consumer_fixture
            .provision_in_memory(&store)
            .expect("provision consumer");
        let proposed = consumer_fixture
            .proposed_effective_output_sources(&source_view)
            .expect("propose source");
        let consumer_prepared = consumer_fixture
            .prepare_on_with_sources(&store, &issuer, proposed)
            .await
            .expect("prepare consumer");
        let consumer_view = settle_fixture(
            &store,
            &issuer,
            &consumer_fixture,
            consumer_prepared,
            r#"{"result":"consumer"}"#,
        )
        .await;
        let consumer_run_id = consumer_view.run_id().clone();
        (
            store,
            issuer,
            source_fixture.tenant_scope_id().clone(),
            source_run_id,
            consumer_run_id,
        )
    }

    async fn settle_fixture(
        store: &AsyncInMemoryRunStore,
        issuer: &RunAccessAuthorityIssuer,
        fixture: &LegalAdmissionFixture,
        prepared: PreparedLegalAdmission,
        output: &str,
    ) -> VerifiedRunView {
        let (admit, append) = prepared.into_parts();
        let outcome = store
            .append_admission(&admit, append)
            .await
            .expect("append fixture admission");
        let AppendOutcome::NewlyAppended(NewlyAppended::RunAdmitted(admitted)) = outcome else {
            panic!("fixture admission must be new");
        };
        let drive =
            issuer.authorize_drive(fixture.tenant_scope_id().clone(), admitted.run_id().clone());
        let open = store
            .load_committed_journal(&drive)
            .await
            .expect("load open fixture")
            .verify_recorded_history()
            .expect("verify open fixture");
        let [node] = open.certified_spec().nodes() else {
            panic!("fixture must certify exactly one node");
        };
        let [output_slot] = node.settlement_contract().output_slots() else {
            panic!("fixture must certify exactly one output");
        };
        let frame = store
            .prepare_frame(&drive, &open, node.node_id())
            .await
            .expect("prepare fixture frame");
        let output =
            PlainCanonicalJsonBytes::from_json_str(output).expect("canonical fixture output");
        let append = store
            .prepare_append(
                &drive,
                &open,
                fixture.successor_append_request_id().clone(),
                ExistingRunAppendMaterial::Transition(Box::new(TransitionMaterial::PureSettled {
                    prepared_frame: Box::new(frame),
                    settlement: SettlementMaterial::Succeeded {
                        output_roots: vec![ProducedOutputSlot::new(
                            output_slot.output_ordinal(),
                            output_slot.field_path().clone(),
                            ProducedObjectRoot::new(output_slot.value_contract().clone(), output),
                        )],
                        fact_roots: Vec::new(),
                    },
                    object_graph: ObjectGraphProposal::empty(),
                })),
            )
            .expect("prepare fixture settlement");
        assert!(matches!(
            store
                .append(&drive, append)
                .await
                .expect("append settlement"),
            AppendOutcome::NewlyAppended(NewlyAppended::Transition(_))
        ));
        store
            .load_committed_journal(&drive)
            .await
            .expect("load closed fixture")
            .verify_recorded_history()
            .expect("verify closed fixture")
    }

    async fn trace_requirements(
        store: &AsyncInMemoryRunStore,
        root: &crate::v1::RunAccessAuthority<crate::v1::InspectTrace>,
    ) -> crate::v1::TransitionTraceSourceRequirements {
        store
            .discover_transition_trace_sources(
                root,
                TransitionTracePageRequest::new(None, 0, 1).expect("trace request"),
            )
            .await
            .expect("discover trace requirements")
    }

    fn only_trace_json(page: &VerifiedTransitionTracePage) -> String {
        let [trace] = page.transitions() else {
            panic!("fixture must render exactly one trace");
        };
        CanonicalJsonBytes::from_value(trace.canonical_value())
            .as_str()
            .to_owned()
    }
}
