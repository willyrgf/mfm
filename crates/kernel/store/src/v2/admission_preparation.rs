use std::collections::{BTreeMap, BTreeSet};

use mfm_canonical::{CanonicalValue, RecoverabilityContractV3};
use mfm_ids::{AppendRequestId, ContentRef, FieldPath, RunId, StableId};
use mfm_journal::v2::{
    ConfigManifest, ContextManifest, CrossRunSourceManifest, CrossRunSourceManifestEntry,
    GenesisPreimage, InitialBinding, ProducerBinding, RootManifestEntry, RunAdmitted,
    RunAdmittedFields, SeedManifest, ValueRef,
};
use mfm_spec::v1::{
    CapabilityBindingManifest, CertifiedAdmissionArtifacts, CertifiedSourceSelector,
    EntryPointContract, ExpandedCertifiedSpec, RetainedValueContract, StateImplementationManifest,
};

use super::objects::{derive_value_ref, validate_value_contract, PreparedAuthority};
use super::{
    derive_initial_run_state_digest, AdmissionMaterial, AdmitRun, AdmittedSupportGraph, Result,
    StoreAuthorityContext, StoreError, VerifiedAdmissionSources,
};

const EXECUTABLE_IDENTITY_PATH: &str = "executable.identity";
const STATE_MANIFEST_PATH: &str = "manifests.state_implementation";
const CAPABILITY_MANIFEST_PATH: &str = "manifests.capability_binding";
const CONFIG_ROOT_PATH: &str = "configured";
const CONFIG_INITIAL_PATH: &str = "config.configured";
const INPUT_INITIAL_PATH: &str = "run_admission.input";

