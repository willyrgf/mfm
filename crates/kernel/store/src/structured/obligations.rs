//! Consuming discharge of the complete external obligation set.

use mfm_facts::{prior_run_fact_selector_contract_ref, FactSelectionRequest};
use mfm_journal::structured::PriorRunFactScannerBindingCertificate;
use mfm_spec::structured::{
    prior_run_fact_scanner_adapter_contract, prior_run_fact_selection_capability_contract,
};
use mfm_values::CanonicalJsonPersistedSchema;

use super::compiler::{ComparedReduction, CompiledAppend};
use super::qualification::{
    invalid, PhysicalBindingAuthorization, PhysicalBindingSupersession, PhysicalObligationChecker,
    QualifiedRunContext, StructuredStoreError,
};
use super::reducer::{ReducedRunState, SemanticObligation};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ObligationDischargeScope {
    RetainedOnly,
    RetainedAndCurrent,
}

pub(super) struct FinalizedReduction {
    reduced: Box<ReducedRunState>,
    compiled: CompiledAppend,
}

impl FinalizedReduction {
    pub(super) fn into_reduced(self) -> Box<ReducedRunState> {
        self.reduced
    }

    pub(super) const fn tenant_fact_plan(
        &self,
    ) -> &super::validated_append::TenantFactProjectionPlan {
        self.compiled.tenant_fact_plan()
    }

    pub(super) fn into_parts(self) -> (Box<ReducedRunState>, CompiledAppend) {
        (self.reduced, self.compiled)
    }
}

pub(super) struct DischargePassed(());

pub(super) fn discharge(
    compared: ComparedReduction,
    context: &QualifiedRunContext,
    checker: &dyn PhysicalObligationChecker,
    scope: ObligationDischargeScope,
) -> Result<FinalizedReduction, StructuredStoreError> {
    for obligation in compared.obligations() {
        discharge_obligation(obligation, context, checker, false)?;
    }
    if scope == ObligationDischargeScope::RetainedAndCurrent {
        for obligation in compared.obligations() {
            discharge_obligation(obligation, context, checker, true)?;
        }
    }
    let (reduced, compiled) = compared.into_finalization_parts(DischargePassed(()));
    Ok(FinalizedReduction { reduced, compiled })
}

fn discharge_obligation(
    obligation: &SemanticObligation,
    context: &QualifiedRunContext,
    checker: &dyn PhysicalObligationChecker,
    current: bool,
) -> Result<(), StructuredStoreError> {
    match obligation {
        SemanticObligation::PhysicalAuthorization {
            access_kind,
            capability_contract_ref,
            capability_implementation_ref,
            adapter_contract_ref,
            adapter_implementation_ref,
            admitted_routing_policy_ref,
            stable_resource_lineage_contract_ref,
            minimum_lineage_head_ref,
            previous_physical_binding_ref,
            certificate_ref,
        } => {
            let inputs = PhysicalBindingAuthorization {
                access_kind: *access_kind,
                capability_contract_ref,
                capability_implementation_ref,
                adapter_contract_ref,
                adapter_implementation_ref,
                admitted_routing_policy_ref,
                stable_resource_lineage_contract_ref: stable_resource_lineage_contract_ref.as_ref(),
                minimum_lineage_head_ref: minimum_lineage_head_ref.as_ref(),
                previous_physical_binding_ref: previous_physical_binding_ref.as_ref(),
            };
            let certificate = context.object(certificate_ref).ok_or_else(invalid)?;
            if current {
                checker.verify_current_authorization(&inputs, certificate)
            } else {
                checker.verify_retained_authorization(&inputs, certificate)
            }
        }
        SemanticObligation::PhysicalSupersession {
            capability_contract_ref,
            adapter_contract_ref,
            adapter_implementation_ref,
            authorized_binding_ref,
            stable_resource_lineage_contract_ref,
            public_lineage_head_ref,
            evidence_ref,
        } => {
            let inputs = PhysicalBindingSupersession {
                capability_contract_ref,
                adapter_contract_ref,
                adapter_implementation_ref,
                authorized_binding_ref,
                stable_resource_lineage_contract_ref,
                public_lineage_head_ref,
            };
            let head = context
                .object(public_lineage_head_ref)
                .ok_or_else(invalid)?;
            let evidence = context.object(evidence_ref).ok_or_else(invalid)?;
            if current {
                checker.verify_current_supersession(&inputs, head, evidence)
            } else {
                checker.verify_retained_supersession(&inputs, head, evidence)
            }
        }
        SemanticObligation::PriorRunSelection {
            request,
            certificate_ref,
        } => (!current)
            .then(|| validate_fact_selection(context, request, certificate_ref))
            .transpose()
            .map(|_| ()),
    }
}

fn validate_fact_selection(
    context: &QualifiedRunContext,
    request_ref: &mfm_journal::structured::TypedValueRef,
    certificate_ref: &mfm_ids::ContentRef,
) -> Result<(), StructuredStoreError> {
    let request_object = context.object(&request_ref.value_ref).ok_or_else(invalid)?;
    let request =
        FactSelectionRequest::from_canonical_json(request_object.canonical_json.as_bytes())
            .map_err(|_| invalid())?;
    let certificate = context
        .object(certificate_ref)
        .ok_or_else(invalid)?
        .decode_persisted::<PriorRunFactScannerBindingCertificate>()
        .map_err(|_| invalid())?;
    let capability = prior_run_fact_selection_capability_contract()
        .map_err(|_| invalid())?
        .content_ref()
        .map_err(|_| invalid())?;
    let adapter = prior_run_fact_scanner_adapter_contract()
        .map_err(|_| invalid())?
        .content_ref()
        .map_err(|_| invalid())?;
    if request.admitted_source_manifest_ref()
        != &context
            .admission
            .admission_material_refs
            .prior_run_source_manifest_ref
        || request.selector_contract_ref()
            != &prior_run_fact_selector_contract_ref().map_err(|_| invalid())?
        || certificate.store_scope_id() != &context.admission.store_scope_id
        || certificate.store_epoch() != context.admission.store_epoch
        || certificate.tenant_scope_id() != &context.admission.tenant_scope_id
        || certificate.capability_contract_ref() != &capability
        || certificate.adapter_contract_ref() != &adapter
    {
        return Err(invalid());
    }
    Ok(())
}
