use std::collections::BTreeSet;

use mfm_canonical::CanonicalValue;
use mfm_facts::FactSelectionRequest;
use mfm_ids::{FactQueryDigest, JournalRecordHash};
use mfm_journal::*;
use serde_json::Value;

const CORPUS_BYTES: &[u8] = include_bytes!("../../../../contracts/recoverability/v1/corpus.json");

#[derive(Debug)]
struct Decoded {
    canonical: Vec<u8>,
    schema_id: String,
}

fn decode_with<T>(
    bytes: &[u8],
    decode: fn(&[u8]) -> Result<T>,
    encode: fn(CanonicalValue) -> Result<T>,
) -> Result<Decoded>
where
    T: PersistedJournalValue,
{
    let decoded = decode(bytes)?;
    let canonical = decoded.as_bytes().to_vec();
    let schema_id = decoded.schema_id().as_str().to_owned();
    let content_ref = decoded.content_ref()?;
    assert_eq!(content_ref.schema_id().as_str(), schema_id);

    let round_trip = encode(decoded.canonical_value()?)?;
    assert_eq!(round_trip.as_bytes(), canonical);
    assert_eq!(round_trip.schema_id(), decoded.schema_id());

    Ok(Decoded {
        canonical,
        schema_id,
    })
}

fn decode_retained_value_contract(bytes: &[u8]) -> Result<Decoded> {
    let decoded = RetainedValueContract::strict_decode(bytes)?;
    let validated = decoded.validated()?;
    let canonical = validated.as_bytes().to_vec();
    let schema_id = validated.schema_id().as_str().to_owned();
    let content_ref = mfm_canonical::RecoverabilityContract::embedded()?.content_ref(&validated)?;
    assert_eq!(content_ref.schema_id().as_str(), schema_id);

    let round_trip = RetainedValueContract::from_validated(validated)?;
    assert_eq!(round_trip.canonical_json()?.as_bytes(), canonical);

    Ok(Decoded {
        canonical,
        schema_id,
    })
}