pub(super) fn prepare_admission_material(
    authority: &StoreAuthorityContext,
    target: &super::authority::AdmitTarget,
    append_request_id: AppendRequestId,
    material: AdmissionMaterial<'_>,
) -> Result<AdmitRun> {
    let (artifacts, input, configured, support, sources) = material.into_parts();
    if !support.belongs_to(authority)
        || !sources.authority.is_same_instance(authority)
        || sources.tenant_scope_id != target.tenant_scope_id
    {
        return Err(StoreError::AdmissionAuthorityMismatch);
    }
    validate_entry_point_target(&artifacts, target)?;
    validate_configured_value(authority, target, configured, artifacts.entry_point())?;
    validate_support_closure(support, &artifacts)?;
    validate_input(&input, artifacts.entry_point())?;

    let mut authorities = support
        .members()
        .values()
        .map(|member| {
            PreparedAuthority::preexisting(member.value_ref().clone(), member.bytes().to_vec())
        })
        .collect::<Result<Vec<_>>>()?;

    let input_ref = derive_value_ref(
        input.value_contract(),
        &ProducerBinding::this_admission(&stable_id("input")?)?,
        input.canonical().as_bytes(),
    )?;
    authorities.push(PreparedAuthority::produced(
        input_ref.clone(),
        input.canonical().to_vec(),
    )?);
    authorities.push(PreparedAuthority::preexisting(
        configured.value_ref().clone(),
        configured.bytes().to_vec(),
    )?);

    let config_manifest = ConfigManifest::new(vec![RootManifestEntry::new(
        field_path(CONFIG_ROOT_PATH)?,
        configured.value_ref().clone(),
    )?])?;
    let seed_roots =
        resolve_manifest_roots(artifacts.expanded_spec(), support, ManifestKind::Seed)?;
    let context_roots =
        resolve_manifest_roots(artifacts.expanded_spec(), support, ManifestKind::Context)?;
    let seed_manifest = SeedManifest::new(
        seed_roots
            .iter()
            .map(|(path, value_ref)| RootManifestEntry::new(path.clone(), value_ref.clone()))
            .collect::<mfm_journal::v2::Result<Vec<_>>>()?,
    )?;
    let context_manifest = ContextManifest::new(
        context_roots
            .iter()
            .map(|(path, value_ref)| RootManifestEntry::new(path.clone(), value_ref.clone()))
            .collect::<mfm_journal::v2::Result<Vec<_>>>()?,
    )?;
    let cross_run_manifest = CrossRunSourceManifest::new(
        sources
            .entries
            .iter()
            .map(|(path, source)| {
                CrossRunSourceManifestEntry::new(path.clone(), source.source.clone())
            })
            .collect::<mfm_journal::v2::Result<Vec<_>>>()?,
    )?;

    let mut initial = BTreeMap::<FieldPath, (ValueRef, ContentRef)>::new();
    insert_initial(
        &mut initial,
        field_path(INPUT_INITIAL_PATH)?,
        input_ref,
        input.value_contract().evidence_contract_ref().clone(),
    )?;
    insert_initial(
        &mut initial,
        field_path(CONFIG_INITIAL_PATH)?,
        configured.value_ref().clone(),
        configured.value_ref().fields()?.evidence_contract_ref,
    )?;
    for (path, value_ref) in &seed_roots {
        insert_initial(
            &mut initial,
            prefixed_path("seed", path)?,
            value_ref.clone(),
            value_ref.fields()?.evidence_contract_ref,
        )?;
    }
    for (path, value_ref) in &context_roots {
        insert_initial(
            &mut initial,
            prefixed_path("context", path)?,
            value_ref.clone(),
            value_ref.fields()?.evidence_contract_ref,
        )?;
    }
    for (path, source) in &sources.entries {
        insert_initial(
            &mut initial,
            prefixed_path("cross_run", path)?,
            source.value_ref.clone(),
            source.source_role_ref.clone(),
        )?;
        authorities.push(PreparedAuthority::produced(
            source.value_ref.clone(),
            source.bytes.clone(),
        )?);
    }
    for path in required_support_paths(artifacts.expanded_spec())? {
        let member = support
            .member(&path)
            .ok_or(StoreError::InvalidPreparedAppend {
                purpose: "admit_run",
                message: "certified qualified-support source is absent",
            })?;
        insert_initial(
            &mut initial,
            path,
            member.value_ref().clone(),
            member.value_ref().fields()?.evidence_contract_ref,
        )?;
    }
    validate_source_totality(
        artifacts.expanded_spec(),
        sources,
        &seed_roots,
        &context_roots,
    )?;
    let initial_bindings = initial
        .into_iter()
        .map(|(path, (value_ref, source_role_ref))| {
            InitialBinding::new(&path, &value_ref, &source_role_ref)
        })
        .collect::<mfm_journal::v2::Result<Vec<_>>>()?;

    let entry_point_ref = artifacts.entry_point().content_ref()?;
    let authored_program_ref = artifacts.authored_program().content_ref()?;
    let certified_spec_ref = artifacts.expanded_spec().content_ref()?;
    let certificate_ref = artifacts.certificate().content_ref()?;
    let state_manifest_ref = artifacts.state_manifest().content_ref()?;
    let capability_manifest_ref = artifacts.capability_manifest().content_ref()?;

    push_artifact(
        &mut authorities,
        "entry_point",
        &EntryPointContract::retained_contract()?,
        artifacts.entry_point().canonical_json()?.as_bytes(),
        &entry_point_ref,
    )?;
    push_artifact(
        &mut authorities,
        "authored_program",
        &mfm_spec::v1::CanonicalAuthoredProgram::retained_contract()?,
        artifacts.authored_program().canonical_json()?.as_bytes(),
        &authored_program_ref,
    )?;
    push_artifact(
        &mut authorities,
        "expanded_spec",
        &ExpandedCertifiedSpec::retained_contract()?,
        artifacts.expanded_spec().canonical_json()?.as_bytes(),
        &certified_spec_ref,
    )?;
    push_artifact(
        &mut authorities,
        "certificate",
        &mfm_spec::v1::Certificate::retained_contract()?,
        artifacts.certificate().canonical_json()?.as_bytes(),
        &certificate_ref,
    )?;
    push_artifact(
        &mut authorities,
        "state_manifest",
        &StateImplementationManifest::retained_contract()?,
        artifacts.state_manifest().canonical_json()?.as_bytes(),
        &state_manifest_ref,
    )?;
    push_artifact(
        &mut authorities,
        "capability_manifest",
        &CapabilityBindingManifest::retained_contract()?,
        artifacts.capability_manifest().canonical_json()?.as_bytes(),
        &capability_manifest_ref,
    )?;

    let config_manifest_ref = push_journal_artifact(
        &mut authorities,
        "config_manifest",
        &ConfigManifest::retained_contract()?,
        config_manifest.as_bytes(),
    )?;
    let seed_manifest_ref = push_journal_artifact(
        &mut authorities,
        "seed_manifest",
        &SeedManifest::retained_contract()?,
        seed_manifest.as_bytes(),
    )?;
    let context_manifest_ref = push_journal_artifact(
        &mut authorities,
        "context_manifest",
        &ContextManifest::retained_contract()?,
        context_manifest.as_bytes(),
    )?;
    let cross_run_source_manifest_ref = push_journal_artifact(
        &mut authorities,
        "cross_run_source_manifest",
        &CrossRunSourceManifest::retained_contract()?,
        cross_run_manifest.as_bytes(),
    )?;

    let executable_identity_ref =
        support_content_ref(support_member(support, EXECUTABLE_IDENTITY_PATH)?.value_ref())?;
    let run_id = derive_run_id(authority, target)?;
    let genesis_digest = GenesisPreimage::new(
        authority.store_identity().store_scope_id(),
        authority.store_identity().store_epoch(),
        &run_id,
    )?
    .genesis_digest()?;
    let initial_run_state_digest =
        derive_initial_run_state_digest(artifacts.expanded_spec(), &initial_bindings)?;
    let admission = RunAdmitted::new(&RunAdmittedFields {
        run_id,
        tenant_scope_id: target.tenant_scope_id.clone(),
        invocation_identity: target.invocation_identity.clone(),
        entry_point_operation_id: target.entry_point_operation_id.clone(),
        operation_contract_ref: entry_point_ref,
        executable_identity_ref,
        spec_hash: artifacts.expanded_spec().spec_hash()?,
        certified_spec_ref,
        certificate_ref,
        state_implementation_manifest_ref: state_manifest_ref,
        capability_binding_manifest_ref: capability_manifest_ref,
        config_manifest_ref,
        seed_manifest_ref,
        context_manifest_ref,
        cross_run_source_manifest_ref,
        initial_bindings,
        genesis_digest,
        initial_run_state_digest,
    })?;
    let objects = super::PreparedObjectGraph::prepare_for_record_values(
        &[admission.canonical_value()?],
        authorities,
    )?;
    AdmitRun::new(
        authority.store_identity(),
        append_request_id,
        admission,
        objects,
    )
}

