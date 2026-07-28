use mfm_canonical::{CanonicalValue, RecoverabilityContractV1, ValidatedCanonicalValueV1};
use mfm_journal::v1::{ProducerBindingFields, RunPhase};

use super::{
    objects::validate_value_contract, PendingEffectStatus, Result, StoreError, VerifiedRunView,
};

const PUBLIC_RUN_VIEW_CONTRACT: &str = "mfm.public-run-view.v1";
const CANONICAL_VALUE_CONTRACT: &str = "mfm.primitive-canonical_value.v1";

/// One callback-free public projection produced from a completely verified run history.
///
/// Construction is private to the store, and this value is intentionally non-cloneable. It
/// contains only the annex-validated public response and grants no journal or object authority.
///
/// ```compile_fail
/// use mfm_store::VerifiedPublicRunView;
///
/// fn cannot_clone(view: VerifiedPublicRunView) {
///     let _ = view.clone();
/// }
/// ```
///
/// ```compile_fail
/// use mfm_store::VerifiedPublicRunView;
///
/// fn cannot_serialize(view: &VerifiedPublicRunView) {
///     let _ = serde_json::to_vec(view);
/// }
/// ```
pub struct VerifiedPublicRunView {
    validated: ValidatedCanonicalValueV1,
}

impl VerifiedPublicRunView {
    pub(super) fn project(view: &VerifiedRunView) -> Result<Self> {
        let status = if view.run_phase() == RunPhase::Open {
            "active"
        } else if view.terminal_succeeded() {
            "succeeded"
        } else {
            "failed"
        };
        let active = if view.run_phase() == RunPhase::Open {
            active_run_view(view)?
        } else {
            CanonicalValue::Null
        };
        let public_outputs = match (status, view.public_output_value_ref()) {
            ("active" | "failed", None) => Vec::new(),
            ("succeeded", Some(value_ref)) => public_outputs(view, value_ref)?,
            ("active" | "failed", Some(_)) | ("succeeded", None) => {
                return Err(StoreError::PersistedMismatch {
                    field: "public_output_presence",
                });
            }
            _ => {
                return Err(StoreError::JournalContract);
            }
        };
        let value = canonical_object([
            (
                "version",
                CanonicalValue::String(PUBLIC_RUN_VIEW_CONTRACT.to_owned()),
            ),
            (
                "run_id",
                CanonicalValue::String(view.run_id().as_str().to_owned()),
            ),
            ("status", CanonicalValue::String(status.to_owned())),
            ("journal_head", view.journal_head().canonical_value()?),
            ("semantic_head", view.semantic_head().canonical_value()?),
            ("active", active),
            ("public_outputs", CanonicalValue::Array(public_outputs)),
        ])?;
        let validated =
            RecoverabilityContractV1::embedded()?.encode(PUBLIC_RUN_VIEW_CONTRACT, &value)?;
        Ok(Self { validated })
    }

    /// Consumes this sealed projection into its exact annex-validated canonical response.
    pub fn into_validated(self) -> ValidatedCanonicalValueV1 {
        self.validated
    }
}

impl std::fmt::Debug for VerifiedPublicRunView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VerifiedPublicRunView")
            .field("schema_id", self.validated.schema_id())
            .finish_non_exhaustive()
    }
}

fn active_run_view(view: &VerifiedRunView) -> Result<CanonicalValue> {
    let ready_node_ids = view
        .ready_node_ids()
        .into_iter()
        .map(|node_id| CanonicalValue::String(node_id.as_str().to_owned()))
        .collect();
    let pending_effects = view
        .pending_effects()?
        .into_iter()
        .map(|pending| {
            canonical_object([
                (
                    "node_id",
                    CanonicalValue::String(pending.node_id().as_str().to_owned()),
                ),
                (
                    "effect_key",
                    CanonicalValue::String(pending.effect_key().as_str().to_owned()),
                ),
                (
                    "executor_status",
                    CanonicalValue::String(pending_effect_status(pending.status()).to_owned()),
                ),
            ])
        })
        .collect::<Result<Vec<_>>>()?;
    canonical_object([
        ("ready_node_ids", CanonicalValue::Array(ready_node_ids)),
        ("pending_effects", CanonicalValue::Array(pending_effects)),
    ])
}

const fn pending_effect_status(status: PendingEffectStatus) -> &'static str {
    match status {
        PendingEffectStatus::NotAuthorized => "not_authorized",
        PendingEffectStatus::AuthorizedUnobserved => "authorized_unobserved",
        PendingEffectStatus::Returned => "returned",
        PendingEffectStatus::DidNotEnter => "did_not_enter",
        PendingEffectStatus::Indeterminate => "indeterminate",
    }
}

fn public_outputs(
    view: &VerifiedRunView,
    value_ref: &mfm_journal::v1::ValueRef,
) -> Result<Vec<CanonicalValue>> {
    let public_contract = view.certified_spec().public_output_contract();
    validate_value_contract(public_contract.value_contract(), value_ref)?;
    let fields = value_ref.fields()?;
    if !matches!(
        fields.producer_binding.fields()?,
        ProducerBindingFields::PublicOutputAssembly { ref run_id } if run_id == view.run_id()
    ) {
        return Err(StoreError::TransitionFoldMismatch {
            field: "public_output_assembly",
        });
    }
    let object = view.retained_value(value_ref)?;
    let contract = RecoverabilityContractV1::embedded()?;
    let aggregate = contract.strict_decode(CANONICAL_VALUE_CONTRACT, object.bytes())?;

    public_contract
        .bindings()
        .iter()
        .map(|binding| {
            let selected = super::frame_preparation::select_canonical(
                aggregate.as_bytes(),
                Some(binding.destination_field_path()),
            )?;
            let validated =
                contract.strict_decode(CANONICAL_VALUE_CONTRACT, selected.as_bytes())?;
            canonical_object([
                (
                    "name",
                    CanonicalValue::String(binding.destination_field_path().as_str().to_owned()),
                ),
                (
                    "schema_id",
                    CanonicalValue::String(
                        binding.value_contract().schema_id().as_str().to_owned(),
                    ),
                ),
                (
                    "value_digest",
                    CanonicalValue::String(
                        contract
                            .raw_content_digest(selected.as_bytes())
                            .as_str()
                            .to_owned(),
                    ),
                ),
                ("value", validated.canonical_value()?),
            ])
        })
        .collect()
}

fn canonical_object<const N: usize>(
    entries: [(&str, CanonicalValue); N],
) -> Result<CanonicalValue> {
    CanonicalValue::object(entries).map_err(|_| StoreError::JournalContract)
}