macro_rules! owned_codec {
    ($contract:expr, $bytes:expr) => {
        match $contract {
            "mfm.non-domain-entry-status.v1" => Some(decode_with(
                $bytes,
                PersistedNonDomainEntryStatus::strict_decode,
                PersistedNonDomainEntryStatus::from_canonical_value,
            )),
            "mfm.non-domain-disposition.v1" => Some(decode_with(
                $bytes,
                PersistedNonDomainDisposition::strict_decode,
                PersistedNonDomainDisposition::from_canonical_value,
            )),
            "mfm.non-domain-failure-code.v1" => Some(decode_with(
                $bytes,
                PersistedNonDomainFailureCode::strict_decode,
                PersistedNonDomainFailureCode::from_canonical_value,
            )),
            "mfm.non-domain-failure.v1" => Some(decode_with(
                $bytes,
                PersistedNonDomainFailure::strict_decode,
                PersistedNonDomainFailure::from_canonical_value,
            )),
            "mfm.access-audit-entry.v1" => Some(decode_with(
                $bytes,
                AccessAuditEntry::strict_decode,
                AccessAuditEntry::from_canonical_value,
            )),
            "mfm.artifact-admission-intent.v1" => Some(decode_with(
                $bytes,
                ArtifactAdmissionIntent::strict_decode,
                ArtifactAdmissionIntent::from_canonical_value,
            )),
            "mfm.artifact-id-preimage.v1" => Some(decode_with(
                $bytes,
                ArtifactIdPreimage::strict_decode,
                ArtifactIdPreimage::from_canonical_value,
            )),
            "mfm.authorization-ref.v1" => Some(decode_with(
                $bytes,
                AuthorizationRef::strict_decode,
                AuthorizationRef::from_canonical_value,
            )),
            "mfm.authorization-scope.v1" => Some(decode_with(
                $bytes,
                AuthorizationScope::strict_decode,
                AuthorizationScope::from_canonical_value,
            )),
            "mfm.binding-delta-entry.v1" => Some(decode_with(
                $bytes,
                BindingDeltaEntry::strict_decode,
                BindingDeltaEntry::from_canonical_value,
            )),
            "mfm.binding-delta.v1" => Some(decode_with(
                $bytes,
                BindingDelta::strict_decode,
                BindingDelta::from_canonical_value,
            )),
            "mfm.blocking-destination.v1" => Some(decode_with(
                $bytes,
                BlockingDestination::strict_decode,
                BlockingDestination::from_canonical_value,
            )),
            "mfm.blocking-producer-requirement.v1" => Some(decode_with(
                $bytes,
                BlockingProducerRequirement::strict_decode,
                BlockingProducerRequirement::from_canonical_value,
            )),
            "mfm.blocking-source.v1" => Some(decode_with(
                $bytes,
                BlockingSource::strict_decode,
                BlockingSource::from_canonical_value,
            )),
            "mfm.candidate-record-envelope.v1" => Some(decode_with(
                $bytes,
                CandidateRecordEnvelope::strict_decode,
                CandidateRecordEnvelope::from_canonical_value,
            )),
            "mfm.capability-binding-ref.v1" => Some(decode_with(
                $bytes,
                CapabilityBindingRef::strict_decode,
                CapabilityBindingRef::from_canonical_value,
            )),
            "mfm.closure-ref.v1" => Some(decode_with(
                $bytes,
                ClosureRef::strict_decode,
                ClosureRef::from_canonical_value,
            )),
            "mfm.commit-candidate-preimage.v1" => Some(decode_with(
                $bytes,
                CommitCandidatePreimage::strict_decode,
                CommitCandidatePreimage::from_canonical_value,
            )),
            "mfm.commit-digest-preimage.v1" => Some(decode_with(
                $bytes,
                CommitDigestPreimage::strict_decode,
                CommitDigestPreimage::from_canonical_value,
            )),
            "mfm.commit-envelope.v1" => Some(decode_with(
                $bytes,
                CommitEnvelope::strict_decode,
                CommitEnvelope::from_canonical_value,
            )),
            "mfm.config-manifest.v1" => Some(decode_with(
                $bytes,
                ConfigManifest::strict_decode,
                ConfigManifest::from_canonical_value,
            )),
            "mfm.configured-value-binding.v1" => Some(decode_with(
                $bytes,
                ConfiguredValueBinding::strict_decode,
                ConfiguredValueBinding::from_canonical_value,
            )),
            "mfm.configured-value-key.v1" => Some(decode_with(
                $bytes,
                ConfiguredValueKey::strict_decode,
                ConfiguredValueKey::from_canonical_value,
            )),
            "mfm.context-manifest.v1" => Some(decode_with(
                $bytes,
                ContextManifest::strict_decode,
                ContextManifest::from_canonical_value,
            )),
            "mfm.cross-run-source-manifest-entry.v1" => Some(decode_with(
                $bytes,
                CrossRunSourceManifestEntry::strict_decode,
                CrossRunSourceManifestEntry::from_canonical_value,
            )),
            "mfm.cross-run-source-manifest.v1" => Some(decode_with(
                $bytes,
                CrossRunSourceManifest::strict_decode,
                CrossRunSourceManifest::from_canonical_value,
            )),
            "mfm.cross-run-source-ref.v1" => Some(decode_with(
                $bytes,
                CrossRunSourceRef::strict_decode,
                CrossRunSourceRef::from_canonical_value,
            )),
            "mfm.external-access-authorized.v1" => Some(decode_with(
                $bytes,
                ExternalAccessAuthorized::strict_decode,
                ExternalAccessAuthorized::from_canonical_value,
            )),
            "mfm.external-access-observed.v1" => Some(decode_with(
                $bytes,
                ExternalAccessObserved::strict_decode,
                ExternalAccessObserved::from_canonical_value,
            )),
            "mfm.executor-ensure-result.v1" => Some(decode_with(
                $bytes,
                ExecutorEnsureResult::strict_decode,
                ExecutorEnsureResult::from_canonical_value,
            )),
            "mfm.executor-proof-basis.v1" => Some(decode_with(
                $bytes,
                ExecutorProofBasis::strict_decode,
                ExecutorProofBasis::from_canonical_value,
            )),
            "mfm.fact-content-identity-preimage.v1" => Some(decode_with(
                $bytes,
                FactContentIdentityPreimage::strict_decode,
                FactContentIdentityPreimage::from_canonical_value,
            )),
            "mfm.fact-claim-envelope.v1" => Some(decode_with(
                $bytes,
                FactClaimEnvelope::strict_decode,
                FactClaimEnvelope::from_canonical_value,
            )),
            "mfm.fact-emission.v1" => Some(decode_with(
                $bytes,
                FactEmission::strict_decode,
                FactEmission::from_canonical_value,
            )),
            "mfm.fact-logical-identity-preimage.v1" => Some(decode_with(
                $bytes,
                FactLogicalIdentityPreimage::strict_decode,
                FactLogicalIdentityPreimage::from_canonical_value,
            )),
            "mfm.fact-publication-routing.v1" => Some(decode_with(
                $bytes,
                FactPublicationRouting::strict_decode,
                FactPublicationRouting::from_canonical_value,
            )),
            "mfm.fact-ref.v1" => Some(decode_with(
                $bytes,
                FactRef::strict_decode,
                FactRef::from_canonical_value,
            )),
            "mfm.fact-selection-completeness.v1" => Some(decode_with(
                $bytes,
                FactSelectionCompleteness::strict_decode,
                FactSelectionCompleteness::from_canonical_value,
            )),
            "mfm.fact-selection-response.v1" => Some(decode_with(
                $bytes,
                FactSelectionResponse::strict_decode,
                FactSelectionResponse::from_canonical_value,
            )),
            "mfm.fact-selection-result.v1" => Some(decode_with(
                $bytes,
                FactSelectionResult::strict_decode,
                FactSelectionResult::from_canonical_value,
            )),
            "mfm.fact-selection-scan-attestation.v1" => Some(decode_with(
                $bytes,
                FactSelectionScanAttestation::strict_decode,
                FactSelectionScanAttestation::from_canonical_value,
            )),
            "mfm.fact-selection-scan-contract.v1" => Some(decode_with(
                $bytes,
                FactSelectionScanContract::strict_decode,
                FactSelectionScanContract::from_canonical_value,
            )),
            "mfm.frozen-read-intent.v1" => Some(decode_with(
                $bytes,
                FrozenReadIntent::strict_decode,
                FrozenReadIntent::from_canonical_value,
            )),
            "mfm.genesis-preimage.v1" => Some(decode_with(
                $bytes,
                GenesisPreimage::strict_decode,
                GenesisPreimage::from_canonical_value,
            )),
            "mfm.initial-binding.v1" => Some(decode_with(
                $bytes,
                InitialBinding::strict_decode,
                InitialBinding::from_canonical_value,
            )),
            "mfm.input-binding.v1" => Some(decode_with(
                $bytes,
                InputBinding::strict_decode,
                InputBinding::from_canonical_value,
            )),
            "mfm.input-manifest.v1" => Some(decode_with(
                $bytes,
                InputManifest::strict_decode,
                InputManifest::from_canonical_value,
            )),
            "mfm.input-manifest-ref.v1" => Some(decode_with(
                $bytes,
                InputManifestRef::strict_decode,
                InputManifestRef::from_canonical_value,
            )),
            "mfm.input-source.v1" => Some(decode_with(
                $bytes,
                InputSource::strict_decode,
                InputSource::from_canonical_value,
            )),
            "mfm.journal-head.v1" => Some(decode_with(
                $bytes,
                JournalHead::strict_decode,
                JournalHead::from_canonical_value,
            )),
            "mfm.journal-predecessor.v1" => Some(decode_with(
                $bytes,
                JournalPredecessor::strict_decode,
                JournalPredecessor::from_canonical_value,
            )),
            "mfm.legal-commit-batch.v1" => Some(decode_with(
                $bytes,
                LegalCommitBatch::strict_decode,
                LegalCommitBatch::from_canonical_value,
            )),
            "mfm.object-evidence-preimage.v1" => Some(decode_with(
                $bytes,
                ObjectEvidencePreimage::strict_decode,
                ObjectEvidencePreimage::from_canonical_value,
            )),
            "mfm.node-semantic-state.v1" => Some(decode_with(
                $bytes,
                NodeSemanticState::strict_decode,
                NodeSemanticState::from_canonical_value,
            )),
            "mfm.node-terminal-outcome.v1" => Some(decode_with(
                $bytes,
                NodeTerminalOutcome::strict_decode,
                NodeTerminalOutcome::from_canonical_value,
            )),
            "mfm.object-path-binding.v1" => Some(decode_with(
                $bytes,
                ObjectPathBinding::strict_decode,
                ObjectPathBinding::from_canonical_value,
            )),
            "mfm.observation-outcome.v1" => Some(decode_with(
                $bytes,
                ObservationOutcome::strict_decode,
                ObservationOutcome::from_canonical_value,
            )),
            "mfm.observation-ref.v1" => Some(decode_with(
                $bytes,
                ObservationRef::strict_decode,
                ObservationRef::from_canonical_value,
            )),
            "mfm.output-binding.v1" => Some(decode_with(
                $bytes,
                OutputBinding::strict_decode,
                OutputBinding::from_canonical_value,
            )),
            "mfm.output-ref.v1" => Some(decode_with(
                $bytes,
                OutputRef::strict_decode,
                OutputRef::from_canonical_value,
            )),
            "mfm.pending-effect.v1" => Some(decode_with(
                $bytes,
                PendingEffect::strict_decode,
                PendingEffect::from_canonical_value,
            )),
            "mfm.pending-effect-state.v1" => Some(decode_with(
                $bytes,
                PendingEffectState::strict_decode,
                PendingEffectState::from_canonical_value,
            )),
            "mfm.producer-binding.v1" => Some(decode_with(
                $bytes,
                ProducerBinding::strict_decode,
                ProducerBinding::from_canonical_value,
            )),
            "mfm.read-capability-binding.v1" => Some(decode_with(
                $bytes,
                ReadCapabilityBinding::strict_decode,
                ReadCapabilityBinding::from_canonical_value,
            )),
            "mfm.record-hash-preimage.v1" => Some(decode_with(
                $bytes,
                RecordHashPreimage::strict_decode,
                RecordHashPreimage::from_canonical_value,
            )),
            "mfm.record-id-preimage.v1" => Some(decode_with(
                $bytes,
                RecordIdPreimage::strict_decode,
                RecordIdPreimage::from_canonical_value,
            )),
            "mfm.record-logical-key.v1" => Some(decode_with(
                $bytes,
                RecordLogicalKey::strict_decode,
                RecordLogicalKey::from_canonical_value,
            )),
            "mfm.record-ref.v1" => Some(decode_with(
                $bytes,
                RecordRef::strict_decode,
                RecordRef::from_canonical_value,
            )),
            "mfm.retained-value-contract.v1" => Some(decode_retained_value_contract($bytes)),
            "mfm.root-manifest-entry.v1" => Some(decode_with(
                $bytes,
                RootManifestEntry::strict_decode,
                RootManifestEntry::from_canonical_value,
            )),
            "mfm.run-admitted.v1" => Some(decode_with(
                $bytes,
                RunAdmitted::strict_decode,
                RunAdmitted::from_canonical_value,
            )),
            "mfm.run-closed.v1" => Some(decode_with(
                $bytes,
                RunClosed::strict_decode,
                RunClosed::from_canonical_value,
            )),
            "mfm.run-journal-record.v1" => Some(decode_with(
                $bytes,
                RunJournalRecord::strict_decode,
                RunJournalRecord::from_canonical_value,
            )),
            "mfm.run-semantic-state-preimage.v1" => Some(decode_with(
                $bytes,
                RunSemanticStatePreimage::strict_decode,
                RunSemanticStatePreimage::from_canonical_value,
            )),
            "mfm.run-terminal-contract.v1" => Some(decode_with(
                $bytes,
                RunTerminalContract::strict_decode,
                RunTerminalContract::from_canonical_value,
            )),
            "mfm.safe-failure.v1" => Some(decode_with(
                $bytes,
                SafeFailure::strict_decode,
                SafeFailure::from_canonical_value,
            )),
            "mfm.seed-manifest.v1" => Some(decode_with(
                $bytes,
                SeedManifest::strict_decode,
                SeedManifest::from_canonical_value,
            )),
            "mfm.selected-fact.v1" => Some(decode_with(
                $bytes,
                SelectedFact::strict_decode,
                SelectedFact::from_canonical_value,
            )),
            "mfm.semantic-anchor.v1" => Some(decode_with(
                $bytes,
                SemanticAnchor::strict_decode,
                SemanticAnchor::from_canonical_value,
            )),
            "mfm.semantic-closure-coordinate.v1" => Some(decode_with(
                $bytes,
                SemanticClosureCoordinate::strict_decode,
                SemanticClosureCoordinate::from_canonical_value,
            )),
            "mfm.semantic-binding.v1" => Some(decode_with(
                $bytes,
                SemanticBinding::strict_decode,
                SemanticBinding::from_canonical_value,
            )),
            "mfm.settlement.v1" => Some(decode_with(
                $bytes,
                Settlement::strict_decode,
                Settlement::from_canonical_value,
            )),
            "mfm.source-object-ref.v1" => Some(decode_with(
                $bytes,
                SourceObjectRef::strict_decode,
                SourceObjectRef::from_canonical_value,
            )),
            "mfm.source-closure-preimage.v1" => Some(decode_with(
                $bytes,
                SourceClosurePreimage::strict_decode,
                SourceClosurePreimage::from_canonical_value,
            )),
            "mfm.state-transition-committed.v1" => Some(decode_with(
                $bytes,
                StateTransitionCommitted::strict_decode,
                StateTransitionCommitted::from_canonical_value,
            )),
            "mfm.tenant-fact-coordinate.v1" => Some(decode_with(
                $bytes,
                TenantFactCoordinate::strict_decode,
                TenantFactCoordinate::from_canonical_value,
            )),
            "mfm.tenant-fact-frontier.v1" => Some(decode_with(
                $bytes,
                TenantFactFrontier::strict_decode,
                TenantFactFrontier::from_canonical_value,
            )),
            "mfm.terminal-effect-evidence.v1" => Some(decode_with(
                $bytes,
                TerminalEffectEvidence::strict_decode,
                TerminalEffectEvidence::from_canonical_value,
            )),
            "mfm.terminal-evidence-preimage.v1" => Some(decode_with(
                $bytes,
                TerminalEvidencePreimage::strict_decode,
                TerminalEvidencePreimage::from_canonical_value,
            )),
            "mfm.transition-after.v1" => Some(decode_with(
                $bytes,
                TransitionAfter::strict_decode,
                TransitionAfter::from_canonical_value,
            )),
            "mfm.transition-before.v1" => Some(decode_with(
                $bytes,
                TransitionBefore::strict_decode,
                TransitionBefore::from_canonical_value,
            )),
            "mfm.transition-body.v1" => Some(decode_with(
                $bytes,
                TransitionBody::strict_decode,
                TransitionBody::from_canonical_value,
            )),
            "mfm.transition-ref.v1" => Some(decode_with(
                $bytes,
                TransitionRef::strict_decode,
                TransitionRef::from_canonical_value,
            )),
            "mfm.value-ref.v1" => Some(decode_with(
                $bytes,
                ValueRef::strict_decode,
                ValueRef::from_canonical_value,
            )),
            _ => None,
        }
    };
}