fn validate_entry_point_target(
    artifacts: &CertifiedAdmissionArtifacts,
    target: &super::authority::AdmitTarget,
) -> Result<()> {
    let entry_point = artifacts.entry_point();
    if entry_point.entry_point_id() != &target.entry_point_id
        || entry_point.entry_point_operation_id() != &target.entry_point_operation_id
        || artifacts.authored_program().entry_point_operation_id()
            != &target.entry_point_operation_id
    {
        return Err(StoreError::AdmissionAuthorityMismatch);
    }
    Ok(())
}

fn validate_configured_value(
    authority: &StoreAuthorityContext,
    target: &super::authority::AdmitTarget,
    configured: &super::VerifiedConfiguredValue,
    entry_point: &EntryPointContract,
) -> Result<()> {
    let key = configured.binding().fields()?.key.fields()?;
    if key.store_scope_id != *authority.store_identity().store_scope_id()
        || key.tenant_scope_id != target.tenant_scope_id
        || key.entry_point_id != target.entry_point_id
        || entry_point.entry_point_id() != &key.entry_point_id
        || configured.binding().fields()?.value_ref != *configured.value_ref()
    {
        return Err(StoreError::AdmissionAuthorityMismatch);
    }
    let fields = configured.value_ref().fields()?;
    RecoverabilityContractV3::embedded()?
        .strict_decode_schema_id(&fields.schema_id, configured.bytes())?;
    Ok(())
}

fn validate_input(
    input: &super::ProposedAdmissionInput,
    entry_point: &EntryPointContract,
) -> Result<()> {
    if input.value_contract().schema_id() != entry_point.input_schema_id() {
        return Err(StoreError::InvalidPreparedAppend {
            purpose: "admit_run",
            message: "input contract differs from the entry-point input schema",
        });
    }
    RecoverabilityContractV3::embedded()?.strict_decode_schema_id(
        input.value_contract().schema_id(),
        input.canonical().as_bytes(),
    )?;
    Ok(())
}

