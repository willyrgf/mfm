use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mfm_ids::{ContentRef, StableId};
use mfm_program::{
    ProgramError, QualifiedCertificationCallback, QualifiedCertificationFactory,
    QualifiedProgramDefinition,
};
use mfm_spec::{
    CanonicalAuthoredProgram, CapabilityBindingManifest, Certificate, CertificateProofEntry,
    CertifiedAdmissionArtifacts, EntryPointContract, StateImplementationManifest,
};

use crate::{validate_terminal_totality, CertifyError, CompositePlanner, PlanningCatalog, Result};

/// Zero-state factory for the sole composite certifier.
#[derive(Debug, Default)]
pub struct CompositeCertificationFactory;

impl QualifiedCertificationFactory for CompositeCertificationFactory {
    fn create(
        &self,
        definition: Arc<QualifiedProgramDefinition>,
    ) -> mfm_program::Result<Arc<dyn QualifiedCertificationCallback>> {
        let catalog = PlanningCatalog::from_definition(definition).map_err(|_| {
            ProgramError::Registry("invalid composite planning definition".to_owned())
        })?;
        Ok(Arc::new(QualifiedCertifier { catalog }))
    }
}

struct QualifiedCertifier {
    catalog: PlanningCatalog,
}

impl QualifiedCertificationCallback for QualifiedCertifier {
    fn certify(
        &self,
        entry_point: &EntryPointContract,
        authored_program: &CanonicalAuthoredProgram,
    ) -> mfm_spec::Result<CertifiedAdmissionArtifacts> {
        certify_entry_point(entry_point, authored_program, &self.catalog)
            .map_err(|error| mfm_spec::SpecError::Invariant(error.to_string()))
    }
}

pub(crate) fn certify_entry_point(
    entry_point: &EntryPointContract,
    authored_program: &CanonicalAuthoredProgram,
    catalog: &PlanningCatalog,
) -> Result<CertifiedAdmissionArtifacts> {
    let expanded_spec = CompositePlanner::new(catalog).expand(entry_point, authored_program)?;
    let state_manifest = catalog.current_state_manifest()?;
    let capability_manifest = catalog.current_capability_manifest()?;
    validate_terminal_totality(&expanded_spec)?;
    validate_state_manifest(&expanded_spec, state_manifest, catalog)?;
    validate_capability_manifest(&expanded_spec, capability_manifest, catalog)?;

    let entry_point_ref = entry_point.content_ref()?;
    let authored_program_ref = authored_program.content_ref()?;
    let planning_profile_ref = entry_point.planning_profile().content_ref()?;
    if &planning_profile_ref != entry_point.planning_profile_ref() {
        return Err(CertifyError::Certification(
            "entry point profile reference is not exact".to_owned(),
        ));
    }
    let state_manifest_ref = state_manifest.content_ref()?;
    let capability_manifest_ref = capability_manifest.content_ref()?;
    let expanded_spec_ref = expanded_spec.content_ref()?;
    let proof_closure = proof_closure(
        &entry_point_ref,
        &authored_program_ref,
        &planning_profile_ref,
        &state_manifest_ref,
        &capability_manifest_ref,
        &expanded_spec_ref,
        entry_point,
        state_manifest,
        capability_manifest,
        catalog,
    )?;
    let certificate =
        Certificate::new(expanded_spec.spec_hash()?, expanded_spec_ref, proof_closure)?;
    CertifiedAdmissionArtifacts::from_certification(
        entry_point.clone(),
        authored_program.clone(),
        expanded_spec,
        certificate,
        state_manifest.clone(),
        capability_manifest.clone(),
    )
    .map_err(Into::into)
}

fn validate_state_manifest(
    spec: &mfm_spec::ExpandedCertifiedSpec,
    manifest: &StateImplementationManifest,
    catalog: &PlanningCatalog,
) -> Result<()> {
    let expected = catalog.expected_state_implementations(spec)?;
    let actual = manifest
        .entries()
        .iter()
        .map(|entry| {
            (
                entry.state_contract_ref.clone(),
                entry.component_implementation_ref.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    if actual != expected {
        return Err(CertifyError::Certification(
            "state implementation manifest is not total and exact".to_owned(),
        ));
    }
    Ok(())
}

fn validate_capability_manifest(
    spec: &mfm_spec::ExpandedCertifiedSpec,
    manifest: &CapabilityBindingManifest,
    catalog: &PlanningCatalog,
) -> Result<()> {
    let expected = catalog.expected_operation_bindings(spec)?;
    let actual = manifest
        .entries()
        .iter()
        .map(|entry| (entry.operation_id.clone(), entry.binding_ref.clone()))
        .collect::<BTreeMap<_, _>>();
    if actual != expected {
        return Err(CertifyError::Certification(
            "capability binding manifest is not total and exact".to_owned(),
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn proof_closure(
    entry_point_ref: &ContentRef,
    authored_program_ref: &ContentRef,
    planning_profile_ref: &ContentRef,
    state_manifest_ref: &ContentRef,
    capability_manifest_ref: &ContentRef,
    spec_ref: &ContentRef,
    entry_point: &EntryPointContract,
    state_manifest: &StateImplementationManifest,
    capability_manifest: &CapabilityBindingManifest,
    catalog: &PlanningCatalog,
) -> Result<Vec<CertificateProofEntry>> {
    let mut entries = vec![
        proof(
            "entry-point-profile",
            entry_point_ref.clone(),
            planning_profile_ref.clone(),
        )?,
        proof(
            "authored-program",
            spec_ref.clone(),
            authored_program_ref.clone(),
        )?,
        proof(
            "planner-implementation",
            catalog.planner_contract_ref().clone(),
            catalog.planner_implementation_ref().clone(),
        )?,
        proof(
            "state-manifest",
            spec_ref.clone(),
            state_manifest_ref.clone(),
        )?,
        proof(
            "capability-manifest",
            spec_ref.clone(),
            capability_manifest_ref.clone(),
        )?,
        proof("expanded-spec", spec_ref.clone(), spec_ref.clone())?,
    ];
    entries.extend(
        entry_point
            .planning_profile()
            .framework_policy_refs()
            .iter()
            .map(|policy_ref| {
                proof(
                    "framework-policy",
                    policy_ref.clone(),
                    planning_profile_ref.clone(),
                )
            })
            .collect::<Result<Vec<_>>>()?,
    );
    entries.extend(
        state_manifest
            .entries()
            .iter()
            .map(|entry| {
                proof(
                    "state-implementation",
                    entry.state_contract_ref.clone(),
                    entry.component_implementation_ref.clone(),
                )
            })
            .collect::<Result<Vec<_>>>()?,
    );
    entries.extend(
        capability_manifest
            .entries()
            .iter()
            .map(|entry| entry.binding_ref.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|binding_ref| {
                proof(
                    "capability-binding",
                    binding_ref,
                    capability_manifest_ref.clone(),
                )
            })
            .collect::<Result<Vec<_>>>()?,
    );
    Ok(entries)
}

fn proof(
    kind: &str,
    subject_ref: ContentRef,
    evidence_ref: ContentRef,
) -> Result<CertificateProofEntry> {
    Ok(CertificateProofEntry {
        proof_kind: StableId::new(kind)
            .map_err(|error| CertifyError::Certification(error.to_string()))?,
        subject_ref,
        evidence_ref,
    })
}