fn corpus() -> Value {
    serde_json::from_slice(CORPUS_BYTES).expect("frozen corpus must decode")
}

fn hex_bytes(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0, "hex fixture must have whole bytes");
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("ASCII hex");
            u8::from_str_radix(pair, 16).expect("valid corpus hex")
        })
        .collect()
}

fn schema_golden(contract: &str) -> Vec<u8> {
    corpus()["positive_vectors"]
        .as_array()
        .expect("positive vector array")
        .iter()
        .find(|vector| {
            vector["kind"].as_str() == Some("schema_acceptance")
                && vector["schema_contract"].as_str() == Some(contract)
        })
        .map(|vector| hex_bytes(vector["canonical_hex"].as_str().expect("canonical hex")))
        .unwrap_or_else(|| panic!("missing schema golden for {contract}"))
}

#[test]
fn every_owned_schema_round_trips_its_frozen_golden_vectors() {
    let corpus = corpus();
    let mut schemas = BTreeSet::new();
    let mut vectors = 0_usize;

    for vector in corpus["positive_vectors"]
        .as_array()
        .expect("positive vector array")
    {
        if vector["kind"].as_str() != Some("schema_acceptance") {
            continue;
        }
        let contract = vector["schema_contract"].as_str().expect("schema contract");
        let bytes = hex_bytes(vector["canonical_hex"].as_str().expect("canonical hex"));
        let Some(decoded) = owned_codec!(contract, &bytes) else {
            continue;
        };
        let decoded = decoded.unwrap_or_else(|error| {
            panic!("{} rejected its positive vector: {error}", vector["id"])
        });

        assert_eq!(decoded.canonical, bytes, "{}", vector["id"]);
        assert_eq!(
            decoded.schema_id,
            vector["expected_schema_id"].as_str().expect("schema id"),
            "{}",
            vector["id"]
        );
        schemas.insert(contract);
        vectors += 1;
    }

    assert_eq!(schemas.len(), 94, "every journal-owned schema must execute");
    assert!(
        vectors >= schemas.len(),
        "every schema needs a positive vector"
    );
}

