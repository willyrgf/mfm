#![warn(missing_docs)]
//! Typed replay brokers and verifier contracts for MFM.
//!
//! Replay is intentionally evidence-only. A [`v1::ReplayBroker`] is built from a
//! certified typed execution spec, the authoritative store-owned run stream, and
//! retained artifact evidence. It never constructs transports, SDK clients,
//! or live capability handles.

/// Versioned v1 typed replay contracts.
pub mod v1 {
    use std::collections::BTreeMap;

    use mfm_capabilities::CapabilitySetDescriptor;
    use mfm_certify::{CertifiedRemediationLink, CertifiedSideEffectContract};
    use mfm_events::v1::{self as events, side_effect, ArtifactRole, KernelEventPayload};
    use mfm_ids::{
        AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind, CapabilityVersion,
        ContentDigest, DigestAlgorithm, NodeId, RunId, SchemaId, SeedId, SemanticTypeId,
        SideEffectPairId, SpecHash,
    };
    use mfm_manual_auth::{
        manual_authorization_proof_schema_id, ManualResolutionEvidenceRef,
        ManualResolutionPrefixAuthority, ManualResolutionProofAuthority,
        VerifiedManualResolutionForPrefix,
    };
    use mfm_spec::v1::{self as spec, CanonicalizerIdentity, HashedSpecEnvelope};
    use mfm_spec::SpecError;
    use mfm_store::v1::{
        self as store, ArtifactEvidenceRef as StoredArtifactEvidenceRef, KernelEventEnvelope,
        ProjectionSnapshot,
    };

    /// Result type for replay broker operations.
    pub type Result<T> = std::result::Result<T, ReplayError>;

    /// Typed replay validation error.
    #[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
    #[error("{}: {message}", .kind.code())]
    pub struct ReplayError {
        /// Stable replay error category.
        pub kind: ReplayErrorKind,
        /// Redaction-safe diagnostic.
        pub message: String,
    }

    impl ReplayError {
        /// Creates a replay error with a stable category.
        pub fn new(kind: ReplayErrorKind, message: impl Into<String>) -> Self {
            Self {
                kind,
                message: message.into(),
            }
        }

