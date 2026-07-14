use super::*;

#[derive(Clone)]
pub(super) struct MissingValidationReportArtifactStore {
    store: ContractRunStore,
    artifacts: ContractArtifactOverlay,
}

impl MissingValidationReportArtifactStore {
    pub(super) fn new(inner: ContractRunStore) -> Self {
        Self {
            store: inner.clone(),
            artifacts: ContractArtifactOverlay::new(Arc::new(inner)),
        }
    }
}

impl store::RunEventStore for MissingValidationReportArtifactStore {
    type Error = store::StoreError;

    fn append_prepared_commit_bundle<'a>(
        &'a self,
        bundle: store::PreparedCommitBundle,
    ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        self.store.append_prepared_commit_bundle(bundle)
    }

    fn load_run_stream<'a>(
        &'a self,
        run_id: &'a mfm_ids::RunId,
    ) -> store::AsyncStoreFuture<'a, Vec<store::KernelEventEnvelope>, Self::Error> {
        self.store.load_run_stream(run_id)
    }

    fn load_committed_run_stream<'a>(
        &'a self,
        run_id: &'a mfm_ids::RunId,
    ) -> store::AsyncStoreFuture<'a, store::CommittedRunStream, Self::Error> {
        self.store.load_committed_run_stream(run_id)
    }

    fn expected_next_seq<'a>(
        &'a self,
        run_id: &'a mfm_ids::RunId,
    ) -> store::AsyncStoreFuture<'a, store::StreamSeq, Self::Error> {
        self.store.expected_next_seq(run_id)
    }

    fn status_projection_snapshot<'a>(
        &'a self,
        run_id: &'a mfm_ids::RunId,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        self.store.status_projection_snapshot(run_id)
    }

    fn fact_projection_snapshot<'a>(
        &'a self,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        self.store.fact_projection_snapshot()
    }
}

impl store::StoreScopeStore for MissingValidationReportArtifactStore {
    type Error = store::StoreError;

    fn load_store_scope_id<'a>(
        &'a self,
    ) -> store::AsyncStoreFuture<'a, store::StoreScopeId, Self::Error> {
        self.store.load_store_scope_id()
    }
}

impl store::RetainedArtifactReadProvider for MissingValidationReportArtifactStore {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        let report_schema =
            ContextBoundValidationReport::schema_id().expect("validation report schema");
        if requirement.artifact_role == Some(events::ArtifactRole::StateOutput)
            && requirement.schema_id.as_ref() == Some(&report_schema)
        {
            let artifact_id = requirement.artifact_id.clone();
            return Box::pin(
                async move { Err(store::StoreError::MissingArtifact { artifact_id }) },
            );
        }
        self.artifacts.read_retained_artifact(requirement)
    }
}

#[derive(Clone)]
pub(super) struct ContractArtifactMaterial {
    pub(super) reference: LifecycleArtifactEvidenceRef,
    bytes: Vec<u8>,
    pub(super) evidence: store::ArtifactEvidenceRef,
}

pub(super) fn contract_artifact_material() -> ContractArtifactMaterial {
    let artifact: ContractArtifactConfig =
        serde_json::from_value(artifact_json()).expect("artifact config");
    let json = serde_json::to_string(&artifact).expect("artifact json");
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json).expect("canonical artifact");
    let bytes = canonical.as_bytes().to_vec();
    let digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(&bytes));
    let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    let schema_id = <ContractArtifactConfig as MfmConfig>::schema_id().expect("artifact schema");
    let evidence = store::ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: bytes.len() as u64,
        media_type: spec::MediaType::new("application/json").expect("media type"),
        schema_id: Some(schema_id),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::TypedConfig,
    };
    let reference = LifecycleArtifactEvidenceRef::new(
        evidence.artifact_id.clone(),
        evidence.digest.clone(),
        evidence
            .evidence_hash()
            .expect("contract artifact evidence hash"),
        evidence.byte_len,
        evidence.schema_id.clone(),
        evidence.semantic_type_id.clone(),
    );
    ContractArtifactMaterial {
        reference,
        bytes,
        evidence,
    }
}

#[derive(Clone)]
pub(super) struct ContractArtifactOverlay {
    inner: Arc<dyn store::RetainedArtifactReadProvider>,
    material: ContractArtifactMaterial,
}

impl ContractArtifactOverlay {
    pub(super) fn new(inner: Arc<dyn store::RetainedArtifactReadProvider>) -> Self {
        Self {
            inner,
            material: contract_artifact_material(),
        }
    }
}

impl store::RetainedArtifactReadProvider for ContractArtifactOverlay {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        if requirement.artifact_id == self.material.evidence.artifact_id {
            return Box::pin(std::future::ready(store::VerifiedRunArtifactBytes::new(
                self.material.bytes.clone(),
                self.material.evidence.clone(),
                requirement,
            )));
        }
        self.inner.read_retained_artifact(requirement)
    }
}
