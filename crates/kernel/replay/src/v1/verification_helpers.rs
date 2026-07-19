use super::*;

pub(super) fn exact_retained_artifact_for_requirement<'a>(
    artifacts: &'a BTreeMap<ReplayArtifactAuthorityKey, StoredArtifactEvidenceRef>,
    requirement: &store::EventArtifactRequirement,
) -> Result<&'a StoredArtifactEvidenceRef> {
    let evidence = artifacts
        .get(&(
            requirement.artifact_id.clone(),
            requirement.evidence_hash.clone(),
        ))
        .ok_or_else(|| {
            ReplayError::new(
                ReplayErrorKind::ArtifactMissing,
                format!(
                    "missing retained artifact evidence for {}",
                    requirement.artifact_id
                ),
            )
        })?;
    store::validate_artifact_requirement_against_evidence(requirement, evidence)
        .map_err(|error| artifact_requirement_replay_error(error, "artifact evidence mismatch"))?;
    Ok(evidence)
}

pub(super) fn insert_fact_descriptor_candidate(
    candidates: &mut BTreeMap<ReplayArtifactAuthorityKey, StoredArtifactEvidenceRef>,
    evidence: &StoredArtifactEvidenceRef,
) -> Result<()> {
    let key = replay_artifact_authority_key(evidence)?;
    if let Some(existing) = candidates.get(&key) {
        if existing != evidence {
            return Err(ReplayError::new(
                ReplayErrorKind::ArtifactMismatch,
                format!(
                    "conflicting retained fact descriptor evidence for {}",
                    evidence.artifact_id
                ),
            ));
        }
    } else {
        candidates.insert(key, evidence.clone());
    }
    Ok(())
}

pub(super) fn artifact_map(
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

pub(super) fn artifact_bytes_map(
    artifacts: Vec<(ReplayArtifactAuthorityKey, Vec<u8>)>,
) -> Result<BTreeMap<ReplayArtifactAuthorityKey, Vec<u8>>> {
    let mut map = BTreeMap::new();
    for (key, bytes) in artifacts {
        match map.entry(key) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(bytes);
            }
            std::collections::btree_map::Entry::Occupied(entry) => {
                if entry.get() != &bytes {
                    return Err(ReplayError::new(
                        ReplayErrorKind::ArtifactMismatch,
                        format!("conflicting retained artifact bytes for {:?}", entry.key()),
                    ));
                }
            }
        }
    }
    Ok(map)
}

pub(super) fn artifact_byte_authority_map(
    artifacts: &BTreeMap<ReplayArtifactAuthorityKey, StoredArtifactEvidenceRef>,
    artifact_bytes: &BTreeMap<ReplayArtifactAuthorityKey, Vec<u8>>,
) -> Result<store::ArtifactByteAuthorityMap> {
    let mut authority = store::ArtifactByteAuthorityMap::new();
    for (key, bytes) in artifact_bytes {
        let evidence = artifacts.get(key).ok_or_else(|| {
            ReplayError::new(
                ReplayErrorKind::ArtifactMismatch,
                format!("unverified retained artifact bytes supplied for {}", key.0),
            )
        })?;
        let verified = store::PreparedArtifactBytes::new(bytes.clone(), evidence.clone()).map_err(
            |error| ReplayError::new(ReplayErrorKind::ArtifactMismatch, error.to_string()),
        )?;
        let (bytes, evidence, evidence_hash) = verified.into_parts();
        if evidence_hash != key.1 {
            return Err(ReplayError::new(
                ReplayErrorKind::ArtifactMismatch,
                format!(
                    "retained artifact evidence hash mismatch for {}",
                    evidence.artifact_id
                ),
            ));
        }
        authority.insert(key.clone(), (bytes, evidence));
    }
    Ok(authority)
}

