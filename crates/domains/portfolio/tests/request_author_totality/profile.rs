use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_certify::CertifiedTypedSpec;
use mfm_ids::{ContentDigest, DigestAlgorithm};
use mfm_portfolio::{
    portfolio_snapshot_program_draft, PortfolioConfig, PortfolioSnapshotOperation,
};
use mfm_program::{BridgeKind, BridgePolicy, Operation as _, RunnerKind, TypedProgramDraft};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::support::evm_portfolio_config;

const PUBLISHED_ENTRY_POINT_ID: &str = "mfm.portfolio/snapshot@1";
const ENTRY_POINT_OPERATION_ID: &str = "mfm.portfolio/snapshot";
const ROOT_OPERATION_NAME: &str = "mfm.portfolio.snapshot";
const PLANNER_CONTRACT_REF: &str = "mfm.composite-planner.v1";
const AUTHORED_PROGRAM_VERSION: &str = "mfm.authored-program.portfolio-snapshot.v1";
const PLANNING_PROFILE_SCHEMA_ID: &str = "mfm.planning-profile.v1";
const CERTIFICATE_VERSION: &str = "mfm.entry-point-certificate.prototype.v1";
const AUTHORED_PROGRAM_DIGEST_DOMAIN: &str = "mfm.authored-program-ref.prototype.v1";
const PLANNING_PROFILE_DIGEST_DOMAIN: &str = "mfm.planning-profile-ref.prototype.v1";
const PLANNER_IMPLEMENTATION_DIGEST_DOMAIN: &str = "mfm.planner-implementation-ref.prototype.v1";
const EXPANDED_SPEC_DIGEST_DOMAIN: &str = "mfm.expanded-spec-hash.prototype.v1";
const CERTIFICATE_DIGEST_DOMAIN: &str = "mfm.entry-point-certificate-hash.prototype.v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetainedAuthoredProgram {
    contract_version: String,
    entry_point_id: String,
    entry_point_operation_id: String,
    root_operation_name: String,
    config: PortfolioConfig,
}

impl RetainedAuthoredProgram {
    fn snapshot(config: PortfolioConfig) -> Self {
        Self {
            contract_version: AUTHORED_PROGRAM_VERSION.to_owned(),
            entry_point_id: PUBLISHED_ENTRY_POINT_ID.to_owned(),
            entry_point_operation_id: ENTRY_POINT_OPERATION_ID.to_owned(),
            root_operation_name: ROOT_OPERATION_NAME.to_owned(),
            config,
        }
    }

    fn validate(&self) -> Result<(), PrototypeAdmissionError> {
        if self.contract_version != AUTHORED_PROGRAM_VERSION
            || self.entry_point_id != PUBLISHED_ENTRY_POINT_ID
            || self.entry_point_operation_id != ENTRY_POINT_OPERATION_ID
            || self.root_operation_name != ROOT_OPERATION_NAME
            || PortfolioSnapshotOperation::name() != ROOT_OPERATION_NAME
        {
            return Err(PrototypeAdmissionError::AuthoredContractMismatch);
        }
        Ok(())
    }

