#![warn(missing_docs)]
//! Typed replay brokers and verifier contracts for MFM.
//!
//! Replay is intentionally evidence-only. A [`v1::ReplayBroker`] is built from a
//! certified typed execution spec, the authoritative store-owned run stream, and
//! retained artifact evidence. It never constructs transports, SDK clients,
//! live capability handles, old dynamic machine state, or generic IO providers.

/// Versioned v1 typed replay contracts.
pub mod v1 {
    use std::collections::BTreeMap;
    use std::fmt;

    use mfm_capabilities::CapabilitySetDescriptor;
    use mfm_events::v1::{self as events, side_effect, ArtifactRole, KernelEventPayload};
    use mfm_ids::{
        AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind, CapabilityVersion,
        ContentDigest, NodeId, SchemaId, SpecHash,
    };
    use mfm_spec::v1::{self as spec, CanonicalizerIdentity, CertifiedSpecEnvelope};
    use mfm_spec::SpecError;
    use mfm_store::v1::{
        self as store, ArtifactEvidenceRef as StoredArtifactEvidenceRef, KernelEventEnvelope,
        ProjectionSnapshot,
    };

    /// Result type for replay broker operations.
    pub type Result<T> = std::result::Result<T, ReplayError>;

    /// Typed replay validation error.
    #[derive(Debug, Clone, PartialEq, Eq)]
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