pub(super) fn verify_replay_artifact_authority(
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

pub(super) fn terminal_completion_event(
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

pub(super) fn verify_resource_lane_release_adjacency(
    certified_spec: &HashedSpecEnvelope,
    stream: &[KernelEventEnvelope],
) -> Result<()> {
    let terminal_policies =
        store::SideEffectTerminalPolicies::from_spec(&certified_spec.spec).map_err(store_error)?;
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

pub(super) fn verify_resource_lane_release_payload_adjacency(
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

pub(super) fn side_effect_terminal_matches_resource_lane_release(
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
    side_effect_terminal_release_policy_allowed(terminal.kind, terminal.pair_id, terminal_policies)
}

pub(super) fn side_effect_terminal_release_policy_allowed(
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

pub(super) fn side_effect_terminal_release_role_allowed(
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

pub(super) fn verify_remediation_ledger_links(
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
                let forward = projection.side_effect_for_pair(&side_effect.run_id, forward_pair_id);
                forward
            }
            events::SideEffectLedgerPurpose::Forward => None,
        };
        let terminal_policies = store::SideEffectTerminalPolicies::from_spec(&certified_spec.spec)
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

pub(super) fn certified_manual_resolution_spec(
    policy: &spec::SagaPolicySpec,
) -> Option<&spec::ManualResolutionEvidenceSpec> {
    match policy {
        spec::SagaPolicySpec::ManualResolution { manual } => Some(manual),
        spec::SagaPolicySpec::CompensateCompleted {
            on_remediation_unresolved: spec::RemediationUnresolvedSpec::ManualResolution { manual },
        } => Some(manual.as_ref()),
        spec::SagaPolicySpec::NoSideEffects
        | spec::SagaPolicySpec::FailWithoutAcdcClaim
        | spec::SagaPolicySpec::CompensateCompleted {
            on_remediation_unresolved: spec::RemediationUnresolvedSpec::FailWithoutAcdcClaim,
        } => None,
    }
}

pub(super) fn is_terminal_lifecycle_node(node: &spec::NodeSpec) -> bool {
    matches!(
        &node.framework,
        Some(
            spec::FrameworkNodeSpec::CompleteRun(_)
                | spec::FrameworkNodeSpec::ResolveSagaTerminal(_)
        )
    )
}

pub(super) fn verify_artifact_fields(
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
        evidence_hash: evidence
            .evidence_hash()
            .map_err(|error| artifact_requirement_replay_error(error, "artifact evidence hash"))?,
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

pub(super) fn verify_artifact_expectation(
    evidence: &StoredArtifactEvidenceRef,
    expected: ArtifactEvidenceExpectation<'_>,
) -> Result<()> {
    if evidence.artifact_id != *expected.artifact_id {
        return Err(ReplayError::new(
            ReplayErrorKind::ArtifactMismatch,
            format!(
                "artifact id mismatch: expected {}, found {}",
                expected.artifact_id, evidence.artifact_id
            ),
        ));
    }
    let actual_evidence_hash = evidence.evidence_hash().map_err(ReplayError::from)?;
    if &actual_evidence_hash != expected.evidence_hash {
        return Err(ReplayError::new(
            ReplayErrorKind::ArtifactMismatch,
            format!(
                "artifact evidence hash mismatch for {}",
                evidence.artifact_id
            ),
        ));
    }
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

pub(super) fn replay_artifact_authority_key(
    evidence: &StoredArtifactEvidenceRef,
) -> Result<ReplayArtifactAuthorityKey> {
    Ok((
        evidence.artifact_id.clone(),
        evidence.evidence_hash().map_err(ReplayError::from)?,
    ))
}

pub(super) fn artifact_requirement_replay_error(
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

pub(super) fn certified_evidence_mismatch(message: &'static str) -> ReplayError {
    ReplayError::new(ReplayErrorKind::CertifiedEvidenceMismatch, message)
}

pub(super) fn certified_contract_mismatch(error: mfm_certify::CertifyError) -> ReplayError {
    ReplayError::new(
        ReplayErrorKind::CertifiedEvidenceMismatch,
        error.to_string(),
    )
}

pub(super) fn certified_spec_error(error: mfm_spec::SpecError) -> ReplayError {
    ReplayError::new(
        ReplayErrorKind::CertifiedEvidenceMismatch,
        error.to_string(),
    )
}

pub(super) fn store_error(error: store::StoreError) -> ReplayError {
    ReplayError::from(error)
}

pub(super) fn run_admitted_payload(stream: &[KernelEventEnvelope]) -> Result<events::RunAdmitted> {
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

pub(super) fn verify_run_start_contract(
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
    if run_admitted.capability_implementations != authority.capability_implementations {
        return Err(ReplayError::new(
            ReplayErrorKind::UnsupportedCapability,
            "capability implementation identities do not match replay authority",
        ));
    }
    let binding_digest = admitted_binding_digest(
        &authority.runner_executables,
        &authority.adapter_executables,
        &authority.capability_implementations,
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

pub(super) fn verify_run_identity_material(
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

pub(super) fn verify_run_artifact(
    artifacts: &BTreeMap<ReplayArtifactAuthorityKey, StoredArtifactEvidenceRef>,
    expected: &events::RunArtifactEvidenceRef,
    role: ArtifactRole,
) -> Result<()> {
    if expected.role != role {
        return Err(ReplayError::new(
            ReplayErrorKind::ArtifactMismatch,
            format!(
                "run admission artifact {} has unexpected role",
                expected.artifact_id
            ),
        ));
    }
    let expected = store::ArtifactEvidenceRef::from_run_artifact(expected);
    let key = replay_artifact_authority_key(&expected)?;
    let Some(actual) = artifacts.get(&key) else {
        return Err(ReplayError::new(
            ReplayErrorKind::ArtifactMissing,
            format!(
                "missing run admission artifact evidence {}",
                expected.artifact_id
            ),
        ));
    };
    if actual != &expected {
        return Err(ReplayError::new(
            ReplayErrorKind::ArtifactMismatch,
            format!(
                "run admission artifact evidence does not match {}",
                expected.artifact_id
            ),
        ));
    }
    Ok(())
}

pub(super) fn admitted_binding_digest(
    runner_executables: &[events::ExecutableIdentity],
    adapter_executables: &[events::ExecutableIdentity],
    capability_implementations: &[events::CapabilityImplementationIdentity],
) -> Result<ContentDigest> {
    let json = serde_json::to_string(&serde_json::json!({
            "adapter_executables": adapter_executables.iter().map(executable_identity_json).collect::<Vec<_>>(),
            "capability_implementations": capability_implementations.iter().map(capability_implementation_identity_json).collect::<Vec<_>>(),
            "runner_executables": runner_executables.iter().map(executable_identity_json).collect::<Vec<_>>(),
        }))
        .map_err(|error| ReplayError::new(ReplayErrorKind::InvalidRunStream, error.to_string()))?;
    let canonical = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| ReplayError::new(ReplayErrorKind::InvalidRunStream, error.to_string()))?;
    Ok(ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        canonical.digest_bytes(),
    ))
}

fn capability_implementation_identity_json(
    identity: &events::CapabilityImplementationIdentity,
) -> serde_json::Value {
    serde_json::json!({
        "capability_kind": identity.capability_kind.as_str(),
        "capability_version": identity.capability_version.as_str(),
        "implementation_id": identity.implementation_id.as_str(),
    })
}

pub(super) fn executable_identity_json(identity: &events::ExecutableIdentity) -> serde_json::Value {
    serde_json::json!({
        "binary_digest": identity.binary_digest.as_str(),
        "cargo_package_digest": identity.cargo_package_digest.as_str(),
        "factory_id": identity.factory_id.as_str(),
        "nix_derivation_hash": identity.nix_derivation_hash.as_ref().map(|value| value.as_str()),
        "nix_output_hash": identity.nix_output_hash.as_ref().map(|value| value.as_str()),
    })
}

pub(super) fn capability_set_contains(
    capabilities: &CapabilitySetDescriptor,
    capability_kind: &CapabilityKind,
    capability_version: &CapabilityVersion,
) -> bool {
    capabilities.capabilities.iter().any(|capability| {
        capability.kind == *capability_kind && capability.version == *capability_version
    })
}

pub(super) fn verify_replay_verifier(
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

pub(super) fn side_effect_mismatch(message: &'static str) -> ReplayError {
    ReplayError::new(ReplayErrorKind::SideEffectMismatch, message)
}

pub(super) fn verify_side_effect_submit_claim_identity(
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

pub(super) fn insert_unique<K, V>(
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

pub(super) fn spec_digest(spec_hash: &SpecHash) -> ContentDigest {
    ContentDigest::from_digest(spec_hash.algorithm(), *spec_hash.digest())
}