#[test]
fn tagged_records_bodies_and_legal_batches_cover_the_closed_algebras() {
    let corpus = corpus();
    let mut records = BTreeSet::new();
    let mut bodies = BTreeSet::new();
    let mut purposes = BTreeSet::new();
    let mut coordinates = BTreeSet::new();

    for vector in corpus["positive_vectors"]
        .as_array()
        .expect("positive vector array")
    {
        match (vector["kind"].as_str(), vector["schema_contract"].as_str()) {
            (Some("schema_acceptance"), Some("mfm.run-journal-record.v1")) => {
                let bytes = hex_bytes(vector["canonical_hex"].as_str().expect("canonical hex"));
                let record = RunJournalRecord::strict_decode(&bytes).expect("record golden");
                let name = match record.fields().expect("typed record") {
                    RunJournalRecordFields::RunAdmitted(_) => "run_admitted",
                    RunJournalRecordFields::StateTransitionCommitted(_) => {
                        "state_transition_committed"
                    }
                    RunJournalRecordFields::ExternalAccessAuthorized(_) => {
                        "external_access_authorized"
                    }
                    RunJournalRecordFields::ExternalAccessObserved(_) => "external_access_observed",
                    RunJournalRecordFields::RunClosed(_) => "run_closed",
                };
                records.insert(name);
            }
            (Some("schema_acceptance"), Some("mfm.transition-body.v1")) => {
                let bytes = hex_bytes(vector["canonical_hex"].as_str().expect("canonical hex"));
                let body = TransitionBody::strict_decode(&bytes).expect("body golden");
                let name = match body.fields().expect("typed body") {
                    TransitionBodyFields::PureSettled { .. } => "pure_settled",
                    TransitionBodyFields::ReadSettled { .. } => "read_settled",
                    TransitionBodyFields::EffectRequested { .. } => "effect_requested",
                    TransitionBodyFields::EffectSettled { .. } => "effect_settled",
                    TransitionBodyFields::DependencySkipped { .. } => "dependency_skipped",
                };
                bodies.insert(name);
            }
            _ => {}
        }

        if vector["kind"].as_str() == Some("legal_batch") {
            let batch_bytes = hex_bytes(vector["batch_hex"].as_str().expect("batch hex"));
            let batch = LegalCommitBatch::strict_decode(&batch_bytes).expect("legal batch");
            batch.fields().expect("typed legal batch");
            purposes.insert(vector["batch_purpose"].as_str().expect("batch purpose"));

            let coordinate_bytes =
                hex_bytes(vector["coordinate_hex"].as_str().expect("coordinate hex"));
            let coordinate =
                TenantFactCoordinate::strict_decode(&coordinate_bytes).expect("coordinate");
            coordinate.fields().expect("typed coordinate");
            coordinates.insert(vector["coordinate_kind"].as_str().expect("coordinate kind"));
        }
    }

    assert_eq!(
        records,
        BTreeSet::from([
            "external_access_authorized",
            "external_access_observed",
            "run_admitted",
            "run_closed",
            "state_transition_committed",
        ])
    );
    assert_eq!(
        bodies,
        BTreeSet::from([
            "dependency_skipped",
            "effect_requested",
            "effect_settled",
            "pure_settled",
            "read_settled",
        ])
    );
    assert_eq!(
        purposes,
        BTreeSet::from([
            "dependency_skip",
            "effect_request",
            "effect_settlement",
            "external_access_authorization",
            "external_access_observation",
            "pure_settlement",
            "read_settlement",
            "run_admission",
        ])
    );
    assert_eq!(
        coordinates,
        BTreeSet::from(["fact_publication", "fact_selection_barrier", "none"])
    );
}