fn validate_support_closure(
    support: &AdmittedSupportGraph,
    artifacts: &CertifiedAdmissionArtifacts,
) -> Result<()> {
    let executable = support_member(support, EXECUTABLE_IDENTITY_PATH)?;
    let executable_fields = executable.value_ref().fields()?;
    if executable_fields.role.as_str() != "mfm.qualification.executable-identity" {
        return Err(StoreError::InvalidPreparedAppend {
            purpose: "admit_run",
            message: "qualified support has no exact executable identity",
        });
    }
    require_support_artifact(
        support,
        STATE_MANIFEST_PATH,
        &artifacts.state_manifest().content_ref()?,
        artifacts.state_manifest().canonical_json()?.as_bytes(),
    )?;
    require_support_artifact(
        support,
        CAPABILITY_MANIFEST_PATH,
        &artifacts.capability_manifest().content_ref()?,
        artifacts.capability_manifest().canonical_json()?.as_bytes(),
    )?;

    let available = support
        .members()
        .values()
        .map(|member| support_content_ref(member.value_ref()))
        .collect::<Result<BTreeSet<_>>>()?;
    let profile = artifacts.entry_point().planning_profile();
    let required = std::iter::once(profile.planner_contract_ref())
        .chain(std::iter::once(profile.planner_implementation_ref()))
        .chain(profile.framework_policy_refs())
        .chain(
            artifacts
                .state_manifest()
                .entries()
                .iter()
                .flat_map(|entry| {
                    [
                        &entry.state_contract_ref,
                        &entry.component_implementation_ref,
                    ]
                }),
        )
        .chain(
            artifacts
                .capability_manifest()
                .entries()
                .iter()
                .map(|entry| &entry.binding_ref),
        );
    if required
        .into_iter()
        .any(|reference| !available.contains(reference))
    {
        return Err(StoreError::InvalidPreparedAppend {
            purpose: "admit_run",
            message: "certified component closure escapes admitted support",
        });
    }
    Ok(())
}

fn require_support_artifact(
    support: &AdmittedSupportGraph,
    path: &str,
    expected_ref: &ContentRef,
    expected_bytes: &[u8],
) -> Result<()> {
    let member = support_member(support, path)?;
    if &support_content_ref(member.value_ref())? != expected_ref || member.bytes() != expected_bytes
    {
        return Err(StoreError::InvalidPreparedAppend {
            purpose: "admit_run",
            message: "certified manifest differs from admitted support",
        });
    }
    Ok(())
}