        /// Returns the stable machine-readable replay error code.
        pub const fn code(&self) -> &'static str {
            self.kind.code()
        }
    }

    impl From<SpecError> for ReplayError {
        fn from(error: SpecError) -> Self {
            Self::new(ReplayErrorKind::CertifiedSpec, error.to_string())
        }
    }

    impl From<store::StoreError> for ReplayError {
        fn from(error: store::StoreError) -> Self {
            Self::new(ReplayErrorKind::InvalidRunStream, error.to_string())
        }
    }

    impl From<mfm_runtime::RuntimeError> for ReplayError {
        fn from(error: mfm_runtime::RuntimeError) -> Self {
            Self::new(
                ReplayErrorKind::CertifiedEvidenceMismatch,
                error.to_string(),
            )
        }
    }

    /// Stable typed replay error category.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum ReplayErrorKind {
        /// The hash-only spec envelope is invalid.
        CertifiedSpec,
        /// No authoritative run-start event was present.
        RunAdmittedMissing,
        /// The run stream is not a valid store-owned typed stream.
        InvalidRunStream,
        /// The stream is bound to a different certified spec hash.
        SpecHashMismatch,
        /// The stream or authority carries a different canonicalizer identity.
        CanonicalizerMismatch,
        /// The run-start descriptor identities disagree with the certified spec.
        DescriptorIdentityMismatch,
        /// Runner executable identities disagree with replay authority.
        ExecutableIdentityMismatch,
        /// Adapter executable identities disagree with replay authority.
        AdapterExecutableMismatch,
        /// A requested node is not certified to use the requested capability.
        UnsupportedCapability,
        /// A requested node is not certified to use the requested adapter.
        UnsupportedAdapter,
        /// Replay attempted to request live capability access.
        LiveCapabilityRequest,
        /// A requested recorded fact was absent.
        FactMissing,
        /// A recorded fact did not match the replay request.
        FactMismatch,
        /// Required retained artifact evidence was absent.
        ArtifactMissing,
        /// Retained artifact evidence disagreed with typed event evidence.
        ArtifactMismatch,
        /// Required side-effect evidence was absent.
        SideEffectMissing,
        /// Recorded side-effect evidence did not match the replay request.
        SideEffectMismatch,
        /// A stream event references evidence outside the certified spec.
        CertifiedEvidenceMismatch,
        /// A side-effect replay verifier identity did not match.
        ReplayVerifierMismatch,
    }

    impl ReplayErrorKind {
        /// Returns the stable replay error code.
        pub const fn code(self) -> &'static str {
            match self {
                Self::CertifiedSpec => "MFM_REPLAY_CERTIFIED_SPEC_INVALID",
                Self::RunAdmittedMissing => "MFM_REPLAY_RUN_ADMITTED_MISSING",
                Self::InvalidRunStream => "MFM_REPLAY_STREAM_INVALID",
                Self::SpecHashMismatch => "MFM_REPLAY_SPEC_HASH_MISMATCH",
                Self::CanonicalizerMismatch => "MFM_REPLAY_CANONICALIZER_MISMATCH",
                Self::DescriptorIdentityMismatch => "MFM_REPLAY_DESCRIPTOR_IDENTITY_MISMATCH",
                Self::ExecutableIdentityMismatch => "MFM_REPLAY_EXECUTABLE_IDENTITY_MISMATCH",
                Self::AdapterExecutableMismatch => "MFM_REPLAY_ADAPTER_EXECUTABLE_MISMATCH",
                Self::UnsupportedCapability => "MFM_REPLAY_CAPABILITY_UNSUPPORTED",
                Self::UnsupportedAdapter => "MFM_REPLAY_ADAPTER_UNSUPPORTED",
                Self::LiveCapabilityRequest => "MFM_REPLAY_LIVE_CAPABILITY_REQUEST",
                Self::FactMissing => "MFM_REPLAY_FACT_MISSING",
                Self::FactMismatch => "MFM_REPLAY_FACT_MISMATCH",
                Self::ArtifactMissing => "MFM_REPLAY_ARTIFACT_MISSING",
                Self::ArtifactMismatch => "MFM_REPLAY_ARTIFACT_MISMATCH",
                Self::SideEffectMissing => "MFM_REPLAY_SIDE_EFFECT_MISSING",
                Self::SideEffectMismatch => "MFM_REPLAY_SIDE_EFFECT_MISMATCH",
                Self::CertifiedEvidenceMismatch => "MFM_REPLAY_CERTIFIED_EVIDENCE_MISMATCH",
                Self::ReplayVerifierMismatch => "MFM_REPLAY_VERIFIER_MISMATCH",
            }
        }
    }

    /// Sealed replay read authority minted from certified spec and verified run history.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ReplayReadAuthority {
        certified_spec: HashedSpecEnvelope,
        stream: Vec<KernelEventEnvelope>,
        canonicalizer_identity: CanonicalizerIdentity,
        runner_executables: Vec<events::ExecutableIdentity>,
        adapter_executables: Vec<events::ExecutableIdentity>,
        artifact_evidence: Vec<StoredArtifactEvidenceRef>,
        artifact_bytes: BTreeMap<ArtifactId, Vec<u8>>,
    }

    impl ReplayReadAuthority {
        /// Mints replay read authority from certifier-backed runtime authority and a shared
        /// verified run-history view.
        pub fn from_verified_run_history_view(
            runtime_spec: &mfm_runtime::CertifiedRuntimeSpec,
            verified_view: &mfm_runtime::VerifiedRunHistoryView,
        ) -> Result<Self> {
            if runtime_spec.spec_hash() != verified_view.spec_hash() {
                return Err(ReplayError::new(
                    ReplayErrorKind::SpecHashMismatch,
                    "verified history spec hash does not match certified runtime spec",
                ));
            }
            let run_admitted = verified_view.run_admitted();
            let artifact_evidence = verified_view
                .artifact_store()
                .artifacts()
                .map(|(_, artifact)| artifact.evidence().clone())
                .collect::<Vec<_>>();
            let artifact_bytes = verified_view
                .artifact_store()
                .artifacts()
                .map(|(_, artifact)| ReplayArtifactBytes {
                    artifact_id: artifact.evidence().artifact_id.clone(),
                    bytes: artifact.bytes().to_vec(),
                })
                .collect::<Vec<_>>();
            let artifacts = artifact_map(artifact_evidence.clone())?;
            let artifact_bytes = artifact_bytes_map(artifact_bytes)?;
            verify_replay_artifact_authority(verified_view, &artifacts)?;
            Ok(Self {
                certified_spec: runtime_spec.envelope().clone(),
                stream: verified_view.events().to_vec(),
                canonicalizer_identity: runtime_spec
                    .envelope()
                    .spec
                    .public_outputs
                    .renderer_descriptor
                    .canonicalizer_identity
                    .clone(),
                runner_executables: run_admitted.runner_executables.clone(),
                adapter_executables: run_admitted.adapter_executables.clone(),
                artifact_evidence,
                artifact_bytes,
            })
        }
    }

    /// Retained artifact bytes supplied to replay for proof-bearing artifacts.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ReplayArtifactBytes {
        /// Artifact id these bytes materialize.
        pub artifact_id: ArtifactId,
        /// Artifact content bytes.
        pub bytes: Vec<u8>,
    }

    /// Request for replaying a previously recorded read fact.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct FactReplayRequest {
        /// Node id that requested the fact.
        pub node_id: NodeId,
        /// Attempt id that requested the fact.
        pub attempt_id: AttemptId,
        /// Stable fact key.
        pub fact_key: events::FactKey,
        /// Capability kind expected by the replaying state.
        pub capability_kind: CapabilityKind,
        /// Capability version expected by the replaying state.
        pub capability_version: CapabilityVersion,
        /// Adapter kind expected by the replaying state.
        pub adapter_kind: AdapterKind,
        /// Adapter version expected by the replaying state.
        pub adapter_version: AdapterVersion,
        /// Request schema id expected by replay.
        pub request_schema_id: SchemaId,
        /// Canonical request hash expected by replay.
        pub request_hash: ContentDigest,
        /// Response schema id expected by replay.
        pub response_schema_id: SchemaId,
    }

    /// Replay evidence returned for a recorded read fact.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RecordedFactReplay {
        /// Recorded fact event payload.
        pub fact: events::FactRecorded,
        /// Retained artifact evidence for the fact response.
        pub artifact: StoredArtifactEvidenceRef,
    }

    /// Request for replaying recorded side-effect evidence.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SideEffectEvidenceReplayRequest {
        /// Stable side-effect pair id.
        pub pair_id: SideEffectPairId,
        /// Invocation epoch to replay.
        pub invocation_epoch: u32,
        /// Evidence schema id expected by replay.
        pub evidence_schema_id: SchemaId,
        /// Canonical evidence hash expected by replay.
        pub evidence_hash: ContentDigest,
        /// Replay verifier required for receipt and confirmation evidence.
        pub replay_verifier_id: Option<events::ReplayVerifierId>,
    }

    /// Broker-indexed side-effect replay frame for one intent and invocation epoch.
    ///
    /// The frame borrows recorded, replay-authorized event payloads from [`ReplayBroker`]. Domain
    /// verifiers still own cardinality, missing-evidence policy, and evidence interpretation.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct SideEffectReplayFrame<'a> {
        /// Intent persisted event payload.
        pub intent: &'a side_effect::IntentPersisted,
        /// Submission observed event payload, when present.
        pub submission: Option<&'a side_effect::SubmissionObserved>,
        /// Not-submitted proof payload, when present.
        pub not_submitted: Option<&'a side_effect::NotSubmittedProven>,
        /// Receipt observed event payload, when present.
        pub receipt: Option<&'a side_effect::ReceiptObserved>,
        /// Confirmation observed event payload, when present.
        pub confirmation: Option<&'a side_effect::ConfirmationObserved>,
        /// Ambiguity event payload, when present.
        pub ambiguity: Option<&'a side_effect::Ambiguous>,
    }

    impl SideEffectReplayFrame<'_> {
        /// Builds the replay request for this frame's submission evidence, when present.
        pub fn submission_request(&self) -> Option<SideEffectEvidenceReplayRequest> {
            let submission = self.submission?;
            Some(side_effect_evidence_replay_request(
                self.intent,
                submission.submission_schema_id.clone(),
                submission.submission_hash.clone(),
                None,
            ))
        }

        /// Builds the replay request for this frame's receipt evidence, when present.
        pub fn receipt_request(&self) -> Option<SideEffectEvidenceReplayRequest> {
            let receipt = self.receipt?;
            Some(side_effect_evidence_replay_request(
                self.intent,
                receipt.receipt_schema_id.clone(),
                receipt.receipt_hash.clone(),
                Some(receipt.replay_verifier_id.clone()),
            ))
        }

        /// Builds the replay request for this frame's not-submitted proof, when present.
        pub fn not_submitted_request(&self) -> Option<SideEffectEvidenceReplayRequest> {
            let proof = self.not_submitted?;
            Some(side_effect_evidence_replay_request(
                self.intent,
                proof.proof_schema_id.clone(),
                proof.proof_hash.clone(),
                None,
            ))
        }

        /// Builds the replay request for this frame's confirmation evidence, when present.
        pub fn confirmation_request(&self) -> Option<SideEffectEvidenceReplayRequest> {
            let confirmation = self.confirmation?;
            Some(side_effect_evidence_replay_request(
                self.intent,
                confirmation.confirmation_schema_id.clone(),
                confirmation.confirmation_hash.clone(),
                Some(confirmation.replay_verifier_id.clone()),
            ))
        }

        /// Builds the replay request for this frame's ambiguity evidence, when present.
        pub fn ambiguity_request(&self) -> Option<SideEffectEvidenceReplayRequest> {
            let ambiguity = self.ambiguity?;
            Some(side_effect_evidence_replay_request(
                self.intent,
                ambiguity.evidence_schema_id.clone(),
                ambiguity.evidence_hash.clone(),
                None,
            ))
        }
    }

    fn side_effect_evidence_replay_request(
        intent: &side_effect::IntentPersisted,
        evidence_schema_id: SchemaId,
        evidence_hash: ContentDigest,
        replay_verifier_id: Option<events::ReplayVerifierId>,
    ) -> SideEffectEvidenceReplayRequest {
        SideEffectEvidenceReplayRequest {
            pair_id: intent.pair_id.clone(),
            invocation_epoch: intent.invocation_epoch,
            evidence_schema_id,
            evidence_hash,
            replay_verifier_id,
        }
    }

    /// Replay evidence returned for an observed side-effect submission.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SubmissionReplayEvidence {
        /// Submission observed event payload.
        pub submission: side_effect::SubmissionObserved,
        /// Retained submission artifact evidence.
        pub artifact: StoredArtifactEvidenceRef,
    }

    /// Replay evidence returned for a side effect proven not submitted.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct NotSubmittedReplayEvidence {
        /// Not-submitted proof event payload.
        pub proof: side_effect::NotSubmittedProven,
        /// Retained not-submitted proof artifact evidence.
        pub artifact: StoredArtifactEvidenceRef,
    }

    /// Replay evidence returned for an observed side-effect receipt.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ReceiptReplayEvidence {
        /// Receipt observed event payload.
        pub receipt: side_effect::ReceiptObserved,
        /// Retained receipt artifact evidence.
        pub artifact: StoredArtifactEvidenceRef,
    }

    /// Replay evidence returned for an observed side-effect confirmation.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ConfirmationReplayEvidence {
        /// Confirmation observed event payload.
        pub confirmation: side_effect::ConfirmationObserved,
        /// Retained confirmation artifact evidence.
        pub artifact: StoredArtifactEvidenceRef,
        /// Retained confirmation artifact bytes.
        pub artifact_bytes: Vec<u8>,
    }

    /// Replay evidence returned for recorded side-effect ambiguity.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct AmbiguityReplayEvidence {
        /// Ambiguous event payload.
        pub ambiguity: side_effect::Ambiguous,
        /// Retained ambiguity artifact evidence.
        pub artifact: StoredArtifactEvidenceRef,
    }

    /// Intent evidence supplied to side-effect replay verifiers.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SideEffectIntentReplayEvidence {
        /// Intent persisted event payload.
        pub intent: side_effect::IntentPersisted,
        /// Retained intent artifact evidence.
        pub artifact: StoredArtifactEvidenceRef,
    }

    /// Evidence-only receipt verifier input.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SideEffectReceiptReplayInput {
        /// Intent evidence for the side effect being verified.
        pub intent: SideEffectIntentReplayEvidence,
        /// Submission evidence when submission was observed before receipt.
        pub submission: Option<SubmissionReplayEvidence>,
        /// Receipt evidence being verified.
        pub receipt: ReceiptReplayEvidence,
    }

    /// Evidence-only submission verifier input.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SideEffectSubmissionReplayInput {
        /// Intent evidence for the side effect being verified.
        pub intent: SideEffectIntentReplayEvidence,
        /// Submission evidence being verified.
        pub submission: SubmissionReplayEvidence,
    }

    /// Evidence-only confirmation verifier input.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SideEffectConfirmationReplayInput {
        /// Intent evidence for the side effect being verified.
        pub intent: SideEffectIntentReplayEvidence,
        /// Submission evidence when submission was observed before confirmation.
        pub submission: Option<SubmissionReplayEvidence>,
        /// Receipt evidence observed before confirmation.
        pub receipt: Option<ReceiptReplayEvidence>,
        /// Certified verification policy for the side effect.
        pub verification: spec::SideEffectVerificationSpec,
        /// Confirmation evidence being verified.
        pub confirmation: ConfirmationReplayEvidence,
    }

    /// Evidence-only side-effect replay verifier contract.
    ///
    /// The broker passes only recorded typed events and retained artifact evidence into this trait.
    /// Verifier implementations do not receive transports, SDK clients, capability handles, or any
    /// generic IO provider from the replay crate.
    pub trait SideEffectReplayVerifier {
        /// Replay verifier identity recorded in receipt and confirmation events.
        fn verifier_id(&self) -> &events::ReplayVerifierId;

        /// Verifies submission evidence without live IO.
        fn verify_submission(&self, input: &SideEffectSubmissionReplayInput) -> Result<()>;

        /// Verifies receipt evidence without live IO.
        fn verify_receipt(&self, input: &SideEffectReceiptReplayInput) -> Result<()>;

        /// Verifies confirmation evidence without live IO.
        fn verify_confirmation(&self, input: &SideEffectConfirmationReplayInput) -> Result<()>;
    }

    /// Request for replaying a retained artifact directly.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ArtifactReplayRequest {
        /// Artifact id requested by replay.
        pub artifact_id: ArtifactId,
        /// Expected artifact role.
        pub role: ArtifactRole,
        /// Expected content digest.
        pub digest: ContentDigest,
        /// Expected schema id, when schema-bearing.
        pub schema_id: Option<SchemaId>,
        /// Expected semantic type id, when value-bearing.
        pub semantic_type_id: Option<SemanticTypeId>,
        /// Expected producer node id, when node-produced.
        pub producer_node_id: Option<NodeId>,
        /// Expected producer seed id, when seed-produced.
        pub producer_seed_id: Option<SeedId>,
    }

    #[derive(Debug, Clone, Copy)]
    struct ArtifactEvidenceExpectation<'a> {
        artifact_id: &'a ArtifactId,
        digest: &'a ContentDigest,
        schema_id: Option<&'a SchemaId>,
        semantic_type_id: Option<&'a SemanticTypeId>,
        role: ArtifactRole,
        producer_node_id: Option<&'a NodeId>,
        producer_seed_id: Option<&'a SeedId>,
    }

    type FactKey = (NodeId, AttemptId, events::FactKey);
    type ReplayArtifactAuthorityKey = (ArtifactId, ContentDigest);
    type SideEffectKey = (SideEffectPairId, u32);

    trait SideEffectReplayArtifact {
        fn artifact_id(&self) -> &ArtifactId;
        fn evidence_hash(&self) -> &ContentDigest;
        fn evidence_schema_id(&self) -> &SchemaId;
        fn artifact_role(&self) -> ArtifactRole;
        fn producer_node_id(&self) -> &NodeId;
        fn mismatch_message(&self) -> &'static str;
        fn replay_verifier_id(&self) -> Option<&events::ReplayVerifierId> {
            None
        }
    }

    impl SideEffectReplayArtifact for side_effect::SubmissionObserved {
        fn artifact_id(&self) -> &ArtifactId {
            &self.submission_artifact_id
        }

        fn evidence_hash(&self) -> &ContentDigest {
            &self.submission_hash
        }

        fn evidence_schema_id(&self) -> &SchemaId {
            &self.submission_schema_id
        }

        fn artifact_role(&self) -> ArtifactRole {
            ArtifactRole::Submission
        }

        fn producer_node_id(&self) -> &NodeId {
            &self.node_id
        }

        fn mismatch_message(&self) -> &'static str {
            "submission evidence mismatch"
        }
    }

    impl SideEffectReplayArtifact for side_effect::NotSubmittedProven {
        fn artifact_id(&self) -> &ArtifactId {
            &self.proof_artifact_id
        }

        fn evidence_hash(&self) -> &ContentDigest {
            &self.proof_hash
        }

        fn evidence_schema_id(&self) -> &SchemaId {
            &self.proof_schema_id
        }

        fn artifact_role(&self) -> ArtifactRole {
            ArtifactRole::NotSubmittedProof
        }

        fn producer_node_id(&self) -> &NodeId {
            &self.node_id
        }

        fn mismatch_message(&self) -> &'static str {
            "not-submitted proof evidence mismatch"
        }
    }

    impl SideEffectReplayArtifact for side_effect::ReceiptObserved {
        fn artifact_id(&self) -> &ArtifactId {
            &self.receipt_artifact_id
        }

        fn evidence_hash(&self) -> &ContentDigest {
            &self.receipt_hash
        }

        fn evidence_schema_id(&self) -> &SchemaId {
            &self.receipt_schema_id
        }

        fn artifact_role(&self) -> ArtifactRole {
            ArtifactRole::Receipt
        }

        fn producer_node_id(&self) -> &NodeId {
            &self.node_id
        }

        fn mismatch_message(&self) -> &'static str {
            "receipt evidence mismatch"
        }

        fn replay_verifier_id(&self) -> Option<&events::ReplayVerifierId> {
            Some(&self.replay_verifier_id)
        }
    }

    impl SideEffectReplayArtifact for side_effect::ConfirmationObserved {
        fn artifact_id(&self) -> &ArtifactId {
            &self.confirmation_artifact_id
        }

        fn evidence_hash(&self) -> &ContentDigest {
            &self.confirmation_hash
        }

        fn evidence_schema_id(&self) -> &SchemaId {
            &self.confirmation_schema_id
        }

        fn artifact_role(&self) -> ArtifactRole {
            ArtifactRole::Confirmation
        }

        fn producer_node_id(&self) -> &NodeId {
            &self.node_id
        }

        fn mismatch_message(&self) -> &'static str {
            "confirmation evidence mismatch"
        }

        fn replay_verifier_id(&self) -> Option<&events::ReplayVerifierId> {
            Some(&self.replay_verifier_id)
        }
    }

    impl SideEffectReplayArtifact for side_effect::Ambiguous {
        fn artifact_id(&self) -> &ArtifactId {
            &self.evidence_artifact_id
        }

        fn evidence_hash(&self) -> &ContentDigest {
            &self.evidence_hash
        }

        fn evidence_schema_id(&self) -> &SchemaId {
            &self.evidence_schema_id
        }

        fn artifact_role(&self) -> ArtifactRole {
            ArtifactRole::AmbiguityEvidence
        }

        fn producer_node_id(&self) -> &NodeId {
            &self.node_id
        }

        fn mismatch_message(&self) -> &'static str {
            "ambiguity evidence mismatch"
        }
    }

    /// Evidence-only broker for certified typed replay.
    #[derive(Debug, Clone)]
    pub struct ReplayBroker {
        certified_spec: HashedSpecEnvelope,
        stream: Vec<KernelEventEnvelope>,
        run_id: events::RunAdmitted,
        projection: ProjectionSnapshot,
        retained_artifacts: BTreeMap<ReplayArtifactAuthorityKey, StoredArtifactEvidenceRef>,
        artifact_bytes: BTreeMap<ArtifactId, Vec<u8>>,
        artifacts: BTreeMap<ReplayArtifactAuthorityKey, StoredArtifactEvidenceRef>,
        facts: BTreeMap<FactKey, events::FactRecorded>,
        intents: BTreeMap<SideEffectPairId, side_effect::IntentPersisted>,
        submissions: BTreeMap<SideEffectKey, side_effect::SubmissionObserved>,
        not_submitted: BTreeMap<SideEffectKey, side_effect::NotSubmittedProven>,
        receipts: BTreeMap<SideEffectKey, side_effect::ReceiptObserved>,
        confirmations: BTreeMap<SideEffectKey, side_effect::ConfirmationObserved>,
        ambiguities: BTreeMap<SideEffectKey, side_effect::Ambiguous>,
        manual_resolutions: BTreeMap<RunId, VerifiedManualResolutionForPrefix>,
    }

    impl ReplayBroker {
        /// Builds a replay broker from sealed replay read authority.
        pub fn from_read_authority(authority: ReplayReadAuthority) -> Result<Self> {
            Self::from_validated_parts(authority)
        }

        fn from_validated_parts(authority: ReplayReadAuthority) -> Result<Self> {
            let certified_spec = authority.certified_spec.clone();
            let stream = authority.stream.clone();
            certified_spec.verify_hash()?;
            ProjectionSnapshot::validate_run_stream(&stream)?;
            let projection = ProjectionSnapshot::rebuild_from_run_stream(&stream)?;
            let retained_artifacts = artifact_map(authority.artifact_evidence.clone())?;
            let run_admitted = run_admitted_payload(&stream)?;

            if run_admitted.spec_hash != certified_spec.spec_hash {
                return Err(ReplayError::new(
                    ReplayErrorKind::SpecHashMismatch,
                    "run-start spec hash does not match certified spec",
                ));
            }
            if stream
                .iter()
                .any(|event| event.spec_hash() != &certified_spec.spec_hash)
            {
                return Err(ReplayError::new(
                    ReplayErrorKind::SpecHashMismatch,
                    "run stream contains payloads for a different certified spec",
                ));
            }
            verify_run_start_contract(
                &certified_spec,
                &run_admitted,
                &authority,
                &retained_artifacts,
            )?;
            verify_remediation_ledger_links(&certified_spec, &projection)?;
            verify_resource_lane_release_adjacency(&certified_spec, &stream)?;

            let mut broker = Self {
                certified_spec,
                stream: stream.clone(),
                run_id: run_admitted,
                projection,
                retained_artifacts,
                artifact_bytes: authority.artifact_bytes.clone(),
                artifacts: BTreeMap::new(),
                facts: BTreeMap::new(),
                intents: BTreeMap::new(),
                submissions: BTreeMap::new(),
                not_submitted: BTreeMap::new(),
                receipts: BTreeMap::new(),
                confirmations: BTreeMap::new(),
                ambiguities: BTreeMap::new(),
                manual_resolutions: BTreeMap::new(),
            };
            broker.authorize_certified_spec_artifacts()?;
            broker.index_stream(&stream)?;
            broker.verify_terminal_outcome_agreement()?;
            broker.reject_unauthorized_artifact_evidence()?;
            Ok(broker)
        }

        /// Returns the certified spec used as replay authority.
        pub fn certified_spec(&self) -> &HashedSpecEnvelope {
            &self.certified_spec
        }

        /// Returns the run-start payload bound to this replay broker.
        pub fn run_admitted(&self) -> &events::RunAdmitted {
            &self.run_id
        }

        /// Returns the broker-owned verified history events used for replay evidence.
        pub fn events(&self) -> &[KernelEventEnvelope] {
            &self.stream
        }

        /// Returns the projection rebuilt from the authoritative run stream.
        pub fn projection_snapshot(&self) -> &ProjectionSnapshot {
            &self.projection
        }

        /// Returns retained artifact evidence for a direct artifact replay request.
        pub fn artifact(
            &self,
            request: &ArtifactReplayRequest,
        ) -> Result<StoredArtifactEvidenceRef> {
            self.verify_artifact(ArtifactEvidenceExpectation {
                artifact_id: &request.artifact_id,
                digest: &request.digest,
                schema_id: request.schema_id.as_ref(),
                semantic_type_id: request.semantic_type_id.as_ref(),
                role: request.role,
                producer_node_id: request.producer_node_id.as_ref(),
                producer_seed_id: request.producer_seed_id.as_ref(),
            })
        }

        /// Returns a recorded fact from replay evidence only.
        pub fn recorded_fact(&self, request: &FactReplayRequest) -> Result<RecordedFactReplay> {
            self.verify_node_capability(
                &request.node_id,
                &request.capability_kind,
                &request.capability_version,
            )?;
            self.verify_node_adapter(
                &request.node_id,
                &request.adapter_kind,
                &request.adapter_version,
            )?;
            let fact = self
                .facts
                .get(&(
                    request.node_id.clone(),
                    request.attempt_id.clone(),
                    request.fact_key.clone(),
                ))
                .ok_or_else(|| {
                    ReplayError::new(
                        ReplayErrorKind::FactMissing,
                        format!("missing recorded fact {}", request.fact_key),
                    )
                })?;
            if fact.capability_kind != request.capability_kind
                || fact.capability_version != request.capability_version
                || fact.adapter_kind != request.adapter_kind
                || fact.adapter_version != request.adapter_version
                || fact.request_schema_id != request.request_schema_id
                || fact.request_hash != request.request_hash
                || fact.response_schema_id != request.response_schema_id
            {
                return Err(ReplayError::new(
                    ReplayErrorKind::FactMismatch,
                    format!(
                        "recorded fact {} does not match replay request",
                        request.fact_key
                    ),
                ));
            }

            Ok(RecordedFactReplay {
                fact: fact.clone(),
                artifact: self.verify_artifact(ArtifactEvidenceExpectation {
                    artifact_id: &fact.artifact_id,
                    digest: &fact.response_hash,
                    schema_id: Some(&fact.response_schema_id),
                    semantic_type_id: None,
                    role: ArtifactRole::FactResponse,
                    producer_node_id: Some(&fact.node_id),
                    producer_seed_id: None,
                })?,
            })
        }

        /// Returns side-effect replay frames whose intent matches a domain predicate.
        ///
        /// This exposes broker-validated recorded evidence only. Domain verifiers remain
        /// responsible for deciding how many frames are valid and which phases are required.
        pub fn side_effect_replay_frames_matching<F>(
            &self,
            mut matches_intent: F,
        ) -> Result<Vec<SideEffectReplayFrame<'_>>>
        where
            F: FnMut(&side_effect::IntentPersisted) -> Result<bool>,
        {
            let mut frames = Vec::new();
            for intent in self.intents.values() {
                if !matches_intent(intent)? {
                    continue;
                }
                let key = (intent.pair_id.clone(), intent.invocation_epoch);
                frames.push(SideEffectReplayFrame {
                    intent,
                    submission: self.submissions.get(&key),
                    not_submitted: self.not_submitted.get(&key),
                    receipt: self.receipts.get(&key),
                    confirmation: self.confirmations.get(&key),
                    ambiguity: self.ambiguities.get(&key),
                });
            }
            Ok(frames)
        }

        /// Returns observed side-effect submission evidence from replay records only.
        pub fn side_effect_submission(
            &self,
            request: &SideEffectEvidenceReplayRequest,
        ) -> Result<SubmissionReplayEvidence> {
            self.verify_side_effect_intent(request)?;
            let submission =
                self.required_side_effect_record(&self.submissions, request, "submission")?;
            Ok(SubmissionReplayEvidence {
                submission: submission.clone(),
                artifact: self.verify_requested_side_effect_artifact(request, submission)?,
            })
        }

        /// Returns side-effect not-submitted proof evidence from replay records only.
        pub fn side_effect_not_submitted(
            &self,
            request: &SideEffectEvidenceReplayRequest,
        ) -> Result<NotSubmittedReplayEvidence> {
            self.verify_side_effect_intent(request)?;
            let proof = self.required_side_effect_record(
                &self.not_submitted,
                request,
                "not-submitted proof",
            )?;
            Ok(NotSubmittedReplayEvidence {
                proof: proof.clone(),
                artifact: self.verify_requested_side_effect_artifact(request, proof)?,
            })
        }

        /// Verifies submission evidence with an evidence-only replay verifier contract.
        pub fn verify_side_effect_submission<V>(
            &self,
            request: &SideEffectEvidenceReplayRequest,
            verifier: &V,
        ) -> Result<SubmissionReplayEvidence>
        where
            V: SideEffectReplayVerifier + ?Sized,
        {
            let submission = self.side_effect_submission(request)?;
            let input = SideEffectSubmissionReplayInput {
                intent: self.side_effect_intent_evidence(request)?,
                submission: submission.clone(),
            };
            verifier.verify_submission(&input)?;
            Ok(submission)
        }

        /// Returns observed side-effect receipt evidence from replay records only.
        pub fn side_effect_receipt(
            &self,
            request: &SideEffectEvidenceReplayRequest,
        ) -> Result<ReceiptReplayEvidence> {
            self.verify_side_effect_intent(request)?;
            let receipt = self.required_side_effect_record(&self.receipts, request, "receipt")?;
            Ok(ReceiptReplayEvidence {
                receipt: receipt.clone(),
                artifact: self.verify_requested_side_effect_artifact(request, receipt)?,
            })
        }

        /// Returns observed side-effect confirmation evidence from replay records only.
        pub fn side_effect_confirmation(
            &self,
            request: &SideEffectEvidenceReplayRequest,
        ) -> Result<ConfirmationReplayEvidence> {
            self.verify_side_effect_intent(request)?;
            let confirmation =
                self.required_side_effect_record(&self.confirmations, request, "confirmation")?;
            Ok(ConfirmationReplayEvidence {
                confirmation: confirmation.clone(),
                artifact: self.verify_requested_side_effect_artifact(request, confirmation)?,
                artifact_bytes: self
                    .artifact_bytes(&confirmation.confirmation_artifact_id)?
                    .to_vec(),
            })
        }

        /// Returns side-effect ambiguity evidence from replay records only.
        pub fn side_effect_ambiguity(
            &self,
            request: &SideEffectEvidenceReplayRequest,
        ) -> Result<AmbiguityReplayEvidence> {
            self.verify_side_effect_intent(request)?;
            let ambiguity =
                self.required_side_effect_record(&self.ambiguities, request, "ambiguity")?;
            Ok(AmbiguityReplayEvidence {
                ambiguity: ambiguity.clone(),
                artifact: self.verify_requested_side_effect_artifact(request, ambiguity)?,
            })
        }

        /// Verifies receipt evidence with an evidence-only replay verifier contract.
        pub fn verify_side_effect_receipt<V>(
            &self,
            request: &SideEffectEvidenceReplayRequest,
            verifier: &V,
        ) -> Result<ReceiptReplayEvidence>
        where
            V: SideEffectReplayVerifier + ?Sized,
        {
            let mut verifier_request = request.clone();
            verifier_request.replay_verifier_id = Some(verifier.verifier_id().clone());
            let receipt = self.side_effect_receipt(&verifier_request)?;
            let input = SideEffectReceiptReplayInput {
                intent: self.side_effect_intent_evidence(request)?,
                submission: self.side_effect_submission_for(request)?,
                receipt: receipt.clone(),
            };
            verifier.verify_receipt(&input)?;
            Ok(receipt)
        }

        /// Verifies confirmation evidence with an evidence-only replay verifier contract.
        pub fn verify_side_effect_confirmation<V>(
            &self,
            request: &SideEffectEvidenceReplayRequest,
            verifier: &V,
        ) -> Result<ConfirmationReplayEvidence>
        where
            V: SideEffectReplayVerifier + ?Sized,
        {
            let mut verifier_request = request.clone();
            verifier_request.replay_verifier_id = Some(verifier.verifier_id().clone());
            let confirmation = self.side_effect_confirmation(&verifier_request)?;
            let input = SideEffectConfirmationReplayInput {
                intent: self.side_effect_intent_evidence(request)?,
                submission: self.side_effect_submission_for(request)?,
                receipt: self.side_effect_receipt_for(request)?,
                verification: self.side_effect_verification_for_pair(&request.pair_id)?,
                confirmation: confirmation.clone(),
            };
            verifier.verify_confirmation(&input)?;
            Ok(confirmation)
        }

        /// Rejects any attempt to obtain live capability access during replay.
        pub fn reject_live_capability_request(&self) -> Result<()> {
            Err(ReplayError::new(
                ReplayErrorKind::LiveCapabilityRequest,
                "typed replay brokers do not construct live capabilities",
            ))
        }

        fn authorize_certified_spec_artifacts(&mut self) -> Result<()> {
            let config_refs = self.certified_spec.spec.config_refs.clone();
            for config in config_refs {
                self.authorize_artifact(ArtifactEvidenceExpectation {
                    artifact_id: &config.artifact_id,
                    digest: &config.digest,
                    schema_id: Some(&config.schema_id),
                    semantic_type_id: None,
                    role: ArtifactRole::TypedConfig,
                    producer_node_id: None,
                    producer_seed_id: None,
                })?;
            }
            Ok(())
        }

        fn index_stream(&mut self, stream: &[KernelEventEnvelope]) -> Result<()> {
            let mut resource_keys = BTreeMap::new();
            for envelope in stream {
                match envelope.payload() {
                    KernelEventPayload::RunAdmitted(payload) => {
                        for seed in &payload.seed_cells {
                            self.verify_seed_against_spec(seed)?;
                        }
                        self.authorize_event_artifacts(envelope.payload())?;
                    }
                    KernelEventPayload::StateAttemptStarted(payload) => {
                        self.verify_state_attempt_started_against_spec(payload)?;
                    }
                    KernelEventPayload::FactRecorded(payload) => {
                        self.verify_fact_against_spec(payload)?;
                        self.authorize_event_artifacts(envelope.payload())?;
                        insert_unique(
                            &mut self.facts,
                            (
                                payload.node_id.clone(),
                                payload.attempt_id.clone(),
                                payload.fact_key.clone(),
                            ),
                            payload.clone(),
                            ReplayErrorKind::InvalidRunStream,
                            "duplicate fact replay event",
                        )?;
                    }
                    KernelEventPayload::ArtifactReferenced(payload) => {
                        if let Some(node_id) = &payload.node_id {
                            self.node(node_id)?;
                        }
                        self.authorize_event_artifacts(envelope.payload())?;
                    }
                    KernelEventPayload::SideEffectIntentPersisted(payload) => {
                        self.verify_side_effect_intent_against_spec(payload)?;
                        self.authorize_event_artifacts(envelope.payload())?;
                        insert_unique(
                            &mut self.intents,
                            payload.pair_id.clone(),
                            payload.clone(),
                            ReplayErrorKind::InvalidRunStream,
                            "duplicate side-effect intent replay event",
                        )?;
                    }
                    KernelEventPayload::SideEffectClaimed(payload) => {
                        self.verify_side_effect_event_against_intent(
                            &payload.pair_id,
                            &payload.ledger_key,
                            payload.invocation_epoch,
                            payload.pair_role,
                            &payload.node_id,
                            &payload.attempt_id,
                        )?;
                    }
                    KernelEventPayload::SideEffectClaimTakenOver(payload) => {
                        self.verify_side_effect_event_against_intent(
                            &payload.pair_id,
                            &payload.ledger_key,
                            payload.invocation_epoch,
                            payload.pair_role,
                            &payload.node_id,
                            &payload.attempt_id,
                        )?;
                    }
                    KernelEventPayload::ResourceLaneClaimed(payload) => {
                        self.verify_resource_lane_claim(payload, &mut resource_keys)?;
                    }
                    KernelEventPayload::ResourceLaneReleased(payload) => {
                        self.verify_resource_lane_release_against_intent(
                            &payload.pair_id,
                            &payload.ledger_key,
                            payload.invocation_epoch,
                            payload.pair_role,
                            payload.release_authority,
                        )?;
                    }
                    KernelEventPayload::ResourceLaneClaimIntent(_)
                    | KernelEventPayload::ResourceLaneReleaseIntent(_) => {
                        return Err(ReplayError::new(
                            ReplayErrorKind::InvalidRunStream,
                            "replay stream contains unmaterialized resource-lane intent",
                        ));
                    }
                    KernelEventPayload::SideEffectInvocationStarted(payload) => {
                        self.verify_side_effect_event_against_intent(
                            &payload.pair_id,
                            &payload.ledger_key,
                            payload.invocation_epoch,
                            payload.pair_role,
                            &payload.node_id,
                            &payload.attempt_id,
                        )?;
                    }
                    KernelEventPayload::SideEffectSubmissionObserved(payload) => {
                        self.verify_side_effect_event_against_intent(
                            &payload.pair_id,
                            &payload.ledger_key,
                            payload.invocation_epoch,
                            payload.pair_role,
                            &payload.node_id,
                            &payload.attempt_id,
                        )?;
                        self.authorize_event_artifacts(envelope.payload())?;
                        insert_unique(
                            &mut self.submissions,
                            (payload.pair_id.clone(), payload.invocation_epoch),
                            payload.clone(),
                            ReplayErrorKind::InvalidRunStream,
                            "duplicate side-effect submission replay event",
                        )?;
                    }
                    KernelEventPayload::SideEffectReceiptObserved(payload) => {
                        self.verify_side_effect_event_against_intent(
                            &payload.pair_id,
                            &payload.ledger_key,
                            payload.invocation_epoch,
                            payload.pair_role,
                            &payload.node_id,
                            &payload.attempt_id,
                        )?;
                        self.verify_resource_touched_set(
                            &payload.pair_id,
                            payload.pair_role,
                            &payload.node_id,
                            payload.resource_touched_set.as_ref(),
                        )?;
                        self.authorize_event_artifacts(envelope.payload())?;
                        insert_unique(
                            &mut self.receipts,
                            (payload.pair_id.clone(), payload.invocation_epoch),
                            payload.clone(),
                            ReplayErrorKind::InvalidRunStream,
                            "duplicate side-effect receipt replay event",
                        )?;
                    }
                    KernelEventPayload::SideEffectConfirmationObserved(payload) => {
                        self.verify_side_effect_event_against_intent(
                            &payload.pair_id,
                            &payload.ledger_key,
                            payload.invocation_epoch,
                            payload.pair_role,
                            &payload.node_id,
                            &payload.attempt_id,
                        )?;
                        self.verify_resource_touched_set(
                            &payload.pair_id,
                            payload.pair_role,
                            &payload.node_id,
                            payload.resource_touched_set.as_ref(),
                        )?;
                        self.authorize_event_artifacts(envelope.payload())?;
                        insert_unique(
                            &mut self.confirmations,
                            (payload.pair_id.clone(), payload.invocation_epoch),
                            payload.clone(),
                            ReplayErrorKind::InvalidRunStream,
                            "duplicate side-effect confirmation replay event",
                        )?;
                    }
                    KernelEventPayload::SideEffectInvocationPrepared(payload) => {
                        self.verify_side_effect_event_against_intent(
                            &payload.pair_id,
                            &payload.ledger_key,
                            payload.invocation_epoch,
                            payload.pair_role,
                            &payload.node_id,
                            &payload.attempt_id,
                        )?;
                        self.verify_invocation_prepared_resource_key(payload, &resource_keys)?;
                        self.authorize_event_artifacts(envelope.payload())?;
                    }
                    KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
                        self.verify_side_effect_event_against_intent(
                            &payload.pair_id,
                            &payload.ledger_key,
                            payload.invocation_epoch,
                            payload.pair_role,
                            &payload.node_id,
                            &payload.attempt_id,
                        )?;
                        self.authorize_event_artifacts(envelope.payload())?;
                        insert_unique(
                            &mut self.not_submitted,
                            (payload.pair_id.clone(), payload.invocation_epoch),
                            payload.clone(),
                            ReplayErrorKind::InvalidRunStream,
                            "duplicate side-effect not-submitted replay event",
                        )?;
                    }
                    KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
                        self.verify_side_effect_event_against_intent(
                            &payload.pair_id,
                            &payload.ledger_key,
                            payload.invocation_epoch,
                            payload.pair_role,
                            &payload.node_id,
                            &payload.attempt_id,
                        )?;
                        self.authorize_event_artifacts(envelope.payload())?;
                    }
                    KernelEventPayload::SideEffectAmbiguous(payload) => {
                        self.verify_side_effect_ambiguity_against_intent(
                            &payload.pair_id,
                            &payload.ledger_key,
                            payload.invocation_epoch,
                            payload.pair_role,
                            &payload.node_id,
                            &payload.attempt_id,
                        )?;
                        self.authorize_event_artifacts(envelope.payload())?;
                        insert_unique(
                            &mut self.ambiguities,
                            (payload.pair_id.clone(), payload.invocation_epoch),
                            payload.clone(),
                            ReplayErrorKind::InvalidRunStream,
                            "duplicate side-effect ambiguity replay event",
                        )?;
                    }
                    KernelEventPayload::CellProduced(payload) => {
                        self.verify_cell_produced_against_spec(payload)?;
                        self.authorize_event_artifacts(envelope.payload())?;
                    }
                    KernelEventPayload::CellSkipped(payload) => {
                        self.verify_cell_skipped_against_spec(payload)?;
                    }
                    KernelEventPayload::PublicOutputProduced(payload) => {
                        self.verify_public_output_against_spec(payload)?;
                        self.authorize_event_artifacts(envelope.payload())?;
                    }
                    KernelEventPayload::PublicOutputRenderFailed(payload) => {
                        self.verify_public_output_render_failure_against_spec(payload)?;
                        self.authorize_event_artifacts(envelope.payload())?;
                    }
                    KernelEventPayload::StateAttemptFailed(payload) => {
                        self.node(&payload.node_id)?;
                        self.authorize_event_artifacts(envelope.payload())?;
                    }
                    KernelEventPayload::StateAttemptInterrupted(payload) => {
                        self.node(&payload.node_id)?;
                    }
                    KernelEventPayload::StateAttemptCompleted(payload) => {
                        self.verify_state_attempt_completed_against_spec(payload)?;
                    }
                    KernelEventPayload::RunCompleted(payload) => match &payload.outcome {
                        events::RunCompletionOutcome::Completed(evidence) => {
                            self.verify_completed_run_public_output(evidence)?;
                        }
                        events::RunCompletionOutcome::Compensated
                        | events::RunCompletionOutcome::ManuallyResolved
                        | events::RunCompletionOutcome::FailedWithoutAcdcClaim => {}
                    },
                    KernelEventPayload::ManualResolutionRecorded(payload) => {
                        let verified =
                            self.verify_manual_resolution_against_spec(envelope, payload)?;
                        insert_unique(
                            &mut self.manual_resolutions,
                            payload.run_id.clone(),
                            verified,
                            ReplayErrorKind::InvalidRunStream,
                            "duplicate manual resolution replay event",
                        )?;
                        self.authorize_event_artifacts(envelope.payload())?;
                    }
                    KernelEventPayload::RetentionManifestProjected(_) => {
                        self.authorize_event_artifacts(envelope.payload())?;
                    }
                    KernelEventPayload::RetentionRefsAppended(_) => {
                        self.authorize_event_artifacts(envelope.payload())?;
                    }
                    KernelEventPayload::SideEffectFailed(payload) => {
                        self.verify_side_effect_event_against_intent(
                            &payload.pair_id,
                            &payload.ledger_key,
                            payload.invocation_epoch,
                            payload.pair_role,
                            &payload.node_id,
                            &payload.attempt_id,
                        )?;
                    }
                }
            }
            Ok(())
        }

        fn side_effect_intent_evidence(
            &self,
            request: &SideEffectEvidenceReplayRequest,
        ) -> Result<SideEffectIntentReplayEvidence> {
            self.verify_side_effect_intent(request)?;
            let intent = self.intents.get(&request.pair_id).ok_or_else(|| {
                ReplayError::new(
                    ReplayErrorKind::SideEffectMissing,
                    format!("missing side-effect intent {}", request.pair_id),
                )
            })?;
            Ok(SideEffectIntentReplayEvidence {
                intent: intent.clone(),
                artifact: self.verify_artifact(ArtifactEvidenceExpectation {
                    artifact_id: &intent.intent_artifact_id,
                    digest: &intent.intent_hash,
                    schema_id: Some(&intent.intent_schema_id),
                    semantic_type_id: None,
                    role: ArtifactRole::SideEffectIntent,
                    producer_node_id: Some(&intent.node_id),
                    producer_seed_id: None,
                })?,
            })
        }

        fn required_side_effect_record<'a, T>(
            &'a self,
            records: &'a BTreeMap<SideEffectKey, T>,
            request: &SideEffectEvidenceReplayRequest,
            label: &'static str,
        ) -> Result<&'a T> {
            records
                .get(&(request.pair_id.clone(), request.invocation_epoch))
                .ok_or_else(|| {
                    ReplayError::new(
                        ReplayErrorKind::SideEffectMissing,
                        format!("missing side-effect {label} {}", request.pair_id),
                    )
                })
        }

        fn optional_side_effect_record<'a, T>(
            &'a self,
            records: &'a BTreeMap<SideEffectKey, T>,
            request: &SideEffectEvidenceReplayRequest,
        ) -> Option<&'a T> {
            records.get(&(request.pair_id.clone(), request.invocation_epoch))
        }

        fn verify_requested_side_effect_artifact<T>(
            &self,
            request: &SideEffectEvidenceReplayRequest,
            evidence: &T,
        ) -> Result<StoredArtifactEvidenceRef>
        where
            T: SideEffectReplayArtifact,
        {
            if let Some(recorded) = evidence.replay_verifier_id() {
                verify_replay_verifier(request.replay_verifier_id.as_ref(), recorded)?;
            }
            if evidence.evidence_schema_id() != &request.evidence_schema_id
                || evidence.evidence_hash() != &request.evidence_hash
            {
                return Err(side_effect_mismatch(evidence.mismatch_message()));
            }
            self.verify_side_effect_artifact(evidence)
        }

        fn verify_side_effect_artifact<T>(&self, evidence: &T) -> Result<StoredArtifactEvidenceRef>
        where
            T: SideEffectReplayArtifact,
        {
            self.verify_artifact(ArtifactEvidenceExpectation {
                artifact_id: evidence.artifact_id(),
                digest: evidence.evidence_hash(),
                schema_id: Some(evidence.evidence_schema_id()),
                semantic_type_id: None,
                role: evidence.artifact_role(),
                producer_node_id: Some(evidence.producer_node_id()),
                producer_seed_id: None,
            })
        }

        fn side_effect_submission_for(
            &self,
            request: &SideEffectEvidenceReplayRequest,
        ) -> Result<Option<SubmissionReplayEvidence>> {
            let Some(submission) = self.optional_side_effect_record(&self.submissions, request)
            else {
                return Ok(None);
            };
            Ok(Some(SubmissionReplayEvidence {
                submission: submission.clone(),
                artifact: self.verify_side_effect_artifact(submission)?,
            }))
        }

        fn side_effect_receipt_for(
            &self,
            request: &SideEffectEvidenceReplayRequest,
        ) -> Result<Option<ReceiptReplayEvidence>> {
            let Some(receipt) = self.optional_side_effect_record(&self.receipts, request) else {
                return Ok(None);
            };
            Ok(Some(ReceiptReplayEvidence {
                receipt: receipt.clone(),
                artifact: self.verify_side_effect_artifact(receipt)?,
            }))
        }

        fn side_effect_verification_for_pair(
            &self,
            pair_id: &SideEffectPairId,
        ) -> Result<spec::SideEffectVerificationSpec> {
            let pair = self
                .certified_spec
                .spec
                .side_effect_verify_pair_for_pair_id(pair_id)
                .map_err(certified_spec_error)?;
            Ok(pair.submit_contract.verification.clone())
        }

        fn verify_side_effect_intent(
            &self,
            request: &SideEffectEvidenceReplayRequest,
        ) -> Result<()> {
            let intent = self.intents.get(&request.pair_id).ok_or_else(|| {
                ReplayError::new(
                    ReplayErrorKind::SideEffectMissing,
                    format!("missing side-effect intent {}", request.pair_id),
                )
            })?;
            if intent.invocation_epoch != request.invocation_epoch {
                return Err(side_effect_mismatch(
                    "side-effect intent does not match replay request",
                ));
            }
            Ok(())
        }

        fn verify_node_capability(
            &self,
            node_id: &NodeId,
            capability_kind: &CapabilityKind,
            capability_version: &CapabilityVersion,
        ) -> Result<()> {
            let node = self.node(node_id)?;
            if capability_set_contains(
                &node.capability_bindings,
                capability_kind,
                capability_version,
            ) {
                Ok(())
            } else {
                Err(ReplayError::new(
                    ReplayErrorKind::UnsupportedCapability,
                    format!("node {node_id} is not certified for capability {capability_kind}"),
                ))
            }
        }

        fn verify_node_adapter(
            &self,
            node_id: &NodeId,
            adapter_kind: &AdapterKind,
            adapter_version: &AdapterVersion,
        ) -> Result<()> {
            let node = self.node(node_id)?;
            if node.adapter_bindings.iter().any(|binding| {
                binding.adapter_kind == *adapter_kind && binding.adapter_version == *adapter_version
            }) {
                Ok(())
            } else {
                Err(ReplayError::new(
                    ReplayErrorKind::UnsupportedAdapter,
                    format!("node {node_id} is not certified for adapter {adapter_kind}"),
                ))
            }
        }

        fn node(&self, node_id: &NodeId) -> Result<&spec::NodeSpec> {
            self.certified_spec
                .spec
                .nodes
                .iter()
                .chain(self.certified_spec.spec.remediations.values())
                .find(|node| &node.node_id == node_id)
                .ok_or_else(|| {
                    ReplayError::new(
                        ReplayErrorKind::CertifiedSpec,
                        format!("node {node_id} is not present in certified spec"),
                    )
                })
        }

        fn cell(&self, cell_id: &mfm_ids::CellId) -> Result<&spec::CellSpec> {
            self.certified_spec
                .spec
                .cells
                .iter()
                .find(|cell| &cell.cell_id == cell_id)
                .ok_or_else(|| {
                    ReplayError::new(
                        ReplayErrorKind::CertifiedEvidenceMismatch,
                        format!("cell {cell_id} is not present in certified spec"),
                    )
                })
        }

        fn is_terminal_lifecycle_receipt_artifact(
            &self,
            node_id: &NodeId,
            artifact_id: &ArtifactId,
            digest: &ContentDigest,
        ) -> Result<bool> {
            let node = self.node(node_id)?;
            if !is_terminal_lifecycle_node(node) {
                return Ok(false);
            }
            Ok(matches!(
                self.projection.cell_terminal(&node.output_cell),
                Some(store::CellTerminalProjection::Produced {
                    artifact_id: projected_artifact_id,
                    content_digest,
                    ..
                }) if projected_artifact_id == artifact_id && content_digest == digest
            ))
        }

        fn verify_seed_against_spec(&self, seed: &events::SeedCellRef) -> Result<()> {
            let certified_seed = self
                .certified_spec
                .spec
                .seeds
                .iter()
                .find(|certified_seed| certified_seed.seed_id == seed.seed_id)
                .ok_or_else(|| {
                    ReplayError::new(
                        ReplayErrorKind::CertifiedEvidenceMismatch,
                        format!("seed {} is not present in certified spec", seed.seed_id),
                    )
                })?;
            if certified_seed.cell_id != seed.cell_id
                || certified_seed.scope_id != seed.scope_id
                || certified_seed.semantic_type_id != seed.semantic_type_id
                || certified_seed.schema_id != seed.schema_id
                || certified_seed
                    .required_digest
                    .as_ref()
                    .is_some_and(|digest| digest != &seed.digest)
            {
                return Err(certified_evidence_mismatch(
                    "run-start seed cell does not match certified spec",
                ));
            }
            let cell = self.cell(&seed.cell_id)?;
            if cell.producer != spec::CellProducer::Seed(seed.seed_id.clone())
                || cell.scope_id != seed.scope_id
                || cell.semantic_type_id != seed.semantic_type_id
                || cell.schema_id != seed.schema_id
            {
                return Err(certified_evidence_mismatch(
                    "run-start seed cell terminal does not match certified spec",
                ));
            }
            Ok(())
        }

        fn verify_state_attempt_started_against_spec(
            &self,
            payload: &events::StateAttemptStarted,
        ) -> Result<()> {
            let node = self.node(&payload.node_id)?;
            if node.state_kind != payload.state_kind || node.state_version != payload.state_version
            {
                return Err(certified_evidence_mismatch(
                    "state attempt start does not match certified node identity",
                ));
            }
            Ok(())
        }

        fn verify_state_attempt_completed_against_spec(
            &self,
            payload: &events::StateAttemptCompleted,
        ) -> Result<()> {
            let node = self.node(&payload.node_id)?;
            if node.output_cell != payload.output_cell_id {
                return Err(certified_evidence_mismatch(
                    "state attempt completion output cell does not match certified node",
                ));
            }
            Ok(())
        }

        fn verify_fact_against_spec(&self, payload: &events::FactRecorded) -> Result<()> {
            self.verify_node_capability(
                &payload.node_id,
                &payload.capability_kind,
                &payload.capability_version,
            )?;
            self.verify_node_adapter(
                &payload.node_id,
                &payload.adapter_kind,
                &payload.adapter_version,
            )?;
            Ok(())
        }

        fn verify_side_effect_intent_against_spec(
            &self,
            payload: &side_effect::IntentPersisted,
        ) -> Result<()> {
            let node = self.node(&payload.node_id)?;
            if node.scope_id != payload.scope_id || node.side_effect.is_none() {
                return Err(certified_evidence_mismatch(
                    "side-effect intent does not match certified side-effect node",
                ));
            }
            let contract =
                CertifiedSideEffectContract::for_node(&self.certified_spec.spec, &payload.node_id)
                    .map_err(certified_contract_mismatch)?;
            let forward = match &payload.ledger_purpose {
                events::SideEffectLedgerPurpose::Remediation { forward_pair_id } => {
                    let forward = self
                        .projection
                        .side_effect_for_pair(&self.run_id.run_id, forward_pair_id);
                    forward
                }
                events::SideEffectLedgerPurpose::Forward => None,
            };
            let terminal_policies =
                store::SideEffectTerminalPolicies::from_spec(&self.certified_spec.spec)
                    .map_err(store_error)?;
            contract
                .validate_remediation_link(CertifiedRemediationLink {
                    remediation_run_id: &self.run_id.run_id,
                    ledger_purpose: &payload.ledger_purpose,
                    forward_run_id: forward.map(|projection| &projection.run_id),
                    forward_node_id: forward.map(|projection| &projection.intent.node_id),
                    forward_ledger_purpose: forward.map(|projection| &projection.ledger_purpose),
                    forward_terminal: forward
                        .map(|projection| {
                            terminal_policies
                                .require(&projection.pair_id)
                                .map(|policy| policy.is_terminal_phase(&projection.phase))
                        })
                        .transpose()
                        .map_err(store_error)?
                        .unwrap_or(false),
                })
                .map_err(certified_contract_mismatch)?;
            self.verify_node_capability(
                &payload.node_id,
                &payload.capability_kind,
                &payload.capability_version,
            )?;
            self.verify_node_adapter(
                &payload.node_id,
                &payload.adapter_kind,
                &payload.adapter_version,
            )?;
            Ok(())
        }

        fn verify_invocation_prepared_resource_key(
            &self,
            payload: &side_effect::InvocationPrepared,
            resource_keys: &BTreeMap<SideEffectPairId, events::ResourceKeyEvidence>,
        ) -> Result<()> {
            self.node(&payload.node_id)?;
            let contract =
                CertifiedSideEffectContract::for_node(&self.certified_spec.spec, &payload.node_id)
                    .map_err(certified_contract_mismatch)?;
            contract
                .validate_epoch_resource_consistency(
                    resource_keys.get(&payload.pair_id),
                    payload.resource_key.as_ref(),
                )
                .map_err(certified_contract_mismatch)?;
            Ok(())
        }

        fn verify_resource_lane_claim(
            &self,
            payload: &events::ResourceLaneClaimed,
            resource_keys: &mut BTreeMap<SideEffectPairId, events::ResourceKeyEvidence>,
        ) -> Result<()> {
            self.verify_side_effect_event_against_intent(
                &payload.pair_id,
                &payload.ledger_key,
                payload.invocation_epoch,
                payload.pair_role,
                &payload.node_id,
                &payload.attempt_id,
            )?;
            let contract =
                CertifiedSideEffectContract::for_node(&self.certified_spec.spec, &payload.node_id)
                    .map_err(certified_contract_mismatch)?;
            contract
                .validate_epoch_resource_consistency(
                    resource_keys.get(&payload.pair_id),
                    Some(&payload.resource_key),
                )
                .map_err(certified_contract_mismatch)?;
            resource_keys.insert(payload.pair_id.clone(), payload.resource_key.clone());
            Ok(())
        }

        fn verify_resource_lane_release_against_intent(
            &self,
            pair_id: &SideEffectPairId,
            ledger_key: &events::SideEffectLedgerKey,
            invocation_epoch: u32,
            pair_role: events::SideEffectPairRole,
            release_authority: events::ResourceLaneReleaseAuthority,
        ) -> Result<()> {
            if pair_role != events::SideEffectPairRole::Verify {
                return Err(side_effect_mismatch(
                    "resource lane release requires verify pair role",
                ));
            }
            match release_authority {
                events::ResourceLaneReleaseAuthority::VerifyTerminal
                | events::ResourceLaneReleaseAuthority::ManualResolution => {}
            }
            let intent =
                self.side_effect_intent_for_event(pair_id, ledger_key, invocation_epoch)?;
            let (_verify_node, verify) = self.side_effect_verify_node_for_pair(pair_id)?;
            if verify.submit_node_id != intent.node_id {
                return Err(side_effect_mismatch(
                    "resource lane release does not match certified pair",
                ));
            }
            Ok(())
        }

        fn verify_resource_touched_set(
            &self,
            pair_id: &SideEffectPairId,
            pair_role: events::SideEffectPairRole,
            node_id: &NodeId,
            touched_set: Option<&events::ResourceTouchedSetEvidence>,
        ) -> Result<()> {
            let contract_node_id =
                self.side_effect_contract_node_for_pair_event(pair_id, pair_role, node_id)?;
            CertifiedSideEffectContract::for_node(&self.certified_spec.spec, contract_node_id)
                .and_then(|contract| contract.validate_touched_set(touched_set))
                .map_err(certified_contract_mismatch)
        }

        fn verify_manual_resolution_against_spec(
            &self,
            envelope: &KernelEventEnvelope,
            payload: &events::ManualResolutionRecorded,
        ) -> Result<VerifiedManualResolutionForPrefix> {
            let manual = certified_manual_resolution_spec(&self.certified_spec.spec.saga)
                .ok_or_else(|| {
                    certified_evidence_mismatch(
                        "manual resolution was recorded without certified manual policy",
                    )
                })?;
            if payload.evidence_schema_id != manual.evidence_schema {
                return Err(certified_evidence_mismatch(
                    "manual resolution evidence schema does not match certified policy",
                ));
            }
            let authorization_schema_id =
                manual_authorization_proof_schema_id().map_err(|error| {
                    ReplayError::new(
                        ReplayErrorKind::CertifiedEvidenceMismatch,
                        error.to_string(),
                    )
                })?;
            if payload.authorization_schema_id != authorization_schema_id {
                return Err(certified_evidence_mismatch(
                    "manual resolution authorization schema does not match certified policy",
                ));
            }
            let manual_start = self
                .stream
                .iter()
                .position(|event| {
                    event.seq() == envelope.seq()
                        && event.ordinal() == envelope.ordinal()
                        && event.event_id() == envelope.event_id()
                })
                .ok_or_else(|| {
                    ReplayError::new(
                        ReplayErrorKind::InvalidRunStream,
                        "manual resolution payload was not found in replay stream",
                    )
                })?;
            let prefix_projection =
                ProjectionSnapshot::rebuild_from_run_stream(&self.stream[..manual_start]).map_err(
                    |error| ReplayError::new(ReplayErrorKind::InvalidRunStream, error.to_string()),
                )?;
            let terminal_policies =
                store::SideEffectTerminalPolicies::from_spec(&self.certified_spec.spec)
                    .map_err(store_error)?;
            prefix_projection
                .require_manual_resolution_admissible(
                    &payload.run_id,
                    &self.certified_spec.spec.saga,
                    &terminal_policies,
                )
                .map_err(|error| {
                    ReplayError::new(
                        ReplayErrorKind::CertifiedEvidenceMismatch,
                        error.to_string(),
                    )
                })?;
            let prefix_saga = prefix_projection
                .derive_saga_projection(
                    &payload.run_id,
                    &self.certified_spec.spec.saga,
                    &terminal_policies,
                )
                .map_err(|error| {
                    ReplayError::new(
                        ReplayErrorKind::CertifiedEvidenceMismatch,
                        error.to_string(),
                    )
                })?;
            let block_reason = prefix_saga.manual_block_reason.ok_or_else(|| {
                certified_evidence_mismatch("manual resolution prefix lacks block reason")
            })?;
            let _evidence_artifact = self
                .retained_artifacts
                .values()
                .find(|evidence| {
                    verify_artifact_fields(
                        evidence,
                        &payload.evidence_hash,
                        Some(&payload.evidence_schema_id),
                        None,
                        ArtifactRole::ManualResolutionEvidence,
                        None,
                        None,
                    )
                    .is_ok()
                })
                .ok_or_else(|| {
                    ReplayError::new(
                        ReplayErrorKind::ArtifactMissing,
                        format!(
                            "missing manual resolution evidence artifact {}",
                            payload.evidence_artifact_id
                        ),
                    )
                })?;
            let _authorization_artifact = self
                .retained_artifacts
                .values()
                .find(|evidence| {
                    verify_artifact_fields(
                        evidence,
                        &payload.authorization_hash,
                        Some(&payload.authorization_schema_id),
                        None,
                        ArtifactRole::ManualResolutionAuthorization,
                        None,
                        None,
                    )
                    .is_ok()
                })
                .ok_or_else(|| {
                    ReplayError::new(
                        ReplayErrorKind::ArtifactMissing,
                        format!(
                            "missing manual resolution authorization artifact {}",
                            payload.authorization_artifact_id
                        ),
                    )
                })?;
            let proof_bytes = self
                .artifact_bytes
                .get(&payload.authorization_artifact_id)
                .ok_or_else(|| {
                    ReplayError::new(
                        ReplayErrorKind::ArtifactMissing,
                        format!(
                            "missing manual resolution authorization artifact bytes {}",
                            payload.authorization_artifact_id
                        ),
                    )
                })?;
            let prefix = ManualResolutionPrefixAuthority::new(
                payload.run_id.clone(),
                payload.spec_hash.clone(),
                envelope.seq().as_u64(),
                mfm_runtime::manual_resolution_stream_prefix_digest(&self.stream[..manual_start])?,
                mfm_runtime::manual_resolution_block_reason(block_reason),
                mfm_runtime::unresolved_manual_obligations_digest(&prefix_saga)?,
                manual.clone(),
            )
            .map_err(|error| {
                ReplayError::new(
                    ReplayErrorKind::CertifiedEvidenceMismatch,
                    format!("manual authorization prefix failed validation: {error}"),
                )
            })?;
            let verified = ManualResolutionProofAuthority::new(
                prefix,
                payload.outcome,
                ManualResolutionEvidenceRef {
                    schema_id: payload.evidence_schema_id.clone(),
                    content_hash: payload.evidence_hash.clone(),
                    artifact_id: payload.evidence_artifact_id.clone(),
                },
                ManualResolutionEvidenceRef {
                    schema_id: payload.authorization_schema_id.clone(),
                    content_hash: payload.authorization_hash.clone(),
                    artifact_id: payload.authorization_artifact_id.clone(),
                },
                proof_bytes.clone(),
            )
            .and_then(ManualResolutionProofAuthority::verify)
            .map_err(|error| {
                ReplayError::new(
                    ReplayErrorKind::CertifiedEvidenceMismatch,
                    format!("manual authorization proof failed verification: {error}"),
                )
            })?;
            Ok(verified)
        }

        fn verify_terminal_outcome_agreement(&self) -> Result<()> {
            let Some((terminal_start, payload)) = terminal_completion_event(&self.stream)? else {
                return Ok(());
            };
            match &payload.outcome {
                events::RunCompletionOutcome::Completed(evidence) => {
                    match self
                        .projection
                        .public_output(&evidence.public_output_schema_id)
                    {
                        Some(store::PublicOutputProjection::Produced { event_id, .. })
                            if event_id == &evidence.public_output_event_id =>
                        {
                            Ok(())
                        }
                        _ => Err(certified_evidence_mismatch(
                            "completed terminal outcome does not match projected public output",
                        )),
                    }
                }
                events::RunCompletionOutcome::Compensated
                | events::RunCompletionOutcome::ManuallyResolved
                | events::RunCompletionOutcome::FailedWithoutAcdcClaim => {
                    let prefix_next_seq = self.stream[..terminal_start]
                        .last()
                        .map(|event| {
                            let next = event
                                .seq()
                                .as_u64()
                                .checked_add(1)
                                .ok_or(store::StoreError::SequenceOverflow)?;
                            store::StreamSeq::new(next)
                        })
                        .transpose()?
                        .unwrap_or(store::StreamSeq::FIRST);
                    let prefix_projection = ProjectionSnapshot::rebuild_from_run_stream(
                        &self.stream[..terminal_start],
                    )?;
                    let terminal_policies =
                        store::SideEffectTerminalPolicies::from_spec(&self.certified_spec.spec)
                            .map_err(store_error)?;
                    let saga = prefix_projection.derive_saga_projection(
                        &self.run_id.run_id,
                        &self.certified_spec.spec.saga,
                        &terminal_policies,
                    )?;
                    let proof = store::SagaTerminalProof::new(
                        &self.certified_spec.spec.saga,
                        &saga,
                        prefix_next_seq,
                        self.manual_resolutions.get(&self.run_id.run_id).cloned(),
                    )
                    .map_err(|error| {
                        ReplayError::new(
                            ReplayErrorKind::CertifiedEvidenceMismatch,
                            error.to_string(),
                        )
                    })?;
                    if proof.outcome() == payload.outcome {
                        Ok(())
                    } else {
                        Err(ReplayError::new(
                            ReplayErrorKind::CertifiedEvidenceMismatch,
                            "saga terminal outcome does not match proof",
                        ))
                    }
                }
            }
        }

        fn reject_unauthorized_artifact_evidence(&self) -> Result<()> {
            for (key, evidence) in &self.retained_artifacts {
                if !self.artifacts.contains_key(key) {
                    return Err(ReplayError::new(
                        ReplayErrorKind::ArtifactMismatch,
                        format!(
                            "retained artifact evidence supplied without replay authorization for {}",
                            evidence.artifact_id
                        ),
                    ));
                }
            }
            for artifact_id in self.artifact_bytes.keys() {
                if !self
                    .retained_artifacts
                    .values()
                    .any(|evidence| &evidence.artifact_id == artifact_id)
                {
                    return Err(ReplayError::new(
                        ReplayErrorKind::ArtifactMismatch,
                        format!(
                            "artifact bytes supplied without verified evidence for {artifact_id}"
                        ),
                    ));
                }
            }
            Ok(())
        }

        fn verify_side_effect_event_against_intent(
            &self,
            pair_id: &SideEffectPairId,
            ledger_key: &events::SideEffectLedgerKey,
            invocation_epoch: u32,
            pair_role: events::SideEffectPairRole,
            node_id: &NodeId,
            attempt_id: &AttemptId,
        ) -> Result<()> {
            let intent =
                self.side_effect_intent_for_event(pair_id, ledger_key, invocation_epoch)?;
            match pair_role {
                events::SideEffectPairRole::Submit => {
                    verify_side_effect_submit_claim_identity(
                        intent,
                        node_id,
                        attempt_id,
                        "side-effect event does not match persisted intent",
                    )?;
                }
                events::SideEffectPairRole::Verify => {
                    let (verify_node, verify) = self.side_effect_verify_node_for_pair(pair_id)?;
                    if verify.submit_node_id != intent.node_id || verify_node.node_id != *node_id {
                        return Err(side_effect_mismatch(
                            "side-effect verify event does not match certified pair",
                        ));
                    }
                    self.node(node_id)?;
                }
            }
            Ok(())
        }

        fn verify_side_effect_ambiguity_against_intent(
            &self,
            pair_id: &SideEffectPairId,
            ledger_key: &events::SideEffectLedgerKey,
            invocation_epoch: u32,
            pair_role: events::SideEffectPairRole,
            node_id: &NodeId,
            attempt_id: &AttemptId,
        ) -> Result<()> {
            self.verify_side_effect_event_against_intent(
                pair_id,
                ledger_key,
                invocation_epoch,
                pair_role,
                node_id,
                attempt_id,
            )
        }

        fn side_effect_intent_for_event(
            &self,
            pair_id: &SideEffectPairId,
            ledger_key: &events::SideEffectLedgerKey,
            invocation_epoch: u32,
        ) -> Result<&side_effect::IntentPersisted> {
            let intent = self.intents.get(pair_id).ok_or_else(|| {
                ReplayError::new(
                    ReplayErrorKind::SideEffectMissing,
                    format!("missing side-effect intent {pair_id}"),
                )
            })?;
            if intent.ledger_key != *ledger_key
                || intent.pair_id != *pair_id
                || intent.invocation_epoch != invocation_epoch
            {
                return Err(side_effect_mismatch(
                    "side-effect event does not match persisted intent",
                ));
            }
            Ok(intent)
        }

        fn side_effect_contract_node_for_pair_event(
            &self,
            pair_id: &SideEffectPairId,
            pair_role: events::SideEffectPairRole,
            node_id: &NodeId,
        ) -> Result<&NodeId> {
            let intent = self.intents.get(pair_id).ok_or_else(|| {
                ReplayError::new(
                    ReplayErrorKind::SideEffectMissing,
                    format!("missing side-effect intent {pair_id}"),
                )
            })?;
            match pair_role {
                events::SideEffectPairRole::Submit => {
                    if intent.node_id != *node_id {
                        return Err(side_effect_mismatch(
                            "side-effect event does not match persisted intent",
                        ));
                    }
                    Ok(&intent.node_id)
                }
                events::SideEffectPairRole::Verify => {
                    let (verify_node, verify) = self.side_effect_verify_node_for_pair(pair_id)?;
                    if verify.submit_node_id != intent.node_id || verify_node.node_id != *node_id {
                        return Err(side_effect_mismatch(
                            "side-effect verify event does not match certified pair",
                        ));
                    }
                    Ok(&intent.node_id)
                }
            }
        }

        fn side_effect_verify_node_for_pair(
            &self,
            pair_id: &SideEffectPairId,
        ) -> Result<(&spec::NodeSpec, &spec::SideEffectVerifyNodeSpec)> {
            let pair = self
                .certified_spec
                .spec
                .side_effect_verify_pair_for_pair_id(pair_id)
                .map_err(certified_spec_error)?;
            Ok((pair.verify_node, pair.verify))
        }

        fn verify_cell_produced_against_spec(&self, payload: &events::CellProduced) -> Result<()> {
            let node = self.node(&payload.node_id)?;
            if node.output_cell != payload.cell_id {
                return Err(certified_evidence_mismatch(
                    "produced cell does not match certified node output",
                ));
            }
            let cell = self.cell(&payload.cell_id)?;
            if cell.producer != spec::CellProducer::Node(payload.node_id.clone())
                || cell.scope_id != payload.scope_id
                || cell.semantic_type_id != payload.semantic_type_id
                || cell.schema_id != payload.schema_id
                || cell.value_lineage != payload.value_lineage
            {
                return Err(certified_evidence_mismatch(
                    "produced cell does not match certified cell spec",
                ));
            }
            Ok(())
        }

        fn verify_cell_skipped_against_spec(&self, payload: &events::CellSkipped) -> Result<()> {
            let node = self.node(&payload.node_id)?;
            if node.output_cell != payload.cell_id {
                return Err(certified_evidence_mismatch(
                    "skipped cell does not match certified node output",
                ));
            }
            let cell = self.cell(&payload.cell_id)?;
            if cell.producer != spec::CellProducer::Node(payload.node_id.clone())
                || cell.scope_id != payload.scope_id
                || cell.semantic_type_id != payload.semantic_type_id
                || cell.schema_id != payload.schema_id
                || cell.value_lineage != payload.value_lineage
            {
                return Err(certified_evidence_mismatch(
                    "skipped cell does not match certified cell spec",
                ));
            }
            Ok(())
        }

        fn verify_public_output_against_spec(
            &self,
            payload: &events::PublicOutputProduced,
        ) -> Result<()> {
            let render = self.verify_public_output_header(
                payload.node_id.clone(),
                &payload.receipt_cell_id,
                &payload.public_schema_id,
                &payload.renderer_descriptor_id,
            )?;
            if payload.output_spec_digest != render.output_spec_digest
                || payload.cells.len() != render.required_cells.len()
            {
                return Err(certified_evidence_mismatch(
                    "public output event does not match certified render contract",
                ));
            }
            for (actual, expected) in payload.cells.iter().zip(render.required_cells.iter()) {
                if actual.public_field_path != expected.public_field_path
                    || actual.cell_id != expected.cell_id
                    || actual.producer != expected.producer
                    || actual.scope_id != expected.scope_id
                    || actual.semantic_type_id != expected.semantic_type_id
                    || actual.schema_id != expected.schema_id
                    || actual.value_lineage != expected.value_lineage
                {
                    return Err(certified_evidence_mismatch(
                        "public output cell does not match certified spec",
                    ));
                }
                self.verify_public_output_cell_projection(actual)?;
            }
            Ok(())
        }

        fn verify_public_output_render_failure_against_spec(
            &self,
            payload: &events::PublicOutputRenderFailed,
        ) -> Result<()> {
            self.verify_public_output_header(
                payload.node_id.clone(),
                &self.node(&payload.node_id)?.output_cell.clone(),
                &payload.public_schema_id,
                &payload.renderer_descriptor_id,
            )?;
            Ok(())
        }

        fn verify_public_output_header(
            &self,
            node_id: NodeId,
            receipt_cell_id: &mfm_ids::CellId,
            public_schema_id: &SchemaId,
            renderer_descriptor_id: &mfm_ids::DescriptorId,
        ) -> Result<&spec::PublicOutputRenderNodeSpec> {
            let node = self.node(&node_id)?;
            let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &node.framework else {
                return Err(certified_evidence_mismatch(
                    "public output event node is not a certified public-output render node",
                ));
            };
            if node.output_cell != *receipt_cell_id
                || render.public_schema_id != *public_schema_id
                || self.certified_spec.spec.public_outputs.public_schema_id != *public_schema_id
                || render.renderer_descriptor.descriptor_id != *renderer_descriptor_id
            {
                return Err(certified_evidence_mismatch(
                    "public output event does not match certified public-output contract",
                ));
            }
            Ok(render)
        }

        fn verify_public_output_cell_projection(
            &self,
            cell: &events::NamedTypedCellRef,
        ) -> Result<()> {
            let Some(projection) = self.projection.cell_terminal(&cell.cell_id) else {
                return Err(certified_evidence_mismatch(
                    "public output cell has no terminal projection",
                ));
            };
            match projection {
                store::CellTerminalProjection::Produced {
                    schema_id,
                    semantic_type_id,
                    artifact_id,
                    content_digest,
                    ..
                } if schema_id == &cell.schema_id
                    && semantic_type_id == &cell.semantic_type_id
                    && artifact_id == &cell.artifact_id
                    && content_digest == &cell.content_digest =>
                {
                    Ok(())
                }
                _ => Err(certified_evidence_mismatch(
                    "public output cell evidence does not match produced cell projection",
                )),
            }
        }

        fn verify_completed_run_public_output(
            &self,
            evidence: &events::PublicOutputCompletionEvidence,
        ) -> Result<()> {
            if evidence.public_output_schema_id
                != self.certified_spec.spec.public_outputs.public_schema_id
            {
                return Err(certified_evidence_mismatch(
                    "run completion public-output schema does not match certified spec",
                ));
            }
            match self
                .projection
                .public_output(&evidence.public_output_schema_id)
            {
                Some(store::PublicOutputProjection::Produced { event_id, .. })
                    if event_id == &evidence.public_output_event_id =>
                {
                    Ok(())
                }
                _ => Err(certified_evidence_mismatch(
                    "run completion public-output evidence does not match projected output",
                )),
            }
        }

        fn authorize_event_artifacts(&mut self, payload: &KernelEventPayload) -> Result<()> {
            for requirement in store::event_artifact_requirements(payload) {
                if self.should_skip_event_artifact_requirement(&requirement)? {
                    self.authorize_skipped_event_artifact_requirement(&requirement)?;
                    continue;
                }
                self.authorize_event_artifact_requirement(&requirement)?;
            }
            Ok(())
        }

        fn should_skip_event_artifact_requirement(
            &self,
            requirement: &store::EventArtifactRequirement,
        ) -> Result<bool> {
            if requirement.source == store::EventArtifactReferenceSource::PublicOutputCell {
                return Ok(true);
            }
            if !requirement.source.is_terminal_lifecycle_receipt_candidate() {
                return Ok(false);
            }
            if requirement.artifact_role != Some(ArtifactRole::StateOutput) {
                return Ok(false);
            }
            let Some(node_id) = requirement.producer_node_id.as_ref() else {
                return Ok(false);
            };
            let node = self.node(node_id)?;
            if !is_terminal_lifecycle_node(node) {
                return Ok(false);
            }
            if requirement.source == store::EventArtifactReferenceSource::ArtifactReferenced {
                let cell = self.cell(&node.output_cell)?;
                if requirement.schema_id.as_ref() != Some(&cell.schema_id)
                    || requirement.semantic_type_id.as_ref() != Some(&cell.semantic_type_id)
                {
                    return Ok(false);
                }
            }
            let Some(digest) = requirement.digest.as_ref() else {
                return Ok(false);
            };
            self.is_terminal_lifecycle_receipt_artifact(node_id, &requirement.artifact_id, digest)
        }

        fn authorize_skipped_event_artifact_requirement(
            &mut self,
            requirement: &store::EventArtifactRequirement,
        ) -> Result<StoredArtifactEvidenceRef> {
            self.validate_event_artifact_requirement(
                requirement,
                "skipped artifact evidence mismatch",
            )
        }

        fn authorize_event_artifact_requirement(
            &mut self,
            requirement: &store::EventArtifactRequirement,
        ) -> Result<StoredArtifactEvidenceRef> {
            if requirement.digest.is_none() {
                return Err(ReplayError::new(
                    ReplayErrorKind::InvalidRunStream,
                    format!(
                        "artifact requirement for {} does not carry a digest",
                        requirement.artifact_id
                    ),
                ));
            }
            if requirement.source == store::EventArtifactReferenceSource::RetentionRef
                && requirement.artifact_role.is_none()
            {
                return Err(ReplayError::new(
                    ReplayErrorKind::InvalidRunStream,
                    format!(
                        "retention requirement for {} does not carry an artifact role",
                        requirement.artifact_id
                    ),
                ));
            }
            if requirement.artifact_role.is_none() && requirement.schema_id.is_none() {
                return Err(ReplayError::new(
                    ReplayErrorKind::InvalidRunStream,
                    format!(
                        "schema-only requirement for {} does not carry a schema id",
                        requirement.artifact_id
                    ),
                ));
            }
            self.validate_event_artifact_requirement(requirement, "artifact evidence mismatch")
        }

        fn validate_event_artifact_requirement(
            &mut self,
            requirement: &store::EventArtifactRequirement,
            _mismatch_context: &'static str,
        ) -> Result<StoredArtifactEvidenceRef> {
            if requirement.digest.is_none() {
                return Err(ReplayError::new(
                    ReplayErrorKind::InvalidRunStream,
                    format!(
                        "artifact requirement for {} does not carry a digest",
                        requirement.artifact_id
                    ),
                ));
            }
            let evidence = self
                .retained_artifacts
                .values()
                .find(|evidence| {
                    store::validate_artifact_requirement_against_evidence(requirement, evidence)
                        .is_ok()
                })
                .ok_or_else(|| {
                    ReplayError::new(
                        ReplayErrorKind::ArtifactMissing,
                        format!(
                            "missing retained artifact evidence for {}",
                            requirement.artifact_id
                        ),
                    )
                })?;
            let evidence = evidence.clone();
            self.insert_authorized_artifact(evidence.clone())?;
            Ok(evidence)
        }

        fn authorize_artifact(
            &mut self,
            expected: ArtifactEvidenceExpectation<'_>,
        ) -> Result<StoredArtifactEvidenceRef> {
            let evidence = self
                .retained_artifacts
                .values()
                .find(|evidence| verify_artifact_expectation(evidence, expected).is_ok())
                .ok_or_else(|| {
                    ReplayError::new(
                        ReplayErrorKind::ArtifactMissing,
                        format!(
                            "missing retained artifact evidence for {}",
                            expected.artifact_id
                        ),
                    )
                })?;
            let evidence = evidence.clone();
            self.insert_authorized_artifact(evidence.clone())?;
            Ok(evidence)
        }

        fn insert_authorized_artifact(
            &mut self,
            evidence: StoredArtifactEvidenceRef,
        ) -> Result<()> {
            let key = replay_artifact_authority_key(&evidence)?;
            if let Some(existing) = self.artifacts.get(&key) {
                if existing != &evidence {
                    return Err(ReplayError::new(
                        ReplayErrorKind::ArtifactMismatch,
                        format!(
                            "conflicting authorized artifact evidence for {}",
                            evidence.artifact_id
                        ),
                    ));
                }
                return Ok(());
            }
            self.artifacts.insert(key, evidence);
            Ok(())
        }

        fn verify_artifact(
            &self,
            expected: ArtifactEvidenceExpectation<'_>,
        ) -> Result<StoredArtifactEvidenceRef> {
            let evidence = self
                .artifacts
                .values()
                .find(|evidence| verify_artifact_expectation(evidence, expected).is_ok())
                .ok_or_else(|| {
                    ReplayError::new(
                        ReplayErrorKind::ArtifactMissing,
                        format!(
                            "missing replay-authorized artifact evidence for {}",
                            expected.artifact_id
                        ),
                    )
                })?;
            Ok(evidence.clone())
        }

        fn artifact_bytes(&self, artifact_id: &ArtifactId) -> Result<&[u8]> {
            self.artifact_bytes
                .get(artifact_id)
                .map(Vec::as_slice)
                .ok_or_else(|| {
                    ReplayError::new(
                        ReplayErrorKind::ArtifactMissing,
                        format!("missing replay-authorized artifact bytes for {artifact_id}"),
                    )
                })
        }
    }

    fn artifact_map(
        artifacts: Vec<StoredArtifactEvidenceRef>,
    ) -> Result<BTreeMap<ReplayArtifactAuthorityKey, StoredArtifactEvidenceRef>> {
        let mut map = BTreeMap::new();
        for artifact in artifacts {
            let key = replay_artifact_authority_key(&artifact)?;
            if let Some(existing) = map.get(&key) {
                if existing != &artifact {
                    return Err(ReplayError::new(
                        ReplayErrorKind::ArtifactMismatch,
                        format!(
                            "conflicting retained artifact evidence for {}",
                            artifact.artifact_id
                        ),
                    ));
                }
            } else {
                map.insert(key, artifact);
            }
        }
        Ok(map)
    }

    fn artifact_bytes_map(
        artifacts: Vec<ReplayArtifactBytes>,
    ) -> Result<BTreeMap<ArtifactId, Vec<u8>>> {
        let mut map = BTreeMap::new();
        for artifact in artifacts {
            match map.entry(artifact.artifact_id) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(artifact.bytes);
                }
                std::collections::btree_map::Entry::Occupied(entry) => {
                    if entry.get() != &artifact.bytes {
                        return Err(ReplayError::new(
                            ReplayErrorKind::ArtifactMismatch,
                            format!("conflicting retained artifact bytes for {}", entry.key()),
                        ));
                    }
                }
            }
        }
        Ok(map)
    }

    fn verify_replay_artifact_authority(
        verified_view: &mfm_runtime::VerifiedRunHistoryView,
        artifacts: &BTreeMap<ReplayArtifactAuthorityKey, StoredArtifactEvidenceRef>,
    ) -> Result<()> {
        let verified_artifacts = verified_view.artifact_store();
        for (key, artifact) in verified_artifacts.artifacts() {
            let evidence = artifacts.get(key).ok_or_else(|| {
                ReplayError::new(
                    ReplayErrorKind::ArtifactMissing,
                    format!(
                        "missing verified artifact evidence for {}",
                        artifact.evidence().artifact_id
                    ),
                )
            })?;
            if evidence != artifact.evidence() {
                return Err(ReplayError::new(
                    ReplayErrorKind::ArtifactMismatch,
                    format!(
                        "verified artifact evidence mismatch for {}",
                        artifact.evidence().artifact_id
                    ),
                ));
            }
        }
        for (key, evidence) in artifacts {
            if !verified_artifacts
                .artifacts()
                .any(|(verified_key, _)| verified_key == key)
            {
                return Err(ReplayError::new(
                    ReplayErrorKind::ArtifactMismatch,
                    format!(
                        "unverified artifact evidence supplied for {}",
                        evidence.artifact_id
                    ),
                ));
            }
        }
        Ok(())
    }

    fn terminal_completion_event(
        stream: &[KernelEventEnvelope],
    ) -> Result<Option<(usize, &events::RunCompleted)>> {
        let mut found = None;
        let mut index = 0;
        while index < stream.len() {
            let first = &stream[index];
            let seq = first.seq();
            let commit_key = first.commit_key().clone();
            let start = index;
            let mut end = index + 1;
            while end < stream.len()
                && stream[end].seq() == seq
                && stream[end].commit_key() == &commit_key
            {
                end += 1;
            }
            for event in &stream[start..end] {
                if let KernelEventPayload::RunCompleted(payload) = event.payload() {
                    if found.replace((start, payload)).is_some() {
                        return Err(ReplayError::new(
                            ReplayErrorKind::InvalidRunStream,
                            "run stream contains multiple terminal completions",
                        ));
                    }
                }
            }
            index = end;
        }
        Ok(found)
    }

    fn verify_resource_lane_release_adjacency(
        certified_spec: &HashedSpecEnvelope,
        stream: &[KernelEventEnvelope],
    ) -> Result<()> {
        let terminal_policies = store::SideEffectTerminalPolicies::from_spec(&certified_spec.spec)
            .map_err(store_error)?;
        let mut index = 0;
        while index < stream.len() {
            let first = &stream[index];
            let seq = first.seq();
            let commit_key = first.commit_key().clone();
            let start = index;
            let mut end = index + 1;
            while end < stream.len()
                && stream[end].seq() == seq
                && stream[end].commit_key() == &commit_key
            {
                end += 1;
            }
            let payloads = stream[start..end]
                .iter()
                .map(KernelEventEnvelope::payload)
                .collect::<Vec<_>>();
            verify_resource_lane_release_payload_adjacency(&terminal_policies, &payloads)?;
            index = end;
        }
        Ok(())
    }

    fn verify_resource_lane_release_payload_adjacency(
        terminal_policies: &store::SideEffectTerminalPolicies,
        payloads: &[&KernelEventPayload],
    ) -> Result<()> {
        for (index, payload) in payloads.iter().enumerate() {
            let KernelEventPayload::ResourceLaneReleased(release) = payload else {
                continue;
            };
            match release.release_authority {
                events::ResourceLaneReleaseAuthority::VerifyTerminal => {
                    let mut matched = false;
                    for terminal in &payloads[index + 1..] {
                        if side_effect_terminal_matches_resource_lane_release(
                            terminal,
                            release,
                            terminal_policies,
                        )? {
                            matched = true;
                            break;
                        }
                    }
                    if !matched {
                        return Err(ReplayError::new(
                            ReplayErrorKind::InvalidRunStream,
                            "resource lane release requires matching verify-terminal payload in the same commit",
                        ));
                    }
                }
                events::ResourceLaneReleaseAuthority::ManualResolution => {
                    if !payloads[index + 1..].iter().any(|payload| {
                        matches!(payload, KernelEventPayload::ManualResolutionRecorded(_))
                    }) {
                        return Err(ReplayError::new(
                            ReplayErrorKind::InvalidRunStream,
                            "manual resource lane release requires ManualResolutionRecorded in the same commit",
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn side_effect_terminal_matches_resource_lane_release(
        terminal: &KernelEventPayload,
        release: &events::ResourceLaneReleased,
        terminal_policies: &store::SideEffectTerminalPolicies,
    ) -> Result<bool> {
        let Some(terminal) = terminal.side_effect_ledger_ref() else {
            return Ok(false);
        };
        if terminal.ledger_key != &release.ledger_key
            || terminal.ledger_purpose != &release.ledger_purpose
            || terminal.pair_id != &release.pair_id
            || terminal.invocation_epoch != Some(release.invocation_epoch)
        {
            return Ok(false);
        }
        if !side_effect_terminal_release_role_allowed(
            terminal.kind,
            terminal.pair_role,
            release.pair_role,
        ) {
            return Ok(false);
        }
        side_effect_terminal_release_policy_allowed(
            terminal.kind,
            terminal.pair_id,
            terminal_policies,
        )
    }

    fn side_effect_terminal_release_policy_allowed(
        terminal_kind: events::SideEffectEventKind,
        pair_id: &SideEffectPairId,
        terminal_policies: &store::SideEffectTerminalPolicies,
    ) -> Result<bool> {
        Ok(match terminal_kind {
            events::SideEffectEventKind::ReceiptObserved => {
                terminal_policies.require(pair_id).map_err(store_error)?
                    == store::SideEffectTerminalPolicy::Receipt
            }
            events::SideEffectEventKind::ConfirmationObserved
            | events::SideEffectEventKind::NotSubmittedProven
            | events::SideEffectEventKind::Failed => true,
            _ => false,
        })
    }

    fn side_effect_terminal_release_role_allowed(
        terminal_kind: events::SideEffectEventKind,
        terminal_role: events::SideEffectPairRole,
        release_role: events::SideEffectPairRole,
    ) -> bool {
        if release_role != events::SideEffectPairRole::Verify {
            return false;
        }
        match terminal_kind {
            events::SideEffectEventKind::NotSubmittedProven => matches!(
                terminal_role,
                events::SideEffectPairRole::Submit | events::SideEffectPairRole::Verify
            ),
            events::SideEffectEventKind::ReceiptObserved
            | events::SideEffectEventKind::ConfirmationObserved => {
                terminal_role == events::SideEffectPairRole::Verify
            }
            events::SideEffectEventKind::Failed => matches!(
                terminal_role,
                events::SideEffectPairRole::Submit | events::SideEffectPairRole::Verify
            ),
            _ => false,
        }
    }

    fn verify_remediation_ledger_links(
        certified_spec: &HashedSpecEnvelope,
        projection: &ProjectionSnapshot,
    ) -> Result<()> {
        for (_, side_effect) in projection.side_effects() {
            let contract = CertifiedSideEffectContract::for_node(
                &certified_spec.spec,
                &side_effect.intent.node_id,
            )
            .map_err(certified_contract_mismatch)?;
            let forward = match &side_effect.ledger_purpose {
                events::SideEffectLedgerPurpose::Remediation { forward_pair_id } => {
                    let forward =
                        projection.side_effect_for_pair(&side_effect.run_id, forward_pair_id);
                    forward
                }
                events::SideEffectLedgerPurpose::Forward => None,
            };
            let terminal_policies =
                store::SideEffectTerminalPolicies::from_spec(&certified_spec.spec)
                    .map_err(store_error)?;
            contract
                .validate_remediation_link(CertifiedRemediationLink {
                    remediation_run_id: &side_effect.run_id,
                    ledger_purpose: &side_effect.ledger_purpose,
                    forward_run_id: forward.map(|projection| &projection.run_id),
                    forward_node_id: forward.map(|projection| &projection.intent.node_id),
                    forward_ledger_purpose: forward.map(|projection| &projection.ledger_purpose),
                    forward_terminal: forward
                        .map(|projection| {
                            terminal_policies
                                .require(&projection.pair_id)
                                .map(|policy| policy.is_terminal_phase(&projection.phase))
                        })
                        .transpose()
                        .map_err(store_error)?
                        .unwrap_or(false),
                })
                .map_err(certified_contract_mismatch)?;
        }
        Ok(())
    }

    fn certified_manual_resolution_spec(
        policy: &spec::SagaPolicySpec,
    ) -> Option<&spec::ManualResolutionEvidenceSpec> {
        match policy {
            spec::SagaPolicySpec::ManualResolution { manual } => Some(manual),
            spec::SagaPolicySpec::CompensateCompleted {
                on_remediation_unresolved:
                    spec::RemediationUnresolvedSpec::ManualResolution { manual },
            } => Some(manual.as_ref()),
            spec::SagaPolicySpec::NoSideEffects
            | spec::SagaPolicySpec::FailWithoutAcdcClaim
            | spec::SagaPolicySpec::CompensateCompleted {
                on_remediation_unresolved: spec::RemediationUnresolvedSpec::FailWithoutAcdcClaim,
            } => None,
        }
    }

    fn is_terminal_lifecycle_node(node: &spec::NodeSpec) -> bool {
        matches!(
            &node.framework,
            Some(
                spec::FrameworkNodeSpec::CompleteRun(_)
                    | spec::FrameworkNodeSpec::ResolveSagaTerminal(_)
            )
        )
    }

    fn verify_artifact_fields(
        evidence: &StoredArtifactEvidenceRef,
        digest: &ContentDigest,
        schema_id: Option<&SchemaId>,
        semantic_type_id: Option<&SemanticTypeId>,
        role: ArtifactRole,
        producer_node_id: Option<&NodeId>,
        producer_seed_id: Option<&SeedId>,
    ) -> Result<()> {
        let requirement = store::EventArtifactRequirement {
            source: store::EventArtifactReferenceSource::ArtifactReferenced,
            artifact_id: evidence.artifact_id.clone(),
            digest: Some(digest.clone()),
            byte_len: None,
            media_type: None,
            schema_id: schema_id.cloned(),
            semantic_type_id: semantic_type_id.cloned(),
            producer_node_id: producer_node_id.cloned(),
            producer_seed_id: producer_seed_id.cloned(),
            artifact_role: Some(role),
        };
        store::validate_artifact_requirement_against_evidence(&requirement, evidence)
            .map_err(|error| artifact_requirement_replay_error(error, "artifact evidence mismatch"))
    }

    fn verify_artifact_expectation(
        evidence: &StoredArtifactEvidenceRef,
        expected: ArtifactEvidenceExpectation<'_>,
    ) -> Result<()> {
        verify_artifact_fields(
            evidence,
            expected.digest,
            expected.schema_id,
            expected.semantic_type_id,
            expected.role,
            expected.producer_node_id,
            expected.producer_seed_id,
        )
    }

    fn replay_artifact_authority_key(
        evidence: &StoredArtifactEvidenceRef,
    ) -> Result<ReplayArtifactAuthorityKey> {
        Ok((
            evidence.artifact_id.clone(),
            evidence.evidence_hash().map_err(ReplayError::from)?,
        ))
    }

    fn artifact_requirement_replay_error(
        error: store::StoreError,
        context: &'static str,
    ) -> ReplayError {
        match error {
            store::StoreError::ArtifactEvidenceMismatch { artifact_id, field } => ReplayError::new(
                ReplayErrorKind::ArtifactMismatch,
                format!("{context} for {artifact_id} field {field}"),
            ),
            error => error.into(),
        }
    }

    fn certified_evidence_mismatch(message: &'static str) -> ReplayError {
        ReplayError::new(ReplayErrorKind::CertifiedEvidenceMismatch, message)
    }

    fn certified_contract_mismatch(error: mfm_certify::CertifyError) -> ReplayError {
        ReplayError::new(
            ReplayErrorKind::CertifiedEvidenceMismatch,
            error.to_string(),
        )
    }

    fn certified_spec_error(error: mfm_spec::SpecError) -> ReplayError {
        ReplayError::new(
            ReplayErrorKind::CertifiedEvidenceMismatch,
            error.to_string(),
        )
    }

    fn store_error(error: store::StoreError) -> ReplayError {
        ReplayError::from(error)
    }

    fn run_admitted_payload(stream: &[KernelEventEnvelope]) -> Result<events::RunAdmitted> {
        let mut run_admitted = None;
        for envelope in stream {
            if let KernelEventPayload::RunAdmitted(payload) = envelope.payload() {
                if run_admitted.is_some() {
                    return Err(ReplayError::new(
                        ReplayErrorKind::InvalidRunStream,
                        "run stream contains more than one RunAdmitted event",
                    ));
                }
                run_admitted = Some((**payload).clone());
            }
        }
        run_admitted.ok_or_else(|| {
            ReplayError::new(
                ReplayErrorKind::RunAdmittedMissing,
                "run stream contains no RunAdmitted event",
            )
        })
    }

    fn verify_run_start_contract(
        certified_spec: &HashedSpecEnvelope,
        run_admitted: &events::RunAdmitted,
        authority: &ReplayReadAuthority,
        artifacts: &BTreeMap<ReplayArtifactAuthorityKey, StoredArtifactEvidenceRef>,
    ) -> Result<()> {
        if run_admitted.spec_version != certified_spec.spec.spec_version
            || run_admitted.lowering_version != certified_spec.spec.lowering_version
            || run_admitted.public_output_schema_id
                != certified_spec.spec.public_outputs.public_schema_id
        {
            return Err(ReplayError::new(
                ReplayErrorKind::CertifiedSpec,
                "run-start spec contract fields do not match certified spec",
            ));
        }
        verify_run_identity_material(certified_spec, run_admitted)?;
        if run_admitted.descriptor_identities != certified_spec.spec.descriptor_identities {
            return Err(ReplayError::new(
                ReplayErrorKind::DescriptorIdentityMismatch,
                "run-start descriptor identities do not match certified spec",
            ));
        }
        let renderer_canonicalizer = &certified_spec
            .spec
            .public_outputs
            .renderer_descriptor
            .canonicalizer_identity;
        if run_admitted.canonicalizer_identity != *renderer_canonicalizer
            || authority.canonicalizer_identity != *renderer_canonicalizer
        {
            return Err(ReplayError::new(
                ReplayErrorKind::CanonicalizerMismatch,
                "run-start canonicalizer identity does not match replay authority",
            ));
        }
        if run_admitted.runner_executables != authority.runner_executables {
            return Err(ReplayError::new(
                ReplayErrorKind::ExecutableIdentityMismatch,
                "runner executable identities do not match replay authority",
            ));
        }
        if run_admitted.adapter_executables != authority.adapter_executables {
            return Err(ReplayError::new(
                ReplayErrorKind::AdapterExecutableMismatch,
                "adapter executable identities do not match replay authority",
            ));
        }
        let binding_digest = admitted_binding_digest(
            &authority.runner_executables,
            &authority.adapter_executables,
        )?;
        if run_admitted.admitted_binding_digest != binding_digest {
            return Err(ReplayError::new(
                ReplayErrorKind::ExecutableIdentityMismatch,
                "RunAdmitted binding digest does not match replay authority",
            ));
        }
        verify_run_artifact(
            artifacts,
            &run_admitted.spec_artifact,
            ArtifactRole::TypedExecutionSpec,
        )?;
        let spec_schema_id = spec::typed_execution_spec_schema_id().map_err(|error| {
            ReplayError::new(
                ReplayErrorKind::ArtifactMismatch,
                format!("typed execution spec schema id is invalid: {error}"),
            )
        })?;
        if run_admitted.spec_artifact.content_digest != spec_digest(&certified_spec.spec_hash)
            || run_admitted.spec_artifact.schema_id.as_ref() != Some(&spec_schema_id)
        {
            return Err(ReplayError::new(
                ReplayErrorKind::ArtifactMismatch,
                "certified spec artifact evidence does not match run admission",
            ));
        }
        verify_run_artifact(
            artifacts,
            &run_admitted.certificate_artifact,
            ArtifactRole::TypedSpecCertificate,
        )?;
        let certificate_schema_id =
            mfm_certify::typed_spec_certificate_schema_id().map_err(|error| {
                ReplayError::new(
                    ReplayErrorKind::ArtifactMismatch,
                    format!("typed spec certificate schema id is invalid: {error}"),
                )
            })?;
        if run_admitted.certificate_artifact.schema_id.as_ref() != Some(&certificate_schema_id) {
            return Err(ReplayError::new(
                ReplayErrorKind::ArtifactMismatch,
                "typed spec certificate artifact evidence does not match run admission",
            ));
        }
        for artifact in &run_admitted.config_artifacts {
            verify_run_artifact(artifacts, artifact, ArtifactRole::TypedConfig)?;
        }
        Ok(())
    }

    fn verify_run_identity_material(
        certified_spec: &HashedSpecEnvelope,
        run_admitted: &events::RunAdmitted,
    ) -> Result<()> {
        if run_admitted.identity_material.certified_spec_hash != run_admitted.spec_hash
            || run_admitted.identity_material.certified_spec_hash != certified_spec.spec_hash
        {
            return Err(ReplayError::new(
                ReplayErrorKind::SpecHashMismatch,
                "run identity material spec hash does not match certified run start",
            ));
        }
        let derived_run_id = run_admitted
            .identity_material
            .derive_run_id()
            .map_err(|_| {
                ReplayError::new(
                    ReplayErrorKind::InvalidRunStream,
                    "run identity material is invalid",
                )
            })?;
        if derived_run_id != run_admitted.run_id {
            return Err(ReplayError::new(
                ReplayErrorKind::InvalidRunStream,
                "RunAdmitted run id does not match identity material",
            ));
        }
        Ok(())
    }

    fn verify_run_artifact(
        artifacts: &BTreeMap<ReplayArtifactAuthorityKey, StoredArtifactEvidenceRef>,
        expected: &events::RunArtifactEvidenceRef,
        role: ArtifactRole,
    ) -> Result<()> {
        let Some(actual) = artifacts.values().find(|actual| {
            actual.artifact_id == expected.artifact_id
                && actual.digest == expected.content_digest
                && actual.byte_len == expected.byte_len
                && actual.media_type == expected.media_type
                && actual.schema_id == expected.schema_id
                && actual.semantic_type_id == expected.semantic_type_id
                && actual.artifact_role == role
                && expected.role == role
        }) else {
            return Err(ReplayError::new(
                ReplayErrorKind::ArtifactMissing,
                format!(
                    "missing run admission artifact evidence {}",
                    expected.artifact_id
                ),
            ));
        };
        let _ = actual;
        Ok(())
    }

    fn admitted_binding_digest(
        runner_executables: &[events::ExecutableIdentity],
        adapter_executables: &[events::ExecutableIdentity],
    ) -> Result<ContentDigest> {
        let json = serde_json::to_string(&serde_json::json!({
            "adapter_executables": adapter_executables.iter().map(executable_identity_json).collect::<Vec<_>>(),
            "runner_executables": runner_executables.iter().map(executable_identity_json).collect::<Vec<_>>(),
        }))
        .map_err(|error| ReplayError::new(ReplayErrorKind::InvalidRunStream, error.to_string()))?;
        let canonical =
            mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&json).map_err(|error| {
                ReplayError::new(ReplayErrorKind::InvalidRunStream, error.to_string())
            })?;
        Ok(ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            canonical.digest_bytes(),
        ))
    }

    fn executable_identity_json(identity: &events::ExecutableIdentity) -> serde_json::Value {
        serde_json::json!({
            "binary_digest": identity.binary_digest.as_str(),
            "cargo_package_digest": identity.cargo_package_digest.as_str(),
            "factory_id": identity.factory_id.as_str(),
            "nix_derivation_hash": identity.nix_derivation_hash.as_ref().map(|value| value.as_str()),
            "nix_output_hash": identity.nix_output_hash.as_ref().map(|value| value.as_str()),
        })
    }

    fn capability_set_contains(
        capabilities: &CapabilitySetDescriptor,
        capability_kind: &CapabilityKind,
        capability_version: &CapabilityVersion,
    ) -> bool {
        capabilities.capabilities.iter().any(|capability| {
            capability.kind == *capability_kind && capability.version == *capability_version
        })
    }

    fn verify_replay_verifier(
        requested: Option<&events::ReplayVerifierId>,
        recorded: &events::ReplayVerifierId,
    ) -> Result<()> {
        match requested {
            Some(requested) if requested == recorded => Ok(()),
            _ => Err(ReplayError::new(
                ReplayErrorKind::ReplayVerifierMismatch,
                "side-effect replay verifier id does not match recorded evidence",
            )),
        }
    }

    fn side_effect_mismatch(message: &'static str) -> ReplayError {
        ReplayError::new(ReplayErrorKind::SideEffectMismatch, message)
    }

    fn verify_side_effect_submit_claim_identity(
        intent: &side_effect::IntentPersisted,
        node_id: &NodeId,
        attempt_id: &AttemptId,
        mismatch_message: &'static str,
    ) -> Result<()> {
        if intent.node_id != *node_id || intent.attempt_id != *attempt_id {
            return Err(side_effect_mismatch(mismatch_message));
        }
        Ok(())
    }

    fn insert_unique<K, V>(
        map: &mut BTreeMap<K, V>,
        key: K,
        value: V,
        kind: ReplayErrorKind,
        message: &'static str,
    ) -> Result<()>
    where
        K: Ord,
    {
        if map.insert(key, value).is_some() {
            return Err(ReplayError::new(kind, message));
        }
        Ok(())
    }

    fn spec_digest(spec_hash: &SpecHash) -> ContentDigest {
        ContentDigest::from_digest(spec_hash.algorithm(), *spec_hash.digest())
    }

    #[cfg(test)]
    mod tests;
}