#[test]
fn closure_is_nested_after_its_transition_and_never_forms_a_standalone_batch() {
    let corpus = corpus();
    let mut saw_closed = 0_usize;
    let mut saw_open_transition = 0_usize;

    for vector in corpus["positive_vectors"]
        .as_array()
        .expect("positive vector array")
        .iter()
        .filter(|vector| vector["kind"].as_str() == Some("legal_batch"))
    {
        let id = vector["id"].as_str().expect("batch id");
        let batch = LegalCommitBatch::strict_decode(&hex_bytes(
            vector["batch_hex"].as_str().expect("batch hex"),
        ))
        .unwrap_or_else(|error| panic!("{id} failed: {error}"));
        if let LegalCommitBatchFields::Transition {
            transition,
            closure,
        } = batch.fields().expect("typed legal batch")
        {
            let expects_closure = id.ends_with("/closed");
            assert_eq!(closure.is_some(), expects_closure, "{id}");
            transition.fields().expect("closure follows a transition");
            if expects_closure {
                closure
                    .expect("closed vector has adjacent closure")
                    .fields()
                    .expect("typed closure");
                saw_closed += 1;
            } else {
                saw_open_transition += 1;
            }
        } else {
            assert!(id.ends_with("/open"), "{id}");
        }
    }
    assert_eq!(saw_closed, 7);
    assert_eq!(saw_open_transition, 8);

    let closure =
        RunClosed::strict_decode(&schema_golden("mfm.run-closed.v1")).expect("closure golden");
    let standalone = CanonicalValue::object([
        ("kind", CanonicalValue::String("run_closed".to_owned())),
        ("closure", closure.canonical_value().expect("closure value")),
    ])
    .expect("canonical standalone attempt");
    let error = LegalCommitBatch::from_canonical_value(standalone)
        .expect_err("standalone close is illegal");
    assert_eq!(
        error
            .recoverability_error()
            .expect("codec rejection")
            .code()
            .as_str(),
        "invalid_value"
    );

    let old_body = TransitionBody::strict_decode(br#"{"kind":"state_attempt_completed"}"#)
        .expect_err("old lifecycle body is outside the closed algebra");
    assert_eq!(
        old_body
            .recoverability_error()
            .expect("codec rejection")
            .code()
            .as_str(),
        "invalid_value"
    );
}

#[test]
fn owned_codecs_return_the_frozen_negative_error_codes() {
    let corpus = corpus();
    let mut checked = 0_usize;

    for vector in corpus["negative_vectors"]
        .as_array()
        .expect("negative vector array")
    {
        if vector["error_class"].as_str() != Some("codec") {
            continue;
        }
        let target = vector["target"].as_str().expect("negative target");
        let bytes = hex_bytes(vector["input_hex"].as_str().expect("negative bytes"));
        let Some(result) = owned_codec!(target, &bytes) else {
            continue;
        };
        let error = result.expect_err("negative vector must reject");
        let codec = error
            .recoverability_error()
            .unwrap_or_else(|| panic!("{} did not return a codec error", vector["id"]));
        assert_eq!(
            codec.code().as_str(),
            vector["expected_error"].as_str().expect("error code"),
            "{}",
            vector["id"]
        );
        checked += 1;
    }

    assert_eq!(
        checked, 1,
        "every frozen codec-negative vector owned by the journal must execute"
    );
}

#[test]
fn typed_identity_helpers_match_the_frozen_domain_vectors() {
    let corpus = corpus();
    let mut checked = BTreeSet::new();

    for vector in corpus["positive_vectors"]
        .as_array()
        .expect("positive vector array")
    {
        if vector["kind"].as_str() != Some("domain_identity") {
            continue;
        }
        let domain = vector["domain"].as_str().expect("domain");
        let bytes = hex_bytes(vector["value_hex"].as_str().expect("preimage bytes"));
        let actual = match domain {
            "mfm.artifact-id.v1" => ArtifactIdPreimage::strict_decode(&bytes)
                .expect("artifact preimage")
                .artifact_id()
                .expect("artifact identity")
                .to_string(),
            "mfm.fact-content-identity.v1" => FactContentIdentityPreimage::strict_decode(&bytes)
                .expect("fact-content preimage")
                .fact_content_identity()
                .expect("fact-content identity")
                .to_string(),
            "mfm.fact-logical-identity.v1" => FactLogicalIdentityPreimage::strict_decode(&bytes)
                .expect("fact-logical preimage")
                .fact_logical_identity()
                .expect("fact-logical identity")
                .to_string(),
            "mfm.genesis.v1" => GenesisPreimage::strict_decode(&bytes)
                .expect("genesis preimage")
                .genesis_digest()
                .expect("genesis digest")
                .to_string(),
            "mfm.journal-candidate.v1" => CommitCandidatePreimage::strict_decode(&bytes)
                .expect("candidate preimage")
                .candidate_digest()
                .expect("candidate digest")
                .to_string(),
            "mfm.journal-commit.v1" => CommitDigestPreimage::strict_decode(&bytes)
                .expect("commit preimage")
                .commit_digest()
                .expect("commit digest")
                .to_string(),
            "mfm.journal-record-id.v1" => RecordIdPreimage::strict_decode(&bytes)
                .expect("record-id preimage")
                .record_id()
                .expect("record id")
                .to_string(),
            "mfm.journal-record.v1" => RecordHashPreimage::strict_decode(&bytes)
                .expect("record preimage")
                .record_hash()
                .expect("record hash")
                .to_string(),
            "mfm.object-evidence.v1" => ObjectEvidencePreimage::strict_decode(&bytes)
                .expect("object-evidence preimage")
                .evidence_hash()
                .expect("evidence digest")
                .to_string(),
            "mfm.run-semantic-state.v1" => RunSemanticStatePreimage::strict_decode(&bytes)
                .expect("run-state preimage")
                .run_state_digest()
                .expect("run-state digest")
                .to_string(),
            "mfm.source-closure.v1" => SourceClosurePreimage::strict_decode(&bytes)
                .expect("source-closure preimage")
                .source_closure_digest()
                .expect("source-closure digest")
                .to_string(),
            "mfm.cross-run-source-redaction.v1" => CrossRunSourceRef::strict_decode(&bytes)
                .expect("cross-run source")
                .redaction_digest()
                .expect("cross-run source redaction digest")
                .to_string(),
            "mfm.terminal-effect-evidence.v1" => TerminalEvidencePreimage::strict_decode(&bytes)
                .expect("terminal-evidence preimage")
                .evidence_digest()
                .expect("terminal-evidence digest")
                .to_string(),
            _ => continue,
        };

        assert_eq!(
            actual,
            vector["expected"]["value"]
                .as_str()
                .expect("expected identity"),
            "{}",
            vector["id"]
        );
        checked.insert(domain);
    }

    assert_eq!(checked.len(), 13);
}

#[test]
fn typed_references_and_object_relations_round_trip_without_identity_substitution() {
    let record =
        RecordRef::strict_decode(&schema_golden("mfm.record-ref.v1")).expect("record-ref golden");
    let record_fields = record.fields().expect("typed record ref");
    let transition = TransitionRef::new(&record).expect("transition ref");
    let authorization = AuthorizationRef::new(&record).expect("authorization ref");
    let observation = ObservationRef::new(&record).expect("observation ref");
    let closure = ClosureRef::new(&record).expect("closure ref");
    for fields in [
        transition.fields().expect("transition fields"),
        authorization.fields().expect("authorization fields"),
        observation.fields().expect("observation fields"),
        closure.fields().expect("closure fields"),
    ] {
        assert_eq!(fields, record_fields);
    }
    assert_ne!(transition.schema_id(), authorization.schema_id());

    let output = OutputRef::new(&transition, 7).expect("output ref");
    let fact = FactRef::new(&transition, 9).expect("fact ref");
    assert_eq!(
        output.fields().expect("output fields").transition_ref,
        transition
    );
    assert_eq!(
        fact.fields().expect("fact fields").transition_ref,
        transition
    );

    let value =
        ValueRef::strict_decode(&schema_golden("mfm.value-ref.v1")).expect("value-ref golden");
    let value_fields = value.fields().expect("typed value ref");
    let rebuilt_value = ValueRef::new(
        &value_fields.artifact_id,
        &value_fields.content_digest,
        &value_fields.evidence_hash,
        &value_fields.schema_id,
        &value_fields.semantic_type_id,
        &value_fields.role,
        value_fields.byte_length,
        &value_fields.media_type,
        &value_fields.evidence_contract_ref,
        &value_fields.producer_binding,
    )
    .expect("rebuilt value ref");
    assert_eq!(rebuilt_value.as_bytes(), value.as_bytes());

    let binding = ObjectPathBinding::strict_decode(&schema_golden("mfm.object-path-binding.v1"))
        .expect("object binding golden");
    let binding_fields = binding.fields().expect("binding fields");
    let intent =
        ArtifactAdmissionIntent::strict_decode(&schema_golden("mfm.artifact-admission-intent.v1"))
            .expect("artifact intent golden");
    let intent_fields = intent.fields().expect("intent fields");
    assert_eq!(binding_fields.value_ref, value);
    assert_eq!(
        binding_fields.evidence_contract_ref,
        value_fields.evidence_contract_ref
    );
    assert_eq!(intent_fields.value_ref, binding_fields.value_ref);
    assert_eq!(
        intent_fields.evidence_contract_ref,
        binding_fields.evidence_contract_ref
    );

    let candidate =
        CandidateRecordEnvelope::strict_decode(&schema_golden("mfm.candidate-record-envelope.v1"))
            .expect("candidate record golden");
    let record_preimage =
        RecordHashPreimage::from_candidate(&candidate).expect("record-hash preimage");
    let frozen_preimage =
        RecordHashPreimage::strict_decode(&schema_golden("mfm.record-hash-preimage.v1"))
            .expect("record-hash golden");
    assert_eq!(record_preimage.as_bytes(), frozen_preimage.as_bytes());
    assert_eq!(
        record_preimage.record_hash().expect("record hash"),
        frozen_preimage.record_hash().expect("frozen record hash")
    );
}

#[test]
fn fact_response_and_access_audit_relations_are_explicitly_checked() {
    let request =
        FactSelectionRequest::from_canonical_json(&schema_golden("mfm.fact-selection-request.v1"))
            .expect("fact request golden");
    let request_digest = request.request_digest().expect("request digest");
    let frontier = TenantFactFrontier::strict_decode(&schema_golden("mfm.tenant-fact-frontier.v1"))
        .expect("frontier golden");
    let result = FactSelectionResult::new(0, &[]).expect("explicit empty result");
    let response = FactSelectionResponse::new(&request_digest, &frontier, &[result])
        .expect("complete response");
    response
        .validate_request(&request)
        .expect("request digest and ordinal coverage");

    let wrong_digest =
        FactQueryDigest::parse(format!("sha256-jcs-v1:{}", "1".repeat(64))).expect("test digest");
    let wrong_response = FactSelectionResponse::new(
        &wrong_digest,
        &frontier,
        &[FactSelectionResult::new(0, &[]).expect("result")],
    )
    .expect("structurally valid wrong response");
    assert!(matches!(
        wrong_response.validate_request(&request),
        Err(JournalError::FactSelectionCoverage)
    ));
    let wrong_ordinal = FactSelectionResponse::new(
        &request_digest,
        &frontier,
        &[FactSelectionResult::new(1, &[]).expect("result")],
    )
    .expect("structurally valid ordinal mismatch");
    assert!(matches!(
        wrong_ordinal.validate_request(&request),
        Err(JournalError::FactSelectionCoverage)
    ));

    let selected = SelectedFact::strict_decode(&schema_golden("mfm.selected-fact.v1"))
        .expect("selected fact golden");
    selected
        .validate_reference_relation()
        .expect("fact coordinate names the repeated producer");
    let selected_fields = selected.fields().expect("selected fact fields");
    let producer = selected_fields
        .producing_transition_ref
        .fields()
        .expect("producer fields");
    let other_hash =
        JournalRecordHash::parse(format!("sha256-jcs-v1:{}", "1".repeat(64))).expect("test hash");
    let other_record = RecordRef::new(
        &producer.run_id,
        producer.run_sequence,
        producer.ordinal,
        &other_hash,
    )
    .expect("other record");
    let other_transition = TransitionRef::new(&other_record).expect("other transition");
    let mismatched = SelectedFact::new(
        &selected_fields.fact_ref,
        &other_transition,
        &selected_fields.descriptor_ref,
        &selected_fields.subject_ref,
        &selected_fields.response_ref,
        &selected_fields.content_identity,
    )
    .expect("structurally valid mismatch");
    assert!(matches!(
        mismatched.validate_reference_relation(),
        Err(JournalError::ReferenceMismatch)
    ));

    let observed =
        ExternalAccessObserved::strict_decode(&schema_golden("mfm.external-access-observed.v1"))
            .expect("observation golden");
    let observed_fields = observed.fields().expect("observation fields");
    let observation_record =
        RecordRef::strict_decode(&schema_golden("mfm.record-ref.v1")).expect("record ref");
    let observation_ref = ObservationRef::new(&observation_record).expect("observation ref");
    let audit = AccessAuditEntry::new(
        &observed_fields.authorization_ref,
        Some(&observation_ref),
        AccessAuditStatus::Returned,
        None,
        None,
        None,
        None,
    )
    .expect("observed audit");
    audit
        .validate_observation(&observation_ref, &observed)
        .expect("audit links exact authorization and observation");

    let unobserved = AccessAuditEntry::strict_decode(&schema_golden("mfm.access-audit-entry.v1"))
        .expect("unobserved audit golden");
    let authorization_ref = unobserved.fields().expect("audit fields").authorization_ref;
    unobserved
        .validate_unobserved(&authorization_ref)
        .expect("unobserved audit relation");
    assert!(matches!(
        unobserved.validate_observation(&observation_ref, &observed),
        Err(JournalError::AccessAuditMismatch)
    ));
}

#[test]
fn wrong_schema_unknown_fields_and_legacy_records_fail_closed() {
    let corpus = corpus();
    let admitted = corpus["positive_vectors"]
        .as_array()
        .expect("positive vector array")
        .iter()
        .find(|vector| {
            vector["kind"].as_str() == Some("schema_acceptance")
                && vector["schema_contract"].as_str() == Some("mfm.run-admitted.v1")
        })
        .expect("run admission golden");
    let admitted_bytes = hex_bytes(admitted["canonical_hex"].as_str().expect("canonical hex"));
    assert!(
        RunClosed::strict_decode(&admitted_bytes).is_err(),
        "a validated value cannot be substituted under another schema"
    );

    let unknown_field = CanonicalValue::object([
        ("extra", CanonicalValue::Bool(true)),
        (
            "terminal_transition_record_hash",
            CanonicalValue::String(format!("sha256-jcs-v1:{}", "0".repeat(64))),
        ),
        (
            "version",
            CanonicalValue::String("mfm.run-closed.v1".to_owned()),
        ),
    ])
    .expect("fixed canonical object");
    let unknown = RunClosed::from_canonical_value(unknown_field).expect_err("unknown field");
    assert_eq!(
        unknown
            .recoverability_error()
            .expect("codec error")
            .code()
            .as_str(),
        "unknown_field"
    );

    for legacy in [
        br#"{"kind":"state_attempt_started","payload":{}}"#.as_slice(),
        br#"{"kind":"fact_recorded","payload":{}}"#.as_slice(),
        br#"{"kind":"run_completed","payload":{}}"#.as_slice(),
    ] {
        let error = RunJournalRecord::strict_decode(legacy).expect_err("legacy record");
        assert_eq!(
            error
                .recoverability_error()
                .expect("codec error")
                .code()
                .as_str(),
            "invalid_value"
        );
    }
}

#[test]
fn source_and_manifest_contain_no_legacy_event_api() {
    let source = [
        include_str!("../src/lib.rs"),
        include_str!("../src/access.rs"),
        include_str!("../src/codec.rs"),
        include_str!("../src/commit.rs"),
        include_str!("../src/fact.rs"),
        include_str!("../src/input.rs"),
        include_str!("../src/object.rs"),
        include_str!("../src/record.rs"),
        include_str!("../src/refs.rs"),
        include_str!("../src/transition.rs"),
        include_str!("../Cargo.toml"),
    ]
    .join("\n");

    for removed in [
        "KernelEventPayload",
        "StateAttemptStarted",
        "FactRecorded",
        "RunCompleted",
        "mfm-events",
        "mfm_events",
    ] {
        assert!(
            !source.contains(removed),
            "legacy event API survived: {removed}"
        );
    }
}