    fn expand(
        &self,
        profile: &PrototypePlanningProfile,
    ) -> Result<ExpandedEvidence, PrototypeAdmissionError> {
        self.validate()?;
        let draft = portfolio_snapshot_program_draft(self.config.clone())
            .map_err(|_| PrototypeAdmissionError::ExpansionFailed)?;
        composite_expansion_evidence(&draft, profile)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrototypePlanningProfile {
    planner_contract_ref: String,
    planner_implementation_ref: String,
    framework_policy_refs: Vec<String>,
    canonical_profile_parameters: CanonicalProfileParameters,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CanonicalProfileParameters {}

impl CanonicalProfileParameters {
    const fn empty() -> Self {
        Self {}
    }
}

impl PrototypePlanningProfile {
    fn exact_snapshot_profile() -> Self {
        Self {
            planner_contract_ref: PLANNER_CONTRACT_REF.to_owned(),
            planner_implementation_ref: planner_implementation_ref(),
            framework_policy_refs: Vec::new(),
            canonical_profile_parameters: CanonicalProfileParameters::empty(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrototypeCertificateEvidence {
    certificate_version: String,
    entry_point_id: String,
    entry_point_operation_id: String,
    authored_program_ref: String,
    planning_profile_schema_id: String,
    planning_profile_ref: String,
    planner_contract_ref: String,
    planner_implementation_ref: String,
    expanded_spec_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrototypeCertificate {
    certificate_hash: String,
    evidence: PrototypeCertificateEvidence,
}

impl PrototypeCertificate {
    fn new(evidence: PrototypeCertificateEvidence) -> Result<Self, PrototypeAdmissionError> {
        let certificate_hash = prototype_digest(CERTIFICATE_DIGEST_DOMAIN, &evidence)
            .map_err(|_| PrototypeAdmissionError::CertificateMismatch)?;
        Ok(Self {
            certificate_hash,
            evidence,
        })
    }

    fn verify(&self) -> Result<(), PrototypeAdmissionError> {
        let actual = prototype_digest(CERTIFICATE_DIGEST_DOMAIN, &self.evidence)
            .map_err(|_| PrototypeAdmissionError::CertificateMismatch)?;
        if actual != self.certificate_hash {
            return Err(PrototypeAdmissionError::CertificateMismatch);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExpandedEvidence {
    spec_hash: String,
    spec_bytes: Vec<u8>,
}

impl ExpandedEvidence {
    fn new(spec: PlainCanonicalJsonBytes) -> Result<Self, PrototypeAdmissionError> {
        let value: serde_json::Value = serde_json::from_slice(spec.as_bytes())
            .map_err(|_| PrototypeAdmissionError::ExpansionFailed)?;
        Ok(Self {
            spec_hash: prototype_digest(EXPANDED_SPEC_DIGEST_DOMAIN, &value)
                .map_err(|_| PrototypeAdmissionError::ExpansionFailed)?,
            spec_bytes: spec.to_vec(),
        })
    }
}

fn composite_expansion_evidence(
    draft: &TypedProgramDraft,
    profile: &PrototypePlanningProfile,
) -> Result<ExpandedEvidence, PrototypeAdmissionError> {
    if profile != &PrototypePlanningProfile::exact_snapshot_profile()
        || !draft.contexts().is_empty()
        || !draft.seeds().is_empty()
        || !draft.remediation_nodes().is_empty()
        || draft
            .state_nodes()
            .iter()
            .any(|node| node.runner == RunnerKind::ApplySideEffect)
    {
        return Err(PrototypeAdmissionError::ExpansionFailed);
    }

    let scopes = draft
        .scopes()
        .iter()
        .map(|scope| {
            serde_json::json!({
                "key": scope.key.as_str(),
                "parent_scope_id": scope.parent_scope_id.as_ref().map(|id| id.as_str()),
                "planning_lineage_digest": scope.planning_lineage.digest.as_str(),
                "scope_id": scope.scope_id.as_str(),
            })
        })
        .collect::<Vec<_>>();
    let operations = draft
        .operation_lineage()
        .iter()
        .map(|operation| {
            serde_json::json!({
                "config_ref_digest": operation.config.config_ref_digest.as_str(),
                "expansion_abi": operation.expansion_abi,
                "input_digest": operation.input.digest.as_str(),
                "lineage_digest": operation.lineage_digest.as_str(),
                "operation_descriptor_id": operation.operation_descriptor_id.as_str(),
                "operation_instance_id": operation.operation_instance_id.as_str(),
                "operation_name": operation.operation_name.as_str(),
                "output_cells": operation
                    .output_handles
                    .iter()
                    .map(|output| output.cell_id().as_str())
                    .collect::<Vec<_>>(),
                "scope_id": operation.scope_id.as_str(),
            })
        })
        .collect::<Vec<_>>();
    let states = draft
        .state_nodes()
        .iter()
        .map(|node| {
            serde_json::json!({
                "config_ref_digest": node.config.config_ref_digest.as_str(),
                "input_digest": node.input.digest.as_str(),
                "node_id": node.node_id.as_str(),
                "output_cell_id": node.output_cell_id.as_str(),
                "planning_lineage_digest": node.planning_lineage.digest.as_str(),
                "runner": runner_name(node.runner),
                "scope_id": node.scope_id.as_str(),
                "state_descriptor_id": node.state_descriptor_id.as_str(),
                "state_descriptor_name": node.state_descriptor_name.as_str(),
            })
        })
        .collect::<Vec<_>>();
    let bridges = draft
        .bridge_nodes()
        .iter()
        .map(|bridge| {
            serde_json::json!({
                "bridge_kind": bridge_kind_name(bridge.bridge_kind),
                "key": bridge.key.as_str(),
                "node_id": bridge.node_id.as_str(),
                "planning_lineage_digest": bridge.planning_lineage.digest.as_str(),
                "policy": bridge_policy_name(bridge.policy),
                "source_cell_id": bridge.source_cell_id.as_str(),
                "source_scope_id": bridge.source_scope_id.as_str(),
                "target_cell_id": bridge.target_cell_id.as_str(),
                "target_scope_id": bridge.target_scope_id.as_str(),
            })
        })
        .collect::<Vec<_>>();
    let public_outputs = draft
        .public_output_spec()
        .outputs()
        .iter()
        .map(|output| {
            serde_json::json!({
                "cell_id": output.cell().cell_id().as_str(),
                "field": output.public_field_path().as_str(),
            })
        })
        .collect::<Vec<_>>();
    let expanded = serde_json::json!({
        "bridges": bridges,
        "canonical_profile_parameters": {},
        "composite_expansion_order": [
            "child_composition",
            "framework_outer",
            "executor_inner",
            "final_ids",
        ],
        "contract_version": "mfm.composite-expanded-program.prototype.v1",
        "framework_policy_refs": [],
        "operations": operations,
        "planner_contract_ref": profile.planner_contract_ref.as_str(),
        "planner_implementation_ref": profile.planner_implementation_ref.as_str(),
        "public_output": {
            "key": draft.public_output_spec().key().as_str(),
            "outputs": public_outputs,
            "schema_id": draft.public_output_spec().public_schema_id().as_str(),
        },
        "root_scope_id": draft.root_scope_id().as_str(),
        "root_scope_key": draft.root_key().as_str(),
        "scopes": scopes,
        "states": states,
    });
    let json =
        serde_json::to_string(&expanded).map_err(|_| PrototypeAdmissionError::ExpansionFailed)?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|_| PrototypeAdmissionError::ExpansionFailed)?;
    ExpandedEvidence::new(canonical)
}

const fn runner_name(runner: RunnerKind) -> &'static str {
    match runner {
        RunnerKind::Pure => "pure",
        RunnerKind::ReadExternal => "read_external",
        RunnerKind::ApplySideEffect => "apply_side_effect",
    }
}

const fn bridge_kind_name(kind: BridgeKind) -> &'static str {
    match kind {
        BridgeKind::ImportFromParent => "import_from_parent",
        BridgeKind::ExportToParent => "export_to_parent",
    }
}

const fn bridge_policy_name(policy: BridgePolicy) -> &'static str {
    match policy {
        BridgePolicy::SameRunSameValueV1 => "same_run_same_value_v1",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrototypeAdmissionError {
    UnknownEntryPoint,
    MalformedAuthoredProgram,
    AuthoredContractMismatch,
    MalformedPlanningProfile,
    PlanningProfileMismatch,
    ExpansionFailed,
    CertificateMismatch,
    RawLibraryPath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeAdmissionAuthority {
    authored_program_bytes: Vec<u8>,
    planning_profile_bytes: Vec<u8>,
    expanded: ExpandedEvidence,
    certificate_bytes: Vec<u8>,
    _seal: AdmissionSeal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AdmissionSeal;

struct PrototypeEntryPointCatalog {
    profile: PrototypePlanningProfile,
}

impl PrototypeEntryPointCatalog {
    fn snapshot_only() -> Self {
        Self {
            profile: PrototypePlanningProfile::exact_snapshot_profile(),
        }
    }

    fn mint(
        &self,
        entry_point_id: &str,
        authored_program_bytes: &[u8],
        planning_profile_bytes: &[u8],
    ) -> Result<PrototypeAdmissionAuthority, PrototypeAdmissionError> {
        if entry_point_id != PUBLISHED_ENTRY_POINT_ID {
            return Err(PrototypeAdmissionError::UnknownEntryPoint);
        }
        let authored: RetainedAuthoredProgram = parse_canonical(authored_program_bytes)
            .map_err(|_| PrototypeAdmissionError::MalformedAuthoredProgram)?;
        authored.validate()?;
        let profile: PrototypePlanningProfile = parse_canonical(planning_profile_bytes)
            .map_err(|_| PrototypeAdmissionError::MalformedPlanningProfile)?;
        if profile != self.profile {
            return Err(PrototypeAdmissionError::PlanningProfileMismatch);
        }

        let expanded = authored.expand(&profile)?;
        let certificate = PrototypeCertificate::new(PrototypeCertificateEvidence {
            certificate_version: CERTIFICATE_VERSION.to_owned(),
            entry_point_id: PUBLISHED_ENTRY_POINT_ID.to_owned(),
            entry_point_operation_id: ENTRY_POINT_OPERATION_ID.to_owned(),
            authored_program_ref: prototype_digest(AUTHORED_PROGRAM_DIGEST_DOMAIN, &authored)
                .map_err(|_| PrototypeAdmissionError::MalformedAuthoredProgram)?,
            planning_profile_schema_id: PLANNING_PROFILE_SCHEMA_ID.to_owned(),
            planning_profile_ref: prototype_digest(PLANNING_PROFILE_DIGEST_DOMAIN, &profile)
                .map_err(|_| PrototypeAdmissionError::MalformedPlanningProfile)?,
            planner_contract_ref: PLANNER_CONTRACT_REF.to_owned(),
            planner_implementation_ref: planner_implementation_ref(),
            expanded_spec_hash: expanded.spec_hash.clone(),
        })?;
        let certificate_bytes = canonical_json(&certificate)
            .map_err(|_| PrototypeAdmissionError::CertificateMismatch)?
            .to_vec();
        Ok(PrototypeAdmissionAuthority {
            authored_program_bytes: authored_program_bytes.to_vec(),
            planning_profile_bytes: planning_profile_bytes.to_vec(),
            expanded,
            certificate_bytes,
            _seal: AdmissionSeal,
        })
    }

    fn verify_and_reexpand(
        &self,
        authority: &PrototypeAdmissionAuthority,
    ) -> Result<(), PrototypeAdmissionError> {
        let authored: RetainedAuthoredProgram = parse_canonical(&authority.authored_program_bytes)
            .map_err(|_| PrototypeAdmissionError::MalformedAuthoredProgram)?;
        authored.validate()?;
        let profile: PrototypePlanningProfile = parse_canonical(&authority.planning_profile_bytes)
            .map_err(|_| PrototypeAdmissionError::MalformedPlanningProfile)?;
        if profile != self.profile {
            return Err(PrototypeAdmissionError::PlanningProfileMismatch);
        }
        let certificate: PrototypeCertificate = parse_canonical(&authority.certificate_bytes)
            .map_err(|_| PrototypeAdmissionError::CertificateMismatch)?;
        certificate.verify()?;

        let authored_ref = prototype_digest(AUTHORED_PROGRAM_DIGEST_DOMAIN, &authored)
            .map_err(|_| PrototypeAdmissionError::MalformedAuthoredProgram)?;
        let profile_ref = prototype_digest(PLANNING_PROFILE_DIGEST_DOMAIN, &profile)
            .map_err(|_| PrototypeAdmissionError::MalformedPlanningProfile)?;
        if certificate.evidence.certificate_version != CERTIFICATE_VERSION
            || certificate.evidence.entry_point_id != PUBLISHED_ENTRY_POINT_ID
            || certificate.evidence.entry_point_operation_id != ENTRY_POINT_OPERATION_ID
            || certificate.evidence.authored_program_ref != authored_ref
            || certificate.evidence.planning_profile_schema_id != PLANNING_PROFILE_SCHEMA_ID
            || certificate.evidence.planning_profile_ref != profile_ref
            || certificate.evidence.planner_contract_ref != PLANNER_CONTRACT_REF
            || certificate.evidence.planner_implementation_ref != planner_implementation_ref()
            || certificate.evidence.expanded_spec_hash != authority.expanded.spec_hash
        {
            return Err(PrototypeAdmissionError::CertificateMismatch);
        }

        let independently_expanded = authored.expand(&profile)?;
        if independently_expanded != authority.expanded {
            return Err(PrototypeAdmissionError::CertificateMismatch);
        }
        Ok(())
    }

    fn reject_raw_library_draft(
        &self,
        draft: &TypedProgramDraft,
    ) -> Result<PrototypeAdmissionAuthority, PrototypeAdmissionError> {
        let _ = draft.root_key();
        Err(PrototypeAdmissionError::RawLibraryPath)
    }

    fn reject_raw_library_certificate(
        &self,
        certificate: &CertifiedTypedSpec,
    ) -> Result<PrototypeAdmissionAuthority, PrototypeAdmissionError> {
        let _ = certificate.spec_hash();
        Err(PrototypeAdmissionError::RawLibraryPath)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrototypeDigestEnvelope {
    domain: String,
    value: serde_json::Value,
}

fn canonical_json<T: Serialize>(value: &T) -> Result<PlainCanonicalJsonBytes, ()> {
    let json = serde_json::to_string(value).map_err(|_| ())?;
    PlainCanonicalJsonBytes::from_json_str(&json).map_err(|_| ())
}

fn canonical_digest_envelope<T: Serialize>(
    domain: &str,
    value: &T,
) -> Result<PlainCanonicalJsonBytes, ()> {
    let envelope = PrototypeDigestEnvelope {
        domain: domain.to_owned(),
        value: serde_json::to_value(value).map_err(|_| ())?,
    };
    canonical_json(&envelope)
}

fn digest_from_canonical_envelope(expected_domain: &str, bytes: &[u8]) -> Result<String, ()> {
    let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes).map_err(|_| ())?;
    let envelope: PrototypeDigestEnvelope =
        serde_json::from_slice(canonical.as_bytes()).map_err(|_| ())?;
    if envelope.domain != expected_domain || canonical_json(&envelope)?.as_bytes() != bytes {
        return Err(());
    }
    Ok(canonical.content_digest().to_string())
}

fn prototype_digest<T: Serialize>(domain: &str, value: &T) -> Result<String, ()> {
    let envelope = canonical_digest_envelope(domain, value)?;
    digest_from_canonical_envelope(domain, envelope.as_bytes())
}

fn planner_implementation_ref() -> String {
    prototype_digest(
        PLANNER_IMPLEMENTATION_DIGEST_DOMAIN,
        &serde_json::json!({"component": "mfm.composite-planner.prototype.v1"}),
    )
    .expect("canonical planner implementation identity envelope")
}

fn raw_content_digest(bytes: &[u8]) -> String {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes)).to_string()
}

fn parse_canonical<T>(bytes: &[u8]) -> Result<T, ()>
where
    T: DeserializeOwned + Serialize,
{
    PlainCanonicalJsonBytes::from_canonical_json_slice(bytes).map_err(|_| ())?;
    let value = serde_json::from_slice::<T>(bytes).map_err(|_| ())?;
    let encoded = canonical_json(&value).map_err(|_| ())?;
    if encoded.as_bytes() != bytes {
        return Err(());
    }
    Ok(value)
}

#[test]
fn exact_snapshot_profile_retains_and_independently_reexpands_authored_bytes() {
    let catalog = PrototypeEntryPointCatalog::snapshot_only();
    let authored = RetainedAuthoredProgram::snapshot(evm_portfolio_config(3));
    let authored_bytes = canonical_json(&authored).expect("canonical authored bytes");
    let profile_bytes = canonical_json(&catalog.profile).expect("canonical profile bytes");
    let profile_json: serde_json::Value =
        serde_json::from_slice(profile_bytes.as_bytes()).expect("profile JSON");
    assert_eq!(profile_json["framework_policy_refs"], serde_json::json!([]));
    assert_eq!(
        profile_json["canonical_profile_parameters"],
        serde_json::json!({})
    );

    let authority = catalog
        .mint(
            PUBLISHED_ENTRY_POINT_ID,
            authored_bytes.as_bytes(),
            profile_bytes.as_bytes(),
        )
        .expect("exact published mapping mints private authority");
    assert_eq!(authority.authored_program_bytes, authored_bytes.as_bytes());
    assert_eq!(authority.planning_profile_bytes, profile_bytes.as_bytes());
    catalog
        .verify_and_reexpand(&authority)
        .expect("independent expansion is byte-identical");
}

#[test]
fn prototype_digests_use_one_canonical_domain_value_envelope() {
    let value = serde_json::json!({"component": "mfm.composite-planner.prototype.v1"});
    let raw_value = canonical_json(&value).expect("canonical raw value");
    let accepted_envelope = canonical_digest_envelope(PLANNER_IMPLEMENTATION_DIGEST_DOMAIN, &value)
        .expect("canonical digest envelope");
    assert_eq!(
        accepted_envelope.as_str(),
        r#"{"domain":"mfm.planner-implementation-ref.prototype.v1","value":{"component":"mfm.composite-planner.prototype.v1"}}"#
    );
    let accepted = prototype_digest(PLANNER_IMPLEMENTATION_DIGEST_DOMAIN, &value)
        .expect("universally enveloped digest");
    assert_eq!(accepted, raw_content_digest(accepted_envelope.as_bytes()));
    assert_eq!(
        digest_from_canonical_envelope(
            PLANNER_IMPLEMENTATION_DIGEST_DOMAIN,
            accepted_envelope.as_bytes(),
        ),
        Ok(accepted.clone())
    );

    let mut prefix_preimage = PLANNER_IMPLEMENTATION_DIGEST_DOMAIN.as_bytes().to_vec();
    prefix_preimage.extend_from_slice(raw_value.as_bytes());
    let mut nul_prefix_preimage = PLANNER_IMPLEMENTATION_DIGEST_DOMAIN.as_bytes().to_vec();
    nul_prefix_preimage.push(0);
    nul_prefix_preimage.extend_from_slice(raw_value.as_bytes());
    let wrong_domain_envelope =
        canonical_digest_envelope("mfm.planner-implementation-ref.wrong.v1", &value)
            .expect("wrong-domain envelope");
    let confused_digests = [
        raw_content_digest(raw_value.as_bytes()),
        raw_content_digest(&prefix_preimage),
        raw_content_digest(&nul_prefix_preimage),
        raw_content_digest(wrong_domain_envelope.as_bytes()),
    ];
    for confused in &confused_digests {
        assert_ne!(&accepted, confused);
    }

    let noncanonical = format!(
        r#"{{"value":{},"domain":"{PLANNER_IMPLEMENTATION_DIGEST_DOMAIN}"}}"#,
        raw_value.as_str()
    );
    let float = format!(r#"{{"domain":"{PLANNER_IMPLEMENTATION_DIGEST_DOMAIN}","value":1.5}}"#);
    let duplicate_key = format!(
        r#"{{"domain":"{PLANNER_IMPLEMENTATION_DIGEST_DOMAIN}","domain":"{PLANNER_IMPLEMENTATION_DIGEST_DOMAIN}","value":{}}}"#,
        raw_value.as_str()
    );
    for hostile in [
        raw_value.as_bytes(),
        prefix_preimage.as_slice(),
        nul_prefix_preimage.as_slice(),
        wrong_domain_envelope.as_bytes(),
        noncanonical.as_bytes(),
        float.as_bytes(),
        duplicate_key.as_bytes(),
    ] {
        assert!(
            digest_from_canonical_envelope(PLANNER_IMPLEMENTATION_DIGEST_DOMAIN, hostile).is_err(),
            "hostile digest representation must reject"
        );
    }

    let catalog = PrototypeEntryPointCatalog::snapshot_only();
    let authored = RetainedAuthoredProgram::snapshot(evm_portfolio_config(1));
    let authored_bytes = canonical_json(&authored).expect("canonical authored bytes");
    let profile_bytes = canonical_json(&catalog.profile).expect("canonical profile bytes");
    let authority = catalog
        .mint(
            PUBLISHED_ENTRY_POINT_ID,
            authored_bytes.as_bytes(),
            profile_bytes.as_bytes(),
        )
        .expect("exact authority");
    let certificate: PrototypeCertificate =
        parse_canonical(&authority.certificate_bytes).expect("retained certificate");
    for confused in confused_digests {
        let mut forged_certificate = certificate.clone();
        forged_certificate.evidence.planner_implementation_ref = confused;
        forged_certificate.certificate_hash =
            prototype_digest(CERTIFICATE_DIGEST_DOMAIN, &forged_certificate.evidence)
                .expect("self-consistent hostile certificate hash");
        let mut forged_authority = authority.clone();
        forged_authority.certificate_bytes = canonical_json(&forged_certificate)
            .expect("forged certificate bytes")
            .to_vec();
        assert_eq!(
            catalog.verify_and_reexpand(&forged_authority),
            Err(PrototypeAdmissionError::CertificateMismatch)
        );
    }
}

#[test]
fn alternate_ids_profiles_and_raw_library_paths_cannot_mint_authority() {
    let catalog = PrototypeEntryPointCatalog::snapshot_only();
    let authored = RetainedAuthoredProgram::snapshot(evm_portfolio_config(1));
    let authored_bytes = canonical_json(&authored).expect("canonical authored bytes");
    let profile_bytes = canonical_json(&catalog.profile).expect("canonical profile bytes");

    assert_eq!(
        catalog.mint(
            "mfm.portfolio/report@1",
            authored_bytes.as_bytes(),
            profile_bytes.as_bytes(),
        ),
        Err(PrototypeAdmissionError::UnknownEntryPoint)
    );

    for substituted in [
        {
            let mut value = authored.clone();
            value.entry_point_operation_id = PUBLISHED_ENTRY_POINT_ID.to_owned();
            value
        },
        {
            let mut value = authored.clone();
            value.entry_point_operation_id = ROOT_OPERATION_NAME.to_owned();
            value
        },
        {
            let mut value = authored.clone();
            value.root_operation_name = ENTRY_POINT_OPERATION_ID.to_owned();
            value
        },
        {
            let mut value = authored.clone();
            value.entry_point_id = ENTRY_POINT_OPERATION_ID.to_owned();
            value
        },
    ] {
        let substituted_bytes =
            canonical_json(&substituted).expect("canonical substituted authored bytes");
        assert_eq!(
            catalog.mint(
                PUBLISHED_ENTRY_POINT_ID,
                substituted_bytes.as_bytes(),
                profile_bytes.as_bytes(),
            ),
            Err(PrototypeAdmissionError::AuthoredContractMismatch)
        );
    }

    let mut alternate_profile = catalog.profile.clone();
    alternate_profile
        .framework_policy_refs
        .push("mfm.framework.unpublished.v1".to_owned());
    let alternate_profile_bytes =
        canonical_json(&alternate_profile).expect("alternate profile bytes");
    assert_eq!(
        catalog.mint(
            PUBLISHED_ENTRY_POINT_ID,
            authored_bytes.as_bytes(),
            alternate_profile_bytes.as_bytes(),
        ),
        Err(PrototypeAdmissionError::PlanningProfileMismatch)
    );

    let mut substituted_profile = catalog.profile.clone();
    substituted_profile.planner_contract_ref = ENTRY_POINT_OPERATION_ID.to_owned();
    let substituted_profile_bytes =
        canonical_json(&substituted_profile).expect("substituted profile bytes");
    assert_eq!(
        catalog.mint(
            PUBLISHED_ENTRY_POINT_ID,
            authored_bytes.as_bytes(),
            substituted_profile_bytes.as_bytes(),
        ),
        Err(PrototypeAdmissionError::PlanningProfileMismatch)
    );

    let mut missing_policy_value = serde_json::to_value(&catalog.profile).expect("profile value");
    missing_policy_value
        .as_object_mut()
        .expect("profile object")
        .remove("framework_policy_refs");
    let missing_policy_list =
        canonical_json(&missing_policy_value).expect("canonicalizable missing-field fixture");
    assert_eq!(
        catalog.mint(
            PUBLISHED_ENTRY_POINT_ID,
            authored_bytes.as_bytes(),
            missing_policy_list.as_bytes(),
        ),
        Err(PrototypeAdmissionError::MalformedPlanningProfile)
    );

    let raw_draft =
        portfolio_snapshot_program_draft(authored.config.clone()).expect("raw library draft");
    let raw_certificate =
        mfm_certify::certify_program_draft(&raw_draft).expect("raw library certificate");
    assert_eq!(
        catalog.reject_raw_library_draft(&raw_draft),
        Err(PrototypeAdmissionError::RawLibraryPath)
    );
    assert_eq!(
        catalog.reject_raw_library_certificate(&raw_certificate),
        Err(PrototypeAdmissionError::RawLibraryPath)
    );
}