fn required_support_paths(spec: &ExpandedCertifiedSpec) -> Result<BTreeSet<FieldPath>> {
    let mut paths = BTreeSet::new();
    for (_, selector, _) in certified_sources(spec) {
        if let CertifiedSourceSelector::QualifiedSupport { member_path, .. } = selector {
            paths.insert(member_path.clone());
        }
    }
    Ok(paths)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ManifestKind {
    Seed,
    Context,
}

fn resolve_manifest_roots(
    spec: &ExpandedCertifiedSpec,
    support: &AdmittedSupportGraph,
    kind: ManifestKind,
) -> Result<BTreeMap<FieldPath, ValueRef>> {
    let mut roots = BTreeMap::new();
    for (expected, selector, _) in certified_sources(spec) {
        let selected_path = match (kind, selector) {
            (ManifestKind::Seed, CertifiedSourceSelector::Seed { source_field_path })
            | (ManifestKind::Context, CertifiedSourceSelector::Context { source_field_path }) => {
                source_field_path.as_ref()
            }
            _ => continue,
        };
        let mut matches = support
            .members()
            .iter()
            .filter_map(|(path, member)| {
                let nested = match selected_path {
                    Some(selected) if selected == path => None,
                    Some(selected) => selected
                        .as_str()
                        .strip_prefix(path.as_str())
                        .and_then(|suffix| suffix.strip_prefix('.'))
                        .and_then(|suffix| FieldPath::new(suffix).ok()),
                    None => None,
                };
                if selected_path.is_some() && nested.is_none() && selected_path != Some(path) {
                    return None;
                }
                let selected =
                    super::frame_preparation::select_canonical(member.bytes(), nested.as_ref())
                        .ok()?;
                RecoverabilityContractV3::embedded()
                    .ok()?
                    .strict_decode_schema_id(expected.schema_id(), selected.as_bytes())
                    .ok()?;
                Some((path.clone(), member.value_ref().clone()))
            })
            .collect::<Vec<_>>();
        if selected_path.is_none() {
            matches.retain(|(_, value_ref)| validate_value_contract(expected, value_ref).is_ok());
        }
        if matches.len() != 1 {
            return Err(StoreError::InvalidPreparedAppend {
                purpose: "admit_run",
                message: "certified seed or context source is ambiguous",
            });
        }
        let (path, value_ref) = matches.pop().ok_or(StoreError::InvalidPreparedAppend {
            purpose: "admit_run",
            message: "certified seed or context source is absent",
        })?;
        if roots
            .insert(path, value_ref.clone())
            .is_some_and(|existing| existing != value_ref)
        {
            return Err(StoreError::InvalidPreparedAppend {
                purpose: "admit_run",
                message: "certified seed or context roots conflict",
            });
        }
    }
    Ok(roots)
}

fn validate_source_totality(
    spec: &ExpandedCertifiedSpec,
    sources: &VerifiedAdmissionSources,
    seed_roots: &BTreeMap<FieldPath, ValueRef>,
    context_roots: &BTreeMap<FieldPath, ValueRef>,
) -> Result<()> {
    let has_seed = certified_sources(spec)
        .any(|(_, source, _)| matches!(source, CertifiedSourceSelector::Seed { .. }));
    let has_context = certified_sources(spec)
        .any(|(_, source, _)| matches!(source, CertifiedSourceSelector::Context { .. }));
    if has_seed == seed_roots.is_empty() || has_context == context_roots.is_empty() {
        return Err(StoreError::InvalidPreparedAppend {
            purpose: "admit_run",
            message: "admission manifests differ from certified sources",
        });
    }

    let mut used = BTreeSet::new();
    for (expected, selector, evidence_role) in certified_sources(spec) {
        let (selected_path, required_role) = match selector {
            CertifiedSourceSelector::CrossRunEffectiveOutput { source_field_path } => {
                (source_field_path.as_ref(), None)
            }
            CertifiedSourceSelector::CrossRunEvidence {
                source_field_path,
                certified_evidence_role_ref,
            } => (
                source_field_path.as_ref(),
                Some(certified_evidence_role_ref),
            ),
            _ => continue,
        };
        let mut matches = sources.entries.iter().filter(|(path, source)| {
            let role_matches = required_role
                .or(evidence_role)
                .is_none_or(|role| &source.source_role_ref == role);
            let path_matches = selected_path.is_none_or(|selected| {
                selected == *path
                    || selected
                        .as_str()
                        .strip_prefix(path.as_str())
                        .is_some_and(|suffix| suffix.starts_with('.'))
            });
            role_matches
                && path_matches
                && source.value_contract.schema_id() == expected.schema_id()
        });
        let Some((path, _)) = matches.next() else {
            return Err(StoreError::InvalidPreparedAppend {
                purpose: "admit_run",
                message: "certified cross-run source is absent",
            });
        };
        if matches.next().is_some() {
            return Err(StoreError::InvalidPreparedAppend {
                purpose: "admit_run",
                message: "certified cross-run source is ambiguous",
            });
        }
        used.insert(path.clone());
    }
    if used.len() != sources.entries.len() {
        return Err(StoreError::InvalidPreparedAppend {
            purpose: "admit_run",
            message: "admission contains an uncertified cross-run source",
        });
    }
    Ok(())
}

fn certified_sources(
    spec: &ExpandedCertifiedSpec,
) -> impl Iterator<
    Item = (
        &RetainedValueContract,
        &CertifiedSourceSelector,
        Option<&ContentRef>,
    ),
> {
    spec.nodes().iter().flat_map(|node| {
        std::iter::once((
            node.config_binding().value_contract(),
            node.config_binding().source(),
            None,
        ))
        .chain(
            node.context_binding()
                .map(|binding| (binding.value_contract(), binding.source(), None)),
        )
        .chain(node.input_bindings().iter().flat_map(|binding| {
            binding.ordered_sources().iter().map(|source| {
                (
                    binding.value_contract(),
                    source,
                    match binding.destination() {
                        mfm_spec::v1::CertifiedInputDestination::EvidenceOnly {
                            certified_evidence_role_ref,
                        } => Some(certified_evidence_role_ref),
                        mfm_spec::v1::CertifiedInputDestination::OrdinaryValue => None,
                    },
                )
            })
        }))
    })
}

fn push_artifact(
    authorities: &mut Vec<PreparedAuthority>,
    slot: &str,
    contract: &RetainedValueContract,
    bytes: &[u8],
    expected_ref: &ContentRef,
) -> Result<()> {
    let actual = push_journal_artifact(authorities, slot, contract, bytes)?;
    if &actual != expected_ref {
        return Err(StoreError::InvalidPreparedAppend {
            purpose: "admit_run",
            message: "certified artifact content reference does not rederive",
        });
    }
    Ok(())
}

fn push_journal_artifact(
    authorities: &mut Vec<PreparedAuthority>,
    slot: &str,
    contract: &RetainedValueContract,
    bytes: &[u8],
) -> Result<ContentRef> {
    let value_ref = derive_value_ref(
        contract,
        &ProducerBinding::this_admission(&stable_id(slot)?)?,
        bytes,
    )?;
    let reference = support_content_ref(&value_ref)?;
    authorities.push(PreparedAuthority::produced(value_ref, bytes.to_vec())?);
    Ok(reference)
}

fn derive_run_id(
    authority: &StoreAuthorityContext,
    target: &super::authority::AdmitTarget,
) -> Result<RunId> {
    let value = CanonicalValue::object([
        (
            "store_scope_id",
            CanonicalValue::String(
                authority
                    .store_identity()
                    .store_scope_id()
                    .as_str()
                    .to_owned(),
            ),
        ),
        (
            "tenant_scope_id",
            CanonicalValue::String(target.tenant_scope_id.as_str().to_owned()),
        ),
        (
            "entry_point_operation_id",
            CanonicalValue::String(target.entry_point_operation_id.as_str().to_owned()),
        ),
        (
            "invocation_identity",
            CanonicalValue::String(target.invocation_identity.as_str().to_owned()),
        ),
    ])
    .map_err(|_| StoreError::JournalContract)?;
    let contract = RecoverabilityContractV3::embedded()?;
    let logical = contract.encode("mfm.admission-logical-key-preimage.v1", &value)?;
    contract.derive_admission_logical_key(&logical)?;
    let run = contract.encode("mfm.run-id-preimage.v1", &value)?;
    contract.derive_run_id(&run).map_err(Into::into)
}

fn insert_initial(
    initial: &mut BTreeMap<FieldPath, (ValueRef, ContentRef)>,
    path: FieldPath,
    value_ref: ValueRef,
    source_role_ref: ContentRef,
) -> Result<()> {
    if initial.insert(path, (value_ref, source_role_ref)).is_some() {
        return Err(StoreError::InvalidPreparedAppend {
            purpose: "admit_run",
            message: "admission has duplicate initial binding paths",
        });
    }
    Ok(())
}

fn support_member<'a>(
    support: &'a AdmittedSupportGraph,
    path: &str,
) -> Result<&'a super::AdmittedSupportMember> {
    support
        .member(&field_path(path)?)
        .ok_or(StoreError::InvalidPreparedAppend {
            purpose: "admit_run",
            message: "admitted support member is absent",
        })
}

fn support_content_ref(value_ref: &ValueRef) -> Result<ContentRef> {
    let fields = value_ref.fields()?;
    ContentRef::new(fields.schema_id, fields.content_digest).map_err(Into::into)
}

fn prefixed_path(prefix: &str, path: &FieldPath) -> Result<FieldPath> {
    field_path(format!("{prefix}.{}", path.as_str()))
}

fn field_path(value: impl Into<String>) -> Result<FieldPath> {
    FieldPath::new(value.into()).map_err(Into::into)
}

fn stable_id(value: &str) -> Result<StableId> {
    StableId::new(value).map_err(Into::into)
}