    impl fmt::Display for ReplayError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}: {}", self.code(), self.message)
        }
    }

    impl std::error::Error for ReplayError {}

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

    /// Stable typed replay error category.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum ReplayErrorKind {
        /// The certified spec envelope is invalid.
        CertifiedSpec,
        /// No authoritative run-start event was present.
        RunStartedMissing,
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
                Self::RunStartedMissing => "MFM_REPLAY_RUN_STARTED_MISSING",
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

    /// Replay authority supplied by the caller from retained run evidence.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ReplayAuthority {
        /// Canonicalizer identity expected for this replay.
        pub canonicalizer_identity: CanonicalizerIdentity,
        /// Runner executable identities retained for the run.
        pub runner_executables: Vec<events::ExecutableIdentity>,
        /// Adapter executable identities retained for the run.
        pub adapter_executables: Vec<events::ExecutableIdentity>,
        /// Retained artifact evidence available to replay brokers.
        pub artifact_evidence: Vec<StoredArtifactEvidenceRef>,
    }

    impl ReplayAuthority {
        /// Builds replay authority from retained executable and artifact evidence.
        pub fn new(
            canonicalizer_identity: CanonicalizerIdentity,
            runner_executables: Vec<events::ExecutableIdentity>,
            adapter_executables: Vec<events::ExecutableIdentity>,
            artifact_evidence: Vec<StoredArtifactEvidenceRef>,
        ) -> Self {
            Self {
                canonicalizer_identity,
                runner_executables,
                adapter_executables,
                artifact_evidence,
            }
        }

        /// Builds authority using the renderer canonicalizer embedded in the certified spec.
        pub fn from_certified_spec(
            certified_spec: &CertifiedSpecEnvelope,
            runner_executables: Vec<events::ExecutableIdentity>,
            adapter_executables: Vec<events::ExecutableIdentity>,
            artifact_evidence: Vec<StoredArtifactEvidenceRef>,
        ) -> Self {
            Self {
                canonicalizer_identity: certified_spec
                    .spec
                    .public_outputs
                    .renderer_descriptor
                    .canonicalizer_identity
                    .clone(),
                runner_executables,
                adapter_executables,
                artifact_evidence,
            }
        }
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

    /// Request for replaying side-effect submission, receipt, or confirmation evidence.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SideEffectEvidenceReplayRequest {
        /// Side-effect ledger key.
        pub ledger_key: events::SideEffectLedgerKey,
        /// Node id that owns the side effect.
        pub node_id: NodeId,
        /// Attempt id that owns the side effect.
        pub attempt_id: AttemptId,
        /// Invocation epoch to replay.
        pub invocation_epoch: u32,
        /// Intent schema id expected by replay.
        pub intent_schema_id: SchemaId,
        /// Canonical intent hash expected by replay.
        pub intent_hash: ContentDigest,
        /// Idempotency input schema id expected by replay.
        pub idempotency_input_schema_id: SchemaId,
        /// Canonical idempotency input hash expected by replay.
        pub idempotency_input_hash: ContentDigest,
        /// Capability kind expected by the replaying state.
        pub capability_kind: CapabilityKind,
        /// Capability version expected by the replaying state.
        pub capability_version: CapabilityVersion,
        /// Adapter kind expected by the replaying state.
        pub adapter_kind: AdapterKind,
        /// Adapter version expected by the replaying state.
        pub adapter_version: AdapterVersion,
        /// Evidence schema id expected by replay.
        pub evidence_schema_id: SchemaId,
        /// Canonical evidence hash expected by replay.
        pub evidence_hash: ContentDigest,
        /// Replay verifier required for receipt and confirmation evidence.
        pub replay_verifier_id: Option<events::ReplayVerifierId>,
    }

    /// Replay evidence returned for an observed side-effect submission.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SubmissionReplayEvidence {
        /// Submission observed event payload.
        pub submission: side_effect::SubmissionObserved,
        /// Retained submission artifact evidence.
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
        /// Expected producer node id, when node-produced.
        pub producer_node_id: Option<NodeId>,
    }

    type FactKey = (NodeId, AttemptId, events::FactKey);
    type SideEffectKey = (events::SideEffectLedgerKey, u32);

    /// Evidence-only broker for certified typed replay.
    #[derive(Debug, Clone)]
    pub struct ReplayBroker {
        certified_spec: CertifiedSpecEnvelope,
        run_id: events::RunStarted,
        projection: ProjectionSnapshot,
        retained_artifacts: BTreeMap<ArtifactId, StoredArtifactEvidenceRef>,
        artifacts: BTreeMap<ArtifactId, StoredArtifactEvidenceRef>,
        facts: BTreeMap<FactKey, events::FactRecorded>,
        intents: BTreeMap<SideEffectKey, side_effect::IntentPersisted>,
        submissions: BTreeMap<SideEffectKey, side_effect::SubmissionObserved>,
        receipts: BTreeMap<SideEffectKey, side_effect::ReceiptObserved>,
        confirmations: BTreeMap<SideEffectKey, side_effect::ConfirmationObserved>,
    }

    impl ReplayBroker {
        /// Builds a replay broker from the stored certified spec and authoritative run stream.
        pub fn from_run_stream(
            certified_spec: CertifiedSpecEnvelope,
            stream: &[KernelEventEnvelope],
            authority: ReplayAuthority,
        ) -> Result<Self> {
            certified_spec.verify_hash()?;
            ProjectionSnapshot::validate_run_stream(stream)?;
            let projection = ProjectionSnapshot::rebuild_from_run_stream(stream)?;
            let retained_artifacts = artifact_map(authority.artifact_evidence.clone())?;
            let run_started = run_started_payload(stream)?;

            if run_started.spec_hash != certified_spec.spec_hash {
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
                &run_started,
                &authority,
                &retained_artifacts,
            )?;

            let mut broker = Self {
                certified_spec,
                run_id: run_started,
                projection,
                retained_artifacts,
                artifacts: BTreeMap::new(),
                facts: BTreeMap::new(),
                intents: BTreeMap::new(),
                submissions: BTreeMap::new(),
                receipts: BTreeMap::new(),
                confirmations: BTreeMap::new(),
            };
            broker.authorize_certified_spec_artifacts()?;
            broker.index_stream(stream)?;
            Ok(broker)
        }

        /// Returns the certified spec used as replay authority.
        pub fn certified_spec(&self) -> &CertifiedSpecEnvelope {
            &self.certified_spec
        }

        /// Returns the run-start payload bound to this replay broker.
        pub fn run_started(&self) -> &events::RunStarted {
            &self.run_id
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
            self.verify_artifact(
                &request.artifact_id,
                &request.digest,
                request.schema_id.as_ref(),
                request.role,
                request.producer_node_id.as_ref(),
            )
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
                artifact: self.verify_artifact(
                    &fact.artifact_id,
                    &fact.response_hash,
                    Some(&fact.response_schema_id),
                    ArtifactRole::FactResponse,
                    Some(&fact.node_id),
                )?,
            })
        }

        /// Returns observed side-effect submission evidence from replay records only.
        pub fn side_effect_submission(
            &self,
            request: &SideEffectEvidenceReplayRequest,
        ) -> Result<SubmissionReplayEvidence> {
            self.verify_side_effect_intent(request)?;
            let submission = self
                .submissions
                .get(&(request.ledger_key.clone(), request.invocation_epoch))
                .ok_or_else(|| {
                    ReplayError::new(
                        ReplayErrorKind::SideEffectMissing,
                        format!("missing side-effect submission {}", request.ledger_key),
                    )
                })?;
            if submission.submission_schema_id != request.evidence_schema_id
                || submission.submission_hash != request.evidence_hash
            {
                return Err(side_effect_mismatch("submission evidence mismatch"));
            }
            Ok(SubmissionReplayEvidence {
                submission: submission.clone(),
                artifact: self.verify_artifact(
                    &submission.submission_artifact_id,
                    &submission.submission_hash,
                    Some(&submission.submission_schema_id),
                    ArtifactRole::Submission,
                    Some(&submission.node_id),
                )?,
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
            let receipt = self
                .receipts
                .get(&(request.ledger_key.clone(), request.invocation_epoch))
                .ok_or_else(|| {
                    ReplayError::new(
                        ReplayErrorKind::SideEffectMissing,
                        format!("missing side-effect receipt {}", request.ledger_key),
                    )
                })?;
            verify_replay_verifier(
                request.replay_verifier_id.as_ref(),
                &receipt.replay_verifier_id,
            )?;
            if receipt.receipt_schema_id != request.evidence_schema_id
                || receipt.receipt_hash != request.evidence_hash
            {
                return Err(side_effect_mismatch("receipt evidence mismatch"));
            }
            Ok(ReceiptReplayEvidence {
                receipt: receipt.clone(),
                artifact: self.verify_artifact(
                    &receipt.receipt_artifact_id,
                    &receipt.receipt_hash,
                    Some(&receipt.receipt_schema_id),
                    ArtifactRole::Receipt,
                    Some(&receipt.node_id),
                )?,
            })
        }

        /// Returns observed side-effect confirmation evidence from replay records only.
        pub fn side_effect_confirmation(
            &self,
            request: &SideEffectEvidenceReplayRequest,
        ) -> Result<ConfirmationReplayEvidence> {
            self.verify_side_effect_intent(request)?;
            let confirmation = self
                .confirmations
                .get(&(request.ledger_key.clone(), request.invocation_epoch))
                .ok_or_else(|| {
                    ReplayError::new(
                        ReplayErrorKind::SideEffectMissing,
                        format!("missing side-effect confirmation {}", request.ledger_key),
                    )
                })?;
            verify_replay_verifier(
                request.replay_verifier_id.as_ref(),
                &confirmation.replay_verifier_id,
            )?;
            if confirmation.confirmation_schema_id != request.evidence_schema_id
                || confirmation.confirmation_hash != request.evidence_hash
            {
                return Err(side_effect_mismatch("confirmation evidence mismatch"));
            }
            Ok(ConfirmationReplayEvidence {
                confirmation: confirmation.clone(),
                artifact: self.verify_artifact(
                    &confirmation.confirmation_artifact_id,
                    &confirmation.confirmation_hash,
                    Some(&confirmation.confirmation_schema_id),
                    ArtifactRole::Confirmation,
                    Some(&confirmation.node_id),
                )?,
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
                self.authorize_artifact(
                    &config.artifact_id,
                    &config.digest,
                    Some(&config.schema_id),
                    ArtifactRole::TypedConfig,
                    None,
                )?;
            }
            Ok(())
        }

        fn index_stream(&mut self, stream: &[KernelEventEnvelope]) -> Result<()> {
            for envelope in stream {
                match envelope.payload() {
                    KernelEventPayload::RunStarted(payload) => {
                        self.authorize_artifact(
                            &payload.spec_artifact_id,
                            &spec_digest(&self.certified_spec.spec_hash),
                            None,
                            ArtifactRole::TypedExecutionSpec,
                            None,
                        )?;
                        for seed in &payload.seed_cells {
                            self.verify_seed_against_spec(seed)?;
                            self.authorize_event_artifact_ref(
                                &seed.seed_artifact,
                                None,
                                Some(&seed.seed_id),
                            )?;
                        }
                    }
                    KernelEventPayload::StateAttemptStarted(payload) => {
                        self.verify_state_attempt_started_against_spec(payload)?;
                    }
                    KernelEventPayload::FactRecorded(payload) => {
                        self.verify_fact_against_spec(payload)?;
                        self.authorize_artifact(
                            &payload.artifact_id,
                            &payload.response_hash,
                            Some(&payload.response_schema_id),
                            ArtifactRole::FactResponse,
                            Some(&payload.node_id),
                        )?;
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
                        self.authorize_event_artifact_ref(
                            &payload.artifact_ref,
                            payload.node_id.as_ref(),
                            None,
                        )?;
                    }
                    KernelEventPayload::SideEffectIntentPersisted(payload) => {
                        self.verify_side_effect_intent_against_spec(payload)?;
                        self.authorize_artifact(
                            &payload.intent_artifact_id,
                            &payload.intent_hash,
                            Some(&payload.intent_schema_id),
                            ArtifactRole::SideEffectIntent,
                            Some(&payload.node_id),
                        )?;
                        insert_unique(
                            &mut self.intents,
                            (payload.ledger_key.clone(), payload.invocation_epoch),
                            payload.clone(),
                            ReplayErrorKind::InvalidRunStream,
                            "duplicate side-effect intent replay event",
                        )?;
                    }
                    KernelEventPayload::SideEffectClaimed(payload) => {
                        self.verify_side_effect_event_against_intent(
                            &payload.ledger_key,
                            payload.invocation_epoch,
                            &payload.node_id,
                            &payload.attempt_id,
                        )?;
                    }
                    KernelEventPayload::SideEffectClaimTakenOver(payload) => {
                        self.verify_side_effect_event_against_intent(
                            &payload.ledger_key,
                            payload.invocation_epoch,
                            &payload.node_id,
                            &payload.attempt_id,
                        )?;
                    }
                    KernelEventPayload::SideEffectInvocationStarted(payload) => {
                        self.verify_side_effect_event_against_intent(
                            &payload.ledger_key,
                            payload.invocation_epoch,
                            &payload.node_id,
                            &payload.attempt_id,
                        )?;
                    }
                    KernelEventPayload::SideEffectSubmissionObserved(payload) => {
                        self.verify_side_effect_event_against_intent(
                            &payload.ledger_key,
                            payload.invocation_epoch,
                            &payload.node_id,
                            &payload.attempt_id,
                        )?;
                        self.authorize_artifact(
                            &payload.submission_artifact_id,
                            &payload.submission_hash,
                            Some(&payload.submission_schema_id),
                            ArtifactRole::Submission,
                            Some(&payload.node_id),
                        )?;
                        insert_unique(
                            &mut self.submissions,
                            (payload.ledger_key.clone(), payload.invocation_epoch),
                            payload.clone(),
                            ReplayErrorKind::InvalidRunStream,
                            "duplicate side-effect submission replay event",
                        )?;
                    }
                    KernelEventPayload::SideEffectReceiptObserved(payload) => {
                        self.verify_side_effect_event_against_intent(
                            &payload.ledger_key,
                            payload.invocation_epoch,
                            &payload.node_id,
                            &payload.attempt_id,
                        )?;
                        self.authorize_artifact(
                            &payload.receipt_artifact_id,
                            &payload.receipt_hash,
                            Some(&payload.receipt_schema_id),
                            ArtifactRole::Receipt,
                            Some(&payload.node_id),
                        )?;
                        insert_unique(
                            &mut self.receipts,
                            (payload.ledger_key.clone(), payload.invocation_epoch),
                            payload.clone(),
                            ReplayErrorKind::InvalidRunStream,
                            "duplicate side-effect receipt replay event",
                        )?;
                    }
                    KernelEventPayload::SideEffectConfirmationObserved(payload) => {
                        self.verify_side_effect_event_against_intent(
                            &payload.ledger_key,
                            payload.invocation_epoch,
                            &payload.node_id,
                            &payload.attempt_id,
                        )?;
                        self.authorize_artifact(
                            &payload.confirmation_artifact_id,
                            &payload.confirmation_hash,
                            Some(&payload.confirmation_schema_id),
                            ArtifactRole::Confirmation,
                            Some(&payload.node_id),
                        )?;
                        insert_unique(
                            &mut self.confirmations,
                            (payload.ledger_key.clone(), payload.invocation_epoch),
                            payload.clone(),
                            ReplayErrorKind::InvalidRunStream,
                            "duplicate side-effect confirmation replay event",
                        )?;
                    }
                    KernelEventPayload::SideEffectInvocationPrepared(payload) => {
                        self.verify_side_effect_event_against_intent(
                            &payload.ledger_key,
                            payload.invocation_epoch,
                            &payload.node_id,
                            &payload.attempt_id,
                        )?;
                        if let (Some(artifact_id), Some(hash)) =
                            (&payload.prepared_artifact_id, &payload.prepared_hash)
                        {
                            self.authorize_artifact(
                                artifact_id,
                                hash,
                                None,
                                ArtifactRole::PreparedInvocation,
                                Some(&payload.node_id),
                            )?;
                        }
                    }
                    KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
                        self.verify_side_effect_event_against_intent(
                            &payload.ledger_key,
                            payload.invocation_epoch,
                            &payload.node_id,
                            &payload.attempt_id,
                        )?;
                        self.authorize_artifact(
                            &payload.proof_artifact_id,
                            &payload.proof_hash,
                            Some(&payload.proof_schema_id),
                            ArtifactRole::NotSubmittedProof,
                            Some(&payload.node_id),
                        )?;
                    }
                    KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
                        self.verify_side_effect_event_against_intent(
                            &payload.ledger_key,
                            payload.invocation_epoch,
                            &payload.node_id,
                            &payload.attempt_id,
                        )?;
                        self.authorize_artifact(
                            &payload.evidence_artifact_id,
                            &payload.evidence_hash,
                            Some(&payload.evidence_schema_id),
                            ArtifactRole::SubmissionUnknownEvidence,
                            Some(&payload.node_id),
                        )?;
                    }
                    KernelEventPayload::SideEffectAmbiguous(payload) => {
                        self.verify_side_effect_event_against_intent(
                            &payload.ledger_key,
                            payload.invocation_epoch,
                            &payload.node_id,
                            &payload.attempt_id,
                        )?;
                        self.authorize_artifact(
                            &payload.evidence_artifact_id,
                            &payload.evidence_hash,
                            Some(&payload.evidence_schema_id),
                            ArtifactRole::AmbiguityEvidence,
                            Some(&payload.node_id),
                        )?;
                    }
                    KernelEventPayload::CellProduced(payload) => {
                        self.verify_cell_produced_against_spec(payload)?;
                        self.authorize_artifact(
                            &payload.artifact_id,
                            &payload.content_digest,
                            Some(&payload.schema_id),
                            ArtifactRole::StateOutput,
                            Some(&payload.node_id),
                        )?;
                    }
                    KernelEventPayload::CellSkipped(payload) => {
                        self.verify_cell_skipped_against_spec(payload)?;
                    }
                    KernelEventPayload::PublicOutputProduced(payload) => {
                        self.verify_public_output_against_spec(payload)?;
                        if let Some(artifact_id) = &payload.rendered_artifact_id {
                            self.authorize_artifact(
                                artifact_id,
                                &payload.rendered_digest,
                                Some(&payload.public_schema_id),
                                ArtifactRole::PublicOutput,
                                Some(&payload.node_id),
                            )?;
                        }
                    }
                    KernelEventPayload::PublicOutputRenderFailed(payload) => {
                        self.verify_public_output_render_failure_against_spec(payload)?;
                        if let Some(evidence) = &payload.error.diagnostic_ref {
                            self.authorize_event_artifact_ref(
                                evidence,
                                Some(&payload.node_id),
                                None,
                            )?;
                        }
                    }
                    KernelEventPayload::StateAttemptFailed(payload) => {
                        self.node(&payload.node_id)?;
                        if let Some(evidence) = &payload.error.diagnostic_ref {
                            self.authorize_event_artifact_ref(
                                evidence,
                                Some(&payload.node_id),
                                None,
                            )?;
                        }
                    }
                    KernelEventPayload::StateAttemptCompleted(payload) => {
                        self.verify_state_attempt_completed_against_spec(payload)?;
                    }
                    KernelEventPayload::RunCompleted(payload) => match &payload.outcome {
                        events::RunCompletionOutcome::Completed(evidence) => {
                            self.verify_completed_run_public_output(evidence)?;
                        }
                        events::RunCompletionOutcome::Failed(error)
                        | events::RunCompletionOutcome::Cancelled(error) => {
                            if let Some(evidence) = &error.diagnostic_ref {
                                self.authorize_event_artifact_ref(evidence, None, None)?;
                            }
                        }
                    },
                    KernelEventPayload::RetentionManifestProjected(payload) => {
                        self.authorize_artifact(
                            &payload.manifest_artifact_id,
                            &payload.manifest_digest,
                            None,
                            ArtifactRole::RetentionManifest,
                            None,
                        )?;
                    }
                    KernelEventPayload::RetentionRefsAppended(payload) => {
                        for retention in &payload.refs {
                            self.authorize_artifact_partial(
                                &retention.artifact_id,
                                &retention.content_digest,
                                retention.role,
                            )?;
                        }
                    }
                    KernelEventPayload::SideEffectFailed(payload) => {
                        self.verify_side_effect_event_against_intent(
                            &payload.ledger_key,
                            payload.invocation_epoch,
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
            let intent = self
                .intents
                .get(&(request.ledger_key.clone(), request.invocation_epoch))
                .ok_or_else(|| {
                    ReplayError::new(
                        ReplayErrorKind::SideEffectMissing,
                        format!("missing side-effect intent {}", request.ledger_key),
                    )
                })?;
            Ok(SideEffectIntentReplayEvidence {
                intent: intent.clone(),
                artifact: self.verify_artifact(
                    &intent.intent_artifact_id,
                    &intent.intent_hash,
                    Some(&intent.intent_schema_id),
                    ArtifactRole::SideEffectIntent,
                    Some(&intent.node_id),
                )?,
            })
        }

        fn side_effect_submission_for(
            &self,
            request: &SideEffectEvidenceReplayRequest,
        ) -> Result<Option<SubmissionReplayEvidence>> {
            let Some(submission) = self
                .submissions
                .get(&(request.ledger_key.clone(), request.invocation_epoch))
            else {
                return Ok(None);
            };
            Ok(Some(SubmissionReplayEvidence {
                submission: submission.clone(),
                artifact: self.verify_artifact(
                    &submission.submission_artifact_id,
                    &submission.submission_hash,
                    Some(&submission.submission_schema_id),
                    ArtifactRole::Submission,
                    Some(&submission.node_id),
                )?,
            }))
        }

        fn side_effect_receipt_for(
            &self,
            request: &SideEffectEvidenceReplayRequest,
        ) -> Result<Option<ReceiptReplayEvidence>> {
            let Some(receipt) = self
                .receipts
                .get(&(request.ledger_key.clone(), request.invocation_epoch))
            else {
                return Ok(None);
            };
            Ok(Some(ReceiptReplayEvidence {
                receipt: receipt.clone(),
                artifact: self.verify_artifact(
                    &receipt.receipt_artifact_id,
                    &receipt.receipt_hash,
                    Some(&receipt.receipt_schema_id),
                    ArtifactRole::Receipt,
                    Some(&receipt.node_id),
                )?,
            }))
        }

        fn verify_side_effect_intent(
            &self,
            request: &SideEffectEvidenceReplayRequest,
        ) -> Result<()> {
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
            let intent = self
                .intents
                .get(&(request.ledger_key.clone(), request.invocation_epoch))
                .ok_or_else(|| {
                    ReplayError::new(
                        ReplayErrorKind::SideEffectMissing,
                        format!("missing side-effect intent {}", request.ledger_key),
                    )
                })?;
            if intent.node_id != request.node_id
                || intent.attempt_id != request.attempt_id
                || intent.intent_schema_id != request.intent_schema_id
                || intent.intent_hash != request.intent_hash
                || intent.idempotency_input_schema_id != request.idempotency_input_schema_id
                || intent.idempotency_input_hash != request.idempotency_input_hash
                || intent.capability_kind != request.capability_kind
                || intent.capability_version != request.capability_version
                || intent.adapter_kind != request.adapter_kind
                || intent.adapter_version != request.adapter_version
            {
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

        fn verify_side_effect_event_against_intent(
            &self,
            ledger_key: &events::SideEffectLedgerKey,
            invocation_epoch: u32,
            node_id: &NodeId,
            attempt_id: &AttemptId,
        ) -> Result<()> {
            let intent = self
                .intents
                .get(&(ledger_key.clone(), invocation_epoch))
                .ok_or_else(|| {
                    ReplayError::new(
                        ReplayErrorKind::SideEffectMissing,
                        format!("missing side-effect intent {ledger_key}"),
                    )
                })?;
            if intent.node_id != *node_id || intent.attempt_id != *attempt_id {
                return Err(side_effect_mismatch(
                    "side-effect event does not match persisted intent",
                ));
            }
            Ok(())
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

        fn authorize_event_artifact_ref(
            &mut self,
            event_ref: &events::ArtifactEvidenceRef,
            producer_node_id: Option<&NodeId>,
            producer_seed_id: Option<&mfm_ids::SeedId>,
        ) -> Result<StoredArtifactEvidenceRef> {
            let evidence = self.authorize_artifact(
                &event_ref.artifact_id,
                &event_ref.content_digest,
                Some(&event_ref.schema_id),
                event_ref.role,
                producer_node_id,
            )?;
            if evidence.byte_len != event_ref.byte_len
                || evidence.media_type != event_ref.media_type
                || evidence.semantic_type_id != event_ref.semantic_type_id
                || producer_seed_id.is_some()
                    && evidence.producer_seed_id.as_ref() != producer_seed_id
            {
                return Err(ReplayError::new(
                    ReplayErrorKind::ArtifactMismatch,
                    format!("artifact evidence mismatch for {}", event_ref.artifact_id),
                ));
            }
            Ok(evidence)
        }

        fn authorize_artifact_partial(
            &mut self,
            artifact_id: &ArtifactId,
            digest: &ContentDigest,
            role: ArtifactRole,
        ) -> Result<StoredArtifactEvidenceRef> {
            let evidence = self.retained_artifacts.get(artifact_id).ok_or_else(|| {
                ReplayError::new(
                    ReplayErrorKind::ArtifactMissing,
                    format!("missing retained artifact evidence for {artifact_id}"),
                )
            })?;
            if &evidence.digest != digest || evidence.artifact_role != role {
                return Err(ReplayError::new(
                    ReplayErrorKind::ArtifactMismatch,
                    format!("retained artifact evidence mismatch for {artifact_id}"),
                ));
            }
            let evidence = evidence.clone();
            self.insert_authorized_artifact(evidence.clone())?;
            Ok(evidence)
        }

        fn authorize_artifact(
            &mut self,
            artifact_id: &ArtifactId,
            digest: &ContentDigest,
            schema_id: Option<&SchemaId>,
            role: ArtifactRole,
            producer_node_id: Option<&NodeId>,
        ) -> Result<StoredArtifactEvidenceRef> {
            let evidence = self.retained_artifacts.get(artifact_id).ok_or_else(|| {
                ReplayError::new(
                    ReplayErrorKind::ArtifactMissing,
                    format!("missing retained artifact evidence for {artifact_id}"),
                )
            })?;
            verify_artifact_fields(evidence, digest, schema_id, role, producer_node_id)?;
            let evidence = evidence.clone();
            self.insert_authorized_artifact(evidence.clone())?;
            Ok(evidence)
        }

        fn insert_authorized_artifact(
            &mut self,
            evidence: StoredArtifactEvidenceRef,
        ) -> Result<()> {
            if let Some(existing) = self.artifacts.get(&evidence.artifact_id) {
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
            self.artifacts
                .insert(evidence.artifact_id.clone(), evidence);
            Ok(())
        }

        fn verify_artifact(
            &self,
            artifact_id: &ArtifactId,
            digest: &ContentDigest,
            schema_id: Option<&SchemaId>,
            role: ArtifactRole,
            producer_node_id: Option<&NodeId>,
        ) -> Result<StoredArtifactEvidenceRef> {
            let evidence = self.artifacts.get(artifact_id).ok_or_else(|| {
                ReplayError::new(
                    ReplayErrorKind::ArtifactMissing,
                    format!("missing replay-authorized artifact evidence for {artifact_id}"),
                )
            })?;
            verify_artifact_fields(evidence, digest, schema_id, role, producer_node_id)?;
            Ok(evidence.clone())
        }
    }

    fn artifact_map(
        artifacts: Vec<StoredArtifactEvidenceRef>,
    ) -> Result<BTreeMap<ArtifactId, StoredArtifactEvidenceRef>> {
        let mut map = BTreeMap::new();
        for artifact in artifacts {
            if let Some(existing) = map.get(&artifact.artifact_id) {
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
                map.insert(artifact.artifact_id.clone(), artifact);
            }
        }
        Ok(map)
    }

    fn verify_artifact_fields(
        evidence: &StoredArtifactEvidenceRef,
        digest: &ContentDigest,
        schema_id: Option<&SchemaId>,
        role: ArtifactRole,
        producer_node_id: Option<&NodeId>,
    ) -> Result<()> {
        if &evidence.digest != digest
            || evidence.schema_id.as_ref() != schema_id
            || evidence.artifact_role != role
            || producer_node_id.is_some() && evidence.producer_node_id.as_ref() != producer_node_id
        {
            return Err(ReplayError::new(
                ReplayErrorKind::ArtifactMismatch,
                format!(
                    "retained artifact evidence mismatch for {}",
                    evidence.artifact_id
                ),
            ));
        }
        Ok(())
    }

    fn certified_evidence_mismatch(message: &'static str) -> ReplayError {
        ReplayError::new(ReplayErrorKind::CertifiedEvidenceMismatch, message)
    }

    fn run_started_payload(stream: &[KernelEventEnvelope]) -> Result<events::RunStarted> {
        let mut run_started = None;
        for envelope in stream {
            if let KernelEventPayload::RunStarted(payload) = envelope.payload() {
                if run_started.is_some() {
                    return Err(ReplayError::new(
                        ReplayErrorKind::InvalidRunStream,
                        "run stream contains more than one RunStarted event",
                    ));
                }
                run_started = Some(payload.clone());
            }
        }
        run_started.ok_or_else(|| {
            ReplayError::new(
                ReplayErrorKind::RunStartedMissing,
                "run stream contains no RunStarted event",
            )
        })
    }

    fn verify_run_start_contract(
        certified_spec: &CertifiedSpecEnvelope,
        run_started: &events::RunStarted,
        authority: &ReplayAuthority,
        artifacts: &BTreeMap<ArtifactId, StoredArtifactEvidenceRef>,
    ) -> Result<()> {
        if run_started.spec_version != certified_spec.spec.spec_version
            || run_started.spec_media_type != certified_spec.spec.media_type
            || run_started.lowering_version != certified_spec.spec.lowering_version
            || run_started.public_output_schema_id
                != certified_spec.spec.public_outputs.public_schema_id
        {
            return Err(ReplayError::new(
                ReplayErrorKind::CertifiedSpec,
                "run-start spec contract fields do not match certified spec",
            ));
        }
        if run_started.descriptor_identities != certified_spec.spec.descriptor_identities {
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
        if run_started.canonicalizer_identity != *renderer_canonicalizer
            || authority.canonicalizer_identity != *renderer_canonicalizer
        {
            return Err(ReplayError::new(
                ReplayErrorKind::CanonicalizerMismatch,
                "run-start canonicalizer identity does not match replay authority",
            ));
        }
        if run_started.runner_executables != authority.runner_executables {
            return Err(ReplayError::new(
                ReplayErrorKind::ExecutableIdentityMismatch,
                "runner executable identities do not match replay authority",
            ));
        }
        if run_started.adapter_executables != authority.adapter_executables {
            return Err(ReplayError::new(
                ReplayErrorKind::AdapterExecutableMismatch,
                "adapter executable identities do not match replay authority",
            ));
        }
        let Some(spec_artifact) = artifacts.get(&run_started.spec_artifact_id) else {
            return Err(ReplayError::new(
                ReplayErrorKind::ArtifactMissing,
                "missing certified spec artifact evidence",
            ));
        };
        if spec_artifact.digest != spec_digest(&certified_spec.spec_hash)
            || spec_artifact.artifact_role != ArtifactRole::TypedExecutionSpec
            || spec_artifact.schema_id.is_some()
        {
            return Err(ReplayError::new(
                ReplayErrorKind::ArtifactMismatch,
                "certified spec artifact evidence does not match run start",
            ));
        }
        Ok(())
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
    mod tests {
        use super::*;
        use mfm_capabilities::{CapabilityDescriptor, CapabilityRole, CapabilitySetDescriptor};
        use mfm_ids::{
            DescriptorId, DigestAlgorithm, DigestBytes, EffectKind, EffectVersion, LoweringVersion,
            RunId, ScopeId, SeedId, SemanticTypeId, SpecVersion, StateKind, StateVersion,
        };
        use mfm_store::v1::{
            build_committed_batch, CommitKey, CommitPreconditions, InMemoryTypedRunStore,
            RequiredRunState, StreamSeq, TypedCommitRequest, TypedRunEventStore,
        };

        const SPEC_MEDIA_TYPE: &str = "application/vnd.mfm.typed-execution-spec+json;version=1";

        #[test]
        fn replay_broker_returns_recorded_fact_and_side_effect_evidence() {
            let fixture = Fixture::new();
            let broker = fixture.broker();

            let fact = broker
                .recorded_fact(&fixture.fact_request())
                .expect("fact replay");
            assert_eq!(fact.fact.response_hash, fixture.fact_response_hash);
            assert_eq!(fact.artifact.artifact_role, ArtifactRole::FactResponse);

            let submission = broker
                .side_effect_submission(&fixture.submission_request())
                .expect("submission replay");
            assert_eq!(
                submission.submission.submission_hash,
                fixture.submission_hash
            );

            let receipt = broker
                .side_effect_receipt(&fixture.receipt_request())
                .expect("receipt replay");
            assert_eq!(receipt.receipt.receipt_hash, fixture.receipt_hash);

            let confirmation = broker
                .side_effect_confirmation(&fixture.confirmation_request())
                .expect("confirmation replay");
            assert_eq!(
                confirmation.confirmation.confirmation_hash,
                fixture.confirmation_hash
            );

            let verifier = TestReplayVerifier {
                verifier_id: events::ReplayVerifierId::new("verifier-1").expect("verifier"),
                submission_hash: fixture.submission_hash.clone(),
                receipt_hash: fixture.receipt_hash.clone(),
                confirmation_hash: fixture.confirmation_hash.clone(),
            };
            let verified_submission = broker
                .verify_side_effect_submission(&fixture.submission_request(), &verifier)
                .expect("verified submission");
            assert_eq!(
                verified_submission.submission.submission_hash,
                fixture.submission_hash
            );
            let verified_receipt = broker
                .verify_side_effect_receipt(&fixture.receipt_request(), &verifier)
                .expect("verified receipt");
            assert_eq!(verified_receipt.receipt.receipt_hash, fixture.receipt_hash);
            let verified_confirmation = broker
                .verify_side_effect_confirmation(&fixture.confirmation_request(), &verifier)
                .expect("verified confirmation");
            assert_eq!(
                verified_confirmation.confirmation.confirmation_hash,
                fixture.confirmation_hash
            );
        }

        #[test]
        fn replay_rejects_missing_fact_and_mismatched_fact_hash() {
            let fixture = Fixture::new();
            let broker = fixture.broker();

            let mut missing = fixture.fact_request();
            missing.fact_key = events::FactKey::new("missing-fact").expect("fact key");
            assert_eq!(
                broker
                    .recorded_fact(&missing)
                    .expect_err("missing fact")
                    .kind,
                ReplayErrorKind::FactMissing
            );

            let mut mismatched = fixture.fact_request();
            mismatched.request_hash = content(0xfa);
            assert_eq!(
                broker
                    .recorded_fact(&mismatched)
                    .expect_err("fact mismatch")
                    .kind,
                ReplayErrorKind::FactMismatch
            );
        }

        #[test]
        fn replay_rejects_unsupported_adapter_and_live_capability() {
            let fixture = Fixture::new();
            let broker = fixture.broker();

            let mut request = fixture.fact_request();
            request.adapter_version =
                AdapterVersion::new("mfm.test.adapter.v2").expect("adapter version");
            assert_eq!(
                broker
                    .recorded_fact(&request)
                    .expect_err("unsupported adapter")
                    .kind,
                ReplayErrorKind::UnsupportedAdapter
            );

            assert_eq!(
                broker
                    .reject_live_capability_request()
                    .expect_err("live caps rejected")
                    .kind,
                ReplayErrorKind::LiveCapabilityRequest
            );
        }

        #[test]
        fn replay_rejects_executable_and_canonicalizer_mismatch() {
            let fixture = Fixture::new();
            let mut authority = fixture.authority();
            authority.runner_executables = vec![executable("different-runner")];
            assert_eq!(
                ReplayBroker::from_run_stream(
                    fixture.envelope.clone(),
                    &fixture.stream,
                    authority,
                )
                .expect_err("runner mismatch")
                .kind,
                ReplayErrorKind::ExecutableIdentityMismatch
            );

            let mut authority = fixture.authority();
            authority.canonicalizer_identity =
                CanonicalizerIdentity::new("different-canonicalizer").expect("canonicalizer");
            assert_eq!(
                ReplayBroker::from_run_stream(
                    fixture.envelope.clone(),
                    &fixture.stream,
                    authority,
                )
                .expect_err("canonicalizer mismatch")
                .kind,
                ReplayErrorKind::CanonicalizerMismatch
            );
        }

        #[test]
        fn replay_rejects_receipt_verifier_or_artifact_mismatch() {
            let fixture = Fixture::new();
            let broker = fixture.broker();

            let mut request = fixture.receipt_request();
            request.replay_verifier_id =
                Some(events::ReplayVerifierId::new("wrong-verifier").expect("verifier"));
            assert_eq!(
                broker
                    .side_effect_receipt(&request)
                    .expect_err("verifier mismatch")
                    .kind,
                ReplayErrorKind::ReplayVerifierMismatch
            );

            let mut authority = fixture.authority();
            let receipt_artifact = authority
                .artifact_evidence
                .iter_mut()
                .find(|artifact| artifact.artifact_id == fixture.receipt_artifact)
                .expect("receipt artifact");
            receipt_artifact.digest = content(0xfb);
            assert_eq!(
                ReplayBroker::from_run_stream(
                    fixture.envelope.clone(),
                    &fixture.stream,
                    authority,
                )
                .expect_err("artifact mismatch")
                .kind,
                ReplayErrorKind::ArtifactMismatch
            );
        }

        #[test]
        fn replay_rejects_missing_artifact_evidence() {
            let fixture = Fixture::new();
            let mut authority = fixture.authority();
            authority
                .artifact_evidence
                .retain(|artifact| artifact.artifact_id != fixture.fact_artifact);
            assert_eq!(
                ReplayBroker::from_run_stream(
                    fixture.envelope.clone(),
                    &fixture.stream,
                    authority,
                )
                .expect_err("missing artifact")
                .kind,
                ReplayErrorKind::ArtifactMissing
            );
        }

        #[test]
        fn replay_rejects_extra_artifacts_not_referenced_by_spec_or_stream() {
            let fixture = Fixture::new();
            let mut authority = fixture.authority();
            let extra_artifact = stored_artifact(
                artifact(0xee),
                content(0xef),
                Some(schema("mfm.test.extra", 0xf0)),
                ArtifactRole::FactResponse,
                Some(fixture.node_id.clone()),
            );
            authority.artifact_evidence.push(extra_artifact.clone());
            let broker =
                ReplayBroker::from_run_stream(fixture.envelope.clone(), &fixture.stream, authority)
                    .expect("broker");

            assert_eq!(
                broker
                    .artifact(&ArtifactReplayRequest {
                        artifact_id: extra_artifact.artifact_id,
                        role: ArtifactRole::FactResponse,
                        digest: extra_artifact.digest,
                        schema_id: extra_artifact.schema_id,
                        producer_node_id: Some(fixture.node_id.clone()),
                    })
                    .expect_err("unauthorized artifact")
                    .kind,
                ReplayErrorKind::ArtifactMissing
            );
        }

        #[test]
        fn replay_construction_rejects_uncertified_fact_capability() {
            let fixture = Fixture::new();
            let mut stream = fixture.stream.clone();
            let fact_index = stream
                .iter()
                .position(|event| matches!(event.payload(), KernelEventPayload::FactRecorded(_)))
                .expect("fact event");
            let KernelEventPayload::FactRecorded(mut fact) = stream[fact_index].payload().clone()
            else {
                unreachable!("fact event")
            };
            fact.capability_kind = CapabilityKind::new(
                "mfm.test",
                "rogue-capability",
                DigestAlgorithm::Sha256JcsV1,
                bytes(0xf1),
            )
            .expect("rogue capability");
            let request = TypedCommitRequest {
                run_id: stream[fact_index].run_id().clone(),
                expected_next_seq: stream[fact_index].seq(),
                commit_key: CommitKey::new("bad-fact-replay").expect("commit key"),
                payloads: vec![KernelEventPayload::FactRecorded(fact)],
                required_artifacts: Vec::new(),
                preconditions: CommitPreconditions::default(),
            };
            let batch = build_committed_batch(&request, stream[fact_index].seq())
                .expect("rebuild fact envelope");
            stream[fact_index] = batch.events()[0].clone();

            assert_eq!(
                ReplayBroker::from_run_stream(
                    fixture.envelope.clone(),
                    &stream,
                    fixture.authority(),
                )
                .expect_err("uncertified fact capability")
                .kind,
                ReplayErrorKind::UnsupportedCapability
            );
        }

        #[test]
        fn replay_construction_rejects_uncertified_public_output_evidence() {
            let fixture = Fixture::new();
            let mut stream = fixture.stream.clone();
            let node = fixture.envelope.spec.nodes[0].clone();
            append_payload_to_stream(
                &mut stream,
                "bad-public-output",
                KernelEventPayload::PublicOutputProduced(events::PublicOutputProduced {
                    spec_hash: fixture.envelope.spec_hash.clone(),
                    node_id: node.node_id,
                    attempt_id: fixture.attempt_id.clone(),
                    receipt_cell_id: node.output_cell,
                    public_schema_id: fixture
                        .envelope
                        .spec
                        .public_outputs
                        .public_schema_id
                        .clone(),
                    output_spec_digest: content(0xf2),
                    cells: Vec::new(),
                    rendered_digest: content(0xf3),
                    rendered_artifact_id: None,
                    renderer_descriptor_id: fixture
                        .envelope
                        .spec
                        .public_outputs
                        .renderer_descriptor
                        .descriptor_id
                        .clone(),
                }),
            );

            assert_eq!(
                ReplayBroker::from_run_stream(
                    fixture.envelope.clone(),
                    &stream,
                    fixture.authority(),
                )
                .expect_err("uncertified public output")
                .kind,
                ReplayErrorKind::CertifiedEvidenceMismatch
            );
        }

        #[test]
        fn replay_construction_rejects_completed_run_without_public_output_projection() {
            let fixture = Fixture::new();
            let mut stream = fixture.stream.clone();
            let run_id = stream[0].run_id().clone();
            let public_output_event_id = stream[0].event_id().clone();
            append_payload_to_stream(
                &mut stream,
                "bad-run-complete",
                KernelEventPayload::RunCompleted(events::RunCompleted {
                    run_id,
                    spec_hash: fixture.envelope.spec_hash.clone(),
                    outcome: events::RunCompletionOutcome::Completed(
                        events::PublicOutputCompletionEvidence {
                            public_output_schema_id: fixture
                                .envelope
                                .spec
                                .public_outputs
                                .public_schema_id
                                .clone(),
                            public_output_event_id,
                        },
                    ),
                }),
            );

            assert_eq!(
                ReplayBroker::from_run_stream(
                    fixture.envelope.clone(),
                    &stream,
                    fixture.authority(),
                )
                .expect_err("missing public output projection")
                .kind,
                ReplayErrorKind::CertifiedEvidenceMismatch
            );
        }

        struct TestReplayVerifier {
            verifier_id: events::ReplayVerifierId,
            submission_hash: ContentDigest,
            receipt_hash: ContentDigest,
            confirmation_hash: ContentDigest,
        }

        impl SideEffectReplayVerifier for TestReplayVerifier {
            fn verifier_id(&self) -> &events::ReplayVerifierId {
                &self.verifier_id
            }

            fn verify_submission(&self, input: &SideEffectSubmissionReplayInput) -> Result<()> {
                if input.submission.submission.submission_hash == self.submission_hash
                    && input.intent.intent.intent_hash != self.submission_hash
                {
                    Ok(())
                } else {
                    Err(ReplayError::new(
                        ReplayErrorKind::ReplayVerifierMismatch,
                        "submission verifier rejected recorded evidence",
                    ))
                }
            }

            fn verify_receipt(&self, input: &SideEffectReceiptReplayInput) -> Result<()> {
                if input.receipt.receipt.receipt_hash == self.receipt_hash
                    && input.submission.is_some()
                    && input.intent.intent.intent_hash != self.receipt_hash
                {
                    Ok(())
                } else {
                    Err(ReplayError::new(
                        ReplayErrorKind::ReplayVerifierMismatch,
                        "receipt verifier rejected recorded evidence",
                    ))
                }
            }

            fn verify_confirmation(&self, input: &SideEffectConfirmationReplayInput) -> Result<()> {
                if input.confirmation.confirmation.confirmation_hash == self.confirmation_hash
                    && input.submission.is_some()
                    && input.receipt.is_some()
                {
                    Ok(())
                } else {
                    Err(ReplayError::new(
                        ReplayErrorKind::ReplayVerifierMismatch,
                        "confirmation verifier rejected recorded evidence",
                    ))
                }
            }
        }

        struct Fixture {
            envelope: CertifiedSpecEnvelope,
            stream: Vec<KernelEventEnvelope>,
            artifacts: Vec<StoredArtifactEvidenceRef>,
            runner: events::ExecutableIdentity,
            adapter_exec: events::ExecutableIdentity,
            node_id: NodeId,
            attempt_id: AttemptId,
            fact_artifact: ArtifactId,
            fact_request_hash: ContentDigest,
            fact_response_hash: ContentDigest,
            submission_hash: ContentDigest,
            receipt_artifact: ArtifactId,
            receipt_hash: ContentDigest,
            confirmation_hash: ContentDigest,
            capability_kind: CapabilityKind,
            capability_version: CapabilityVersion,
            adapter_kind: AdapterKind,
            adapter_version: AdapterVersion,
            intent_schema_id: SchemaId,
            intent_hash: ContentDigest,
            idempotency_input_schema_id: SchemaId,
            idempotency_input_hash: ContentDigest,
            ledger_key: events::SideEffectLedgerKey,
        }

        impl Fixture {
            fn new() -> Self {
                let node_id = node(0x10);
                let attempt_id = attempt(0x11);
                let output_cell = cell(0x12);
                let scope_id = scope(0x13);
                let descriptor_id = descriptor(0x14);
                let semantic = semantic("value", 0x15);
                let value_schema = schema("mfm.test.value", 0x16);
                let input_schema = schema("mfm.test.input", 0x17);
                let config_schema = schema("mfm.test.config", 0x18);
                let public_schema = schema("mfm.test.public", 0x19);
                let capability_kind = CapabilityKind::new(
                    "mfm.test",
                    "external-mutation",
                    DigestAlgorithm::Sha256JcsV1,
                    bytes(0x20),
                )
                .expect("capability kind");
                let capability_version =
                    CapabilityVersion::new("mfm.test.capability.v1").expect("capability version");
                let adapter_kind = AdapterKind::new(
                    "mfm.test",
                    "adapter",
                    DigestAlgorithm::Sha256JcsV1,
                    bytes(0x21),
                )
                .expect("adapter kind");
                let adapter_version =
                    AdapterVersion::new("mfm.test.adapter.v1").expect("adapter version");
                let capability = CapabilityDescriptor::new(
                    capability_kind.clone(),
                    capability_version.clone(),
                    CapabilityRole::ExternalMutationAuthority,
                    "external-mutation",
                )
                .expect("capability");
                let capabilities =
                    CapabilitySetDescriptor::new(vec![capability]).expect("capabilities");
                let config_ref = spec::ConfigRef {
                    schema_id: config_schema.clone(),
                    artifact_id: artifact(0x22),
                    digest: content(0x23),
                    byte_len: 2,
                    media_type: spec::MediaType::new("application/json").expect("media"),
                };
                let planning = spec::PlanningLineage {
                    active_operation_instances: Vec::new(),
                    completed_operation_frames: Vec::new(),
                    lineage_digest: content(0x24),
                };
                let lineage = spec::ValueLineageRef {
                    lineage_digest: content(0x25),
                };
                let effect_kind = EffectKind::new(
                    "mfm.test",
                    "side-effect",
                    DigestAlgorithm::Sha256JcsV1,
                    bytes(0x26),
                )
                .expect("effect");
                let state_kind = StateKind::new(
                    "mfm.test",
                    "sidefx",
                    DigestAlgorithm::Sha256JcsV1,
                    bytes(0x27),
                )
                .expect("state kind");
                let state_version =
                    StateVersion::new("mfm.test.side_effect_state.v1").expect("state version");
                let contract_digest = content(0x28);
                let node_spec = spec::NodeSpec {
                    node_id: node_id.clone(),
                    stable_key: spec::StableAuthorKey::new("sidefx").expect("stable key"),
                    scope_id: scope_id.clone(),
                    state_kind: state_kind.clone(),
                    state_version: state_version.clone(),
                    descriptor_id: descriptor_id.clone(),
                    config_ref: config_ref.clone(),
                    input_bindings: spec::InputBindingSpec {
                        input_schema_id: input_schema.clone(),
                        input_descriptor_id: descriptor(0x29),
                        root: spec::InputBindingNodeSpec::Unit,
                        digest: content(0x2a),
                    },
                    output_cell: output_cell.clone(),
                    effect_kind: effect_kind.clone(),
                    capability_bindings: capabilities.clone(),
                    adapter_bindings: vec![spec::AdapterBinding {
                        adapter_kind: adapter_kind.clone(),
                        adapter_version: adapter_version.clone(),
                        binding_digest: None,
                    }],
                    side_effect: Some(spec::SideEffectContractSpec {
                        contract_digest: contract_digest.clone(),
                    }),
                    framework: None,
                    planning_lineage: planning.clone(),
                    deterministic_predecessors: Vec::new(),
                };
                let renderer = spec::RendererDescriptorIdentity {
                    descriptor_id: descriptor(0x2b),
                    renderer_kind: spec::RendererKind::new("public-output/json")
                        .expect("renderer kind"),
                    renderer_version: spec::RendererVersion::new("mfm.renderer.test.v1")
                        .expect("renderer version"),
                    public_schema_id: public_schema.clone(),
                    canonicalizer_identity: CanonicalizerIdentity::new("sha256-jcs-v1")
                        .expect("canonicalizer"),
                };
                let state_descriptor = spec::StateDescriptorIdentity {
                    descriptor_id: descriptor_id.clone(),
                    name: "mfm.test.sidefx".to_owned(),
                    state_kind: state_kind.clone(),
                    state_version: state_version.clone(),
                    config_schema_id: config_schema.clone(),
                    input_schema_id: input_schema,
                    output_schema_id: value_schema.clone(),
                    output_semantic_type_id: semantic.clone(),
                    effect_kind,
                    effect_class: "sidefx".to_owned(),
                    effect_name: "sidefx".to_owned(),
                    effect_version: EffectVersion::new("mfm.effect.v1").expect("effect version"),
                    capabilities: capabilities.clone(),
                    runner: "sidefx".to_owned(),
                    side_effect_contract_digest: Some(contract_digest),
                };
                let spec = spec::TypedExecutionSpec::new(spec::TypedExecutionSpecParts {
                    authoring: spec::AuthoringProvenance::StateComposition {
                        descriptor: spec::CompositionDescriptor {
                            descriptor_id: descriptor(0x2c),
                            name: "mfm.test.replay".to_owned(),
                            version: "mfm.test.replay.v1".to_owned(),
                        },
                        config_hash: content(0x2d),
                    },
                    scopes: vec![spec::ScopeSpec {
                        scope_id: scope_id.clone(),
                        parent_scope_id: None,
                        stable_key: spec::StableAuthorKey::new("root").expect("stable key"),
                        planning_lineage: planning.clone(),
                    }],
                    seeds: Vec::new(),
                    descriptor_identities: vec![
                        spec::DescriptorIdentity::State(Box::new(state_descriptor)),
                        spec::DescriptorIdentity::Renderer(Box::new(renderer.clone())),
                    ],
                    config_refs: vec![config_ref],
                    nodes: vec![node_spec],
                    cells: vec![spec::CellSpec {
                        cell_id: output_cell.clone(),
                        producer: spec::CellProducer::Node(node_id.clone()),
                        scope_id: scope_id.clone(),
                        semantic_type_id: semantic.clone(),
                        schema_id: value_schema.clone(),
                        value_lineage: lineage.clone(),
                        terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                        storage_policy: spec::StoragePolicy::ContentAddressed,
                        redaction_policy: spec::RedactionPolicy::Public,
                    }],
                    value_lineages: vec![spec::ValueLineage {
                        lineage_ref: lineage.clone(),
                        scope_id: scope_id.clone(),
                        producer: spec::CellProducer::Node(node_id.clone()),
                        input_cells: Vec::new(),
                        config_ref_digest: None,
                        planning_lineage: planning,
                        domain_keys: Vec::new(),
                        transform_policy: spec::LineageTransformPolicy::StateOutput,
                    }],
                    planning_lineage: Vec::new(),
                    public_outputs: spec::PublicOutputSpec {
                        public_schema_id: public_schema,
                        outputs: vec![spec::PublicOutputCell {
                            public_field_path: spec::PublicFieldPath::new("result").expect("field"),
                            cell_id: output_cell,
                            producer: spec::CellProducer::Node(node_id.clone()),
                            scope_id: scope_id.clone(),
                            semantic_type_id: semantic,
                            schema_id: value_schema,
                            value_lineage: lineage,
                            required_terminal: spec::RequiredTerminal::ProducedOnly,
                        }],
                        renderer_descriptor: renderer,
                    },
                })
                .expect("typed spec");
                let envelope =
                    CertifiedSpecEnvelope::new(spec, spec::TypedExecutionSpecAudit::default())
                        .expect("envelope");
                let run_id = RunId::from_digest(DigestAlgorithm::Sha256JcsV1, bytes(0x30));
                let runner = executable("sidefx");
                let adapter_exec = executable("adapter");
                let fact_artifact = artifact(0x31);
                let fact_request_hash = content(0x32);
                let fact_response_hash = content(0x33);
                let side_effect_artifact = artifact(0x34);
                let intent_schema_id = schema("mfm.test.intent", 0x35);
                let intent_hash = content(0x36);
                let idempotency_input_schema_id = schema("mfm.test.idempotency", 0x37);
                let idempotency_input_hash = content(0x38);
                let submission_artifact = artifact(0x39);
                let submission_hash = content(0x3a);
                let receipt_artifact = artifact(0x3b);
                let receipt_hash = content(0x3c);
                let confirmation_artifact = artifact(0x3d);
                let confirmation_hash = content(0x3e);
                let ledger_key =
                    events::SideEffectLedgerKey::new("ledger-key-1").expect("ledger key");

                let mut store = InMemoryTypedRunStore::default();
                let mut artifacts = Vec::new();
                let certified_config = envelope.spec.config_refs[0].clone();
                artifacts.push(stored_artifact(
                    certified_config.artifact_id,
                    certified_config.digest,
                    Some(certified_config.schema_id),
                    ArtifactRole::TypedConfig,
                    None,
                ));
                let mut spec_artifact = stored_artifact(
                    artifact(0x40),
                    spec_digest(&envelope.spec_hash),
                    None,
                    ArtifactRole::TypedExecutionSpec,
                    None,
                );
                spec_artifact.media_type = spec::MediaType::new(SPEC_MEDIA_TYPE).expect("media");
                append(
                    &mut store,
                    &mut artifacts,
                    &run_id,
                    "run-start",
                    vec![KernelEventPayload::RunStarted(events::RunStarted {
                        run_id: run_id.clone(),
                        spec_hash: envelope.spec_hash.clone(),
                        spec_artifact_id: spec_artifact.artifact_id.clone(),
                        spec_media_type: spec::MediaType::new(SPEC_MEDIA_TYPE).expect("media"),
                        spec_version: SpecVersion::new(spec::SPEC_VERSION).expect("spec version"),
                        lowering_version: LoweringVersion::new(spec::LOWERING_VERSION)
                            .expect("lowering version"),
                        public_output_schema_id: envelope
                            .spec
                            .public_outputs
                            .public_schema_id
                            .clone(),
                        descriptor_identities: envelope.spec.descriptor_identities.clone(),
                        runner_executables: vec![runner.clone()],
                        adapter_executables: vec![adapter_exec.clone()],
                        canonicalizer_identity: envelope
                            .spec
                            .public_outputs
                            .renderer_descriptor
                            .canonicalizer_identity
                            .clone(),
                        framework_version: events::FrameworkVersion::new("mfm.test.1")
                            .expect("framework"),
                        source_revision: events::SourceRevision::new("test-revision")
                            .expect("source"),
                        seed_cells: Vec::new(),
                    })],
                    vec![spec_artifact],
                    CommitPreconditions {
                        required_run_state: RequiredRunState::Absent,
                        ..CommitPreconditions::default()
                    },
                );
                append(
                    &mut store,
                    &mut artifacts,
                    &run_id,
                    "attempt-start",
                    vec![KernelEventPayload::StateAttemptStarted(
                        events::StateAttemptStarted {
                            spec_hash: envelope.spec_hash.clone(),
                            node_id: node_id.clone(),
                            attempt_id: attempt_id.clone(),
                            attempt_no: 1,
                            state_kind,
                            state_version,
                        },
                    )],
                    Vec::new(),
                    CommitPreconditions::default(),
                );
                append(
                    &mut store,
                    &mut artifacts,
                    &run_id,
                    "fact",
                    vec![KernelEventPayload::FactRecorded(events::FactRecorded {
                        spec_hash: envelope.spec_hash.clone(),
                        node_id: node_id.clone(),
                        attempt_id: attempt_id.clone(),
                        capability_kind: capability_kind.clone(),
                        capability_version: capability_version.clone(),
                        adapter_kind: adapter_kind.clone(),
                        adapter_version: adapter_version.clone(),
                        request_schema_id: schema("mfm.test.fact_request", 0x41),
                        request_hash: fact_request_hash.clone(),
                        response_schema_id: schema("mfm.test.fact_response", 0x42),
                        response_hash: fact_response_hash.clone(),
                        fact_key: events::FactKey::new("fact-key-1").expect("fact key"),
                        artifact_id: fact_artifact.clone(),
                    })],
                    vec![stored_artifact(
                        fact_artifact.clone(),
                        fact_response_hash.clone(),
                        Some(schema("mfm.test.fact_response", 0x42)),
                        ArtifactRole::FactResponse,
                        Some(node_id.clone()),
                    )],
                    CommitPreconditions::default(),
                );
                append(
                    &mut store,
                    &mut artifacts,
                    &run_id,
                    "side-effect-intent",
                    vec![
                        KernelEventPayload::SideEffectIntentPersisted(
                            side_effect::IntentPersisted {
                                spec_hash: envelope.spec_hash.clone(),
                                node_id: node_id.clone(),
                                scope_id: scope_id.clone(),
                                attempt_id: attempt_id.clone(),
                                ledger_key: ledger_key.clone(),
                                invocation_epoch: 1,
                                intent_schema_id: intent_schema_id.clone(),
                                intent_hash: intent_hash.clone(),
                                intent_artifact_id: side_effect_artifact.clone(),
                                idempotency_input_schema_id: idempotency_input_schema_id.clone(),
                                idempotency_input_hash: idempotency_input_hash.clone(),
                                idempotency_key: events::IdempotencyKeyRef::new("idem-key-1")
                                    .expect("idempotency"),
                                capability_kind: capability_kind.clone(),
                                capability_version: capability_version.clone(),
                                adapter_kind: adapter_kind.clone(),
                                adapter_version: adapter_version.clone(),
                            },
                        ),
                        KernelEventPayload::SideEffectClaimed(side_effect::Claimed {
                            spec_hash: envelope.spec_hash.clone(),
                            node_id: node_id.clone(),
                            attempt_id: attempt_id.clone(),
                            ledger_key: ledger_key.clone(),
                            claim_owner: events::RunnerInvocationId::new("owner-1").expect("owner"),
                            invocation_epoch: 1,
                            claim_generation: 1,
                            claim_fencing_token: side_effect::ClaimFencingToken::new("token-1")
                                .expect("token"),
                        }),
                        KernelEventPayload::SideEffectInvocationPrepared(
                            side_effect::InvocationPrepared {
                                spec_hash: envelope.spec_hash.clone(),
                                node_id: node_id.clone(),
                                attempt_id: attempt_id.clone(),
                                ledger_key: ledger_key.clone(),
                                invocation_epoch: 1,
                                claim_generation: 1,
                                claim_fencing_token: side_effect::ClaimFencingToken::new("token-1")
                                    .expect("token"),
                                prepared_artifact_id: None,
                                prepared_hash: None,
                            },
                        ),
                        KernelEventPayload::SideEffectInvocationStarted(
                            side_effect::InvocationStarted {
                                spec_hash: envelope.spec_hash.clone(),
                                node_id: node_id.clone(),
                                attempt_id: attempt_id.clone(),
                                ledger_key: ledger_key.clone(),
                                invocation_epoch: 1,
                                claim_owner: events::RunnerInvocationId::new("owner-1")
                                    .expect("owner"),
                                claim_generation: 1,
                                claim_fencing_token: side_effect::ClaimFencingToken::new("token-1")
                                    .expect("token"),
                            },
                        ),
                    ],
                    vec![stored_artifact(
                        side_effect_artifact.clone(),
                        intent_hash.clone(),
                        Some(intent_schema_id.clone()),
                        ArtifactRole::SideEffectIntent,
                        Some(node_id.clone()),
                    )],
                    CommitPreconditions::default(),
                );
                append(
                    &mut store,
                    &mut artifacts,
                    &run_id,
                    "side-effect-evidence",
                    vec![
                        KernelEventPayload::SideEffectSubmissionObserved(
                            side_effect::SubmissionObserved {
                                spec_hash: envelope.spec_hash.clone(),
                                node_id: node_id.clone(),
                                attempt_id: attempt_id.clone(),
                                ledger_key: ledger_key.clone(),
                                invocation_epoch: 1,
                                submission_schema_id: schema("mfm.test.submission", 0x43),
                                submission_hash: submission_hash.clone(),
                                submission_artifact_id: submission_artifact.clone(),
                            },
                        ),
                        KernelEventPayload::SideEffectReceiptObserved(
                            side_effect::ReceiptObserved {
                                spec_hash: envelope.spec_hash.clone(),
                                node_id: node_id.clone(),
                                attempt_id: attempt_id.clone(),
                                ledger_key: ledger_key.clone(),
                                invocation_epoch: 1,
                                receipt_schema_id: schema("mfm.test.receipt", 0x44),
                                receipt_hash: receipt_hash.clone(),
                                receipt_artifact_id: receipt_artifact.clone(),
                                replay_verifier_id: events::ReplayVerifierId::new("verifier-1")
                                    .expect("verifier"),
                            },
                        ),
                        KernelEventPayload::SideEffectConfirmationObserved(
                            side_effect::ConfirmationObserved {
                                spec_hash: envelope.spec_hash.clone(),
                                node_id: node_id.clone(),
                                attempt_id: attempt_id.clone(),
                                ledger_key: ledger_key.clone(),
                                invocation_epoch: 1,
                                confirmation_schema_id: schema("mfm.test.confirmation", 0x45),
                                confirmation_hash: confirmation_hash.clone(),
                                confirmation_artifact_id: confirmation_artifact.clone(),
                                replay_verifier_id: events::ReplayVerifierId::new("verifier-1")
                                    .expect("verifier"),
                            },
                        ),
                    ],
                    vec![
                        stored_artifact(
                            submission_artifact.clone(),
                            submission_hash.clone(),
                            Some(schema("mfm.test.submission", 0x43)),
                            ArtifactRole::Submission,
                            Some(node_id.clone()),
                        ),
                        stored_artifact(
                            receipt_artifact.clone(),
                            receipt_hash.clone(),
                            Some(schema("mfm.test.receipt", 0x44)),
                            ArtifactRole::Receipt,
                            Some(node_id.clone()),
                        ),
                        stored_artifact(
                            confirmation_artifact.clone(),
                            confirmation_hash.clone(),
                            Some(schema("mfm.test.confirmation", 0x45)),
                            ArtifactRole::Confirmation,
                            Some(node_id.clone()),
                        ),
                    ],
                    CommitPreconditions::default(),
                );
                let stream = store.load_run_stream(&run_id);
                Self {
                    envelope,
                    stream,
                    artifacts,
                    runner,
                    adapter_exec,
                    node_id,
                    attempt_id,
                    fact_artifact,
                    fact_request_hash,
                    fact_response_hash,
                    submission_hash,
                    receipt_artifact,
                    receipt_hash,
                    confirmation_hash,
                    capability_kind,
                    capability_version,
                    adapter_kind,
                    adapter_version,
                    intent_schema_id,
                    intent_hash,
                    idempotency_input_schema_id,
                    idempotency_input_hash,
                    ledger_key,
                }
            }

            fn authority(&self) -> ReplayAuthority {
                ReplayAuthority::from_certified_spec(
                    &self.envelope,
                    vec![self.runner.clone()],
                    vec![self.adapter_exec.clone()],
                    self.artifacts.clone(),
                )
            }

            fn broker(&self) -> ReplayBroker {
                ReplayBroker::from_run_stream(self.envelope.clone(), &self.stream, self.authority())
                    .expect("broker")
            }

            fn fact_request(&self) -> FactReplayRequest {
                FactReplayRequest {
                    node_id: self.node_id.clone(),
                    attempt_id: self.attempt_id.clone(),
                    fact_key: events::FactKey::new("fact-key-1").expect("fact key"),
                    capability_kind: self.capability_kind.clone(),
                    capability_version: self.capability_version.clone(),
                    adapter_kind: self.adapter_kind.clone(),
                    adapter_version: self.adapter_version.clone(),
                    request_schema_id: schema("mfm.test.fact_request", 0x41),
                    request_hash: self.fact_request_hash.clone(),
                    response_schema_id: schema("mfm.test.fact_response", 0x42),
                }
            }

            fn submission_request(&self) -> SideEffectEvidenceReplayRequest {
                self.side_effect_request(
                    schema("mfm.test.submission", 0x43),
                    self.submission_hash.clone(),
                    None,
                )
            }

            fn receipt_request(&self) -> SideEffectEvidenceReplayRequest {
                self.side_effect_request(
                    schema("mfm.test.receipt", 0x44),
                    self.receipt_hash.clone(),
                    Some(events::ReplayVerifierId::new("verifier-1").expect("verifier")),
                )
            }

            fn confirmation_request(&self) -> SideEffectEvidenceReplayRequest {
                self.side_effect_request(
                    schema("mfm.test.confirmation", 0x45),
                    self.confirmation_hash.clone(),
                    Some(events::ReplayVerifierId::new("verifier-1").expect("verifier")),
                )
            }

            fn side_effect_request(
                &self,
                evidence_schema_id: SchemaId,
                evidence_hash: ContentDigest,
                replay_verifier_id: Option<events::ReplayVerifierId>,
            ) -> SideEffectEvidenceReplayRequest {
                SideEffectEvidenceReplayRequest {
                    ledger_key: self.ledger_key.clone(),
                    node_id: self.node_id.clone(),
                    attempt_id: self.attempt_id.clone(),
                    invocation_epoch: 1,
                    intent_schema_id: self.intent_schema_id.clone(),
                    intent_hash: self.intent_hash.clone(),
                    idempotency_input_schema_id: self.idempotency_input_schema_id.clone(),
                    idempotency_input_hash: self.idempotency_input_hash.clone(),
                    capability_kind: self.capability_kind.clone(),
                    capability_version: self.capability_version.clone(),
                    adapter_kind: self.adapter_kind.clone(),
                    adapter_version: self.adapter_version.clone(),
                    evidence_schema_id,
                    evidence_hash,
                    replay_verifier_id,
                }
            }
        }

        fn append(
            store: &mut InMemoryTypedRunStore,
            retained_artifacts: &mut Vec<StoredArtifactEvidenceRef>,
            run_id: &RunId,
            commit_key: &str,
            payloads: Vec<KernelEventPayload>,
            artifacts: Vec<StoredArtifactEvidenceRef>,
            preconditions: CommitPreconditions,
        ) {
            for artifact in &artifacts {
                store
                    .record_artifact_evidence(artifact.clone())
                    .expect("record artifact");
            }
            retained_artifacts.extend(artifacts.iter().cloned());
            store
                .append_typed_run_commit(TypedCommitRequest {
                    run_id: run_id.clone(),
                    expected_next_seq: store.expected_next_seq(run_id),
                    commit_key: CommitKey::new(commit_key).expect("commit key"),
                    payloads,
                    required_artifacts: artifacts,
                    preconditions,
                })
                .expect("append typed commit");
        }

        fn append_payload_to_stream(
            stream: &mut Vec<KernelEventEnvelope>,
            commit_key: &str,
            payload: KernelEventPayload,
        ) {
            let seq = stream.last().expect("non-empty stream").seq();
            let seq = StreamSeq::new(seq.as_u64() + 1).expect("next sequence");
            let request = TypedCommitRequest {
                run_id: stream[0].run_id().clone(),
                expected_next_seq: seq,
                commit_key: CommitKey::new(commit_key).expect("commit key"),
                payloads: vec![payload],
                required_artifacts: Vec::new(),
                preconditions: CommitPreconditions::default(),
            };
            let batch = build_committed_batch(&request, seq).expect("build envelope");
            stream.extend(batch.events().iter().cloned());
        }

        fn stored_artifact(
            artifact_id: ArtifactId,
            digest: ContentDigest,
            schema_id: Option<SchemaId>,
            role: ArtifactRole,
            producer_node_id: Option<NodeId>,
        ) -> StoredArtifactEvidenceRef {
            StoredArtifactEvidenceRef {
                artifact_id,
                digest,
                byte_len: 128,
                media_type: spec::MediaType::new("application/json").expect("media"),
                schema_id,
                semantic_type_id: None,
                producer_node_id,
                producer_seed_id: None::<SeedId>,
                artifact_role: role,
            }
        }

        fn executable(factory: &str) -> events::ExecutableIdentity {
            events::ExecutableIdentity {
                factory_id: events::RunnerFactoryId::new(factory).expect("factory"),
                source_revision: events::SourceRevision::new("test-revision").expect("source"),
                cargo_package_name: events::PackageName::new("mfm-test").expect("package"),
                cargo_package_version: events::PackageVersion::new("0.1.0").expect("version"),
                cargo_package_digest: content(0xe1),
                binary_digest: content(0xe2),
                nix_derivation_hash: None,
                nix_output_hash: None,
            }
        }

        fn bytes(byte: u8) -> DigestBytes {
            DigestBytes::from_array([byte; 32])
        }

        fn content(byte: u8) -> ContentDigest {
            ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, bytes(byte))
        }

        fn artifact(byte: u8) -> ArtifactId {
            ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, bytes(byte))
        }

        fn attempt(byte: u8) -> AttemptId {
            AttemptId::from_digest(DigestAlgorithm::Sha256JcsV1, bytes(byte))
        }

        fn node(byte: u8) -> NodeId {
            NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, bytes(byte))
        }

        fn cell(byte: u8) -> mfm_ids::CellId {
            mfm_ids::CellId::from_digest(DigestAlgorithm::Sha256JcsV1, bytes(byte))
        }

        fn scope(byte: u8) -> ScopeId {
            ScopeId::from_digest(DigestAlgorithm::Sha256JcsV1, bytes(byte))
        }

        fn descriptor(byte: u8) -> DescriptorId {
            DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, bytes(byte))
        }

        fn schema(name: &str, byte: u8) -> SchemaId {
            SchemaId::new(name, "1", DigestAlgorithm::Sha256JcsV1, bytes(byte)).expect("schema")
        }

        fn semantic(name: &str, byte: u8) -> SemanticTypeId {
            SemanticTypeId::new(
                "mfm.test",
                name,
                "1",
                DigestAlgorithm::Sha256JcsV1,
                bytes(byte),
            )
            .expect("semantic")
        }
    }
}
