//! Qualified product registration for EVM transaction submission.

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{ContentRef, FieldPath};
use mfm_program::{
    decode_boundary, unit_config_value_contract, ProgramError, QualifiedAuthoringInputs,
    QualifiedAuthoringRootRequirement, QualifiedEntryPointInputContract,
    QualifiedEntryPointRegistration, QualifiedSourceContract, Result, UnitConfig,
};
use mfm_values::RetainedValueContract;

use crate::{
    evm_submit_transaction_entry_point_contract, evm_submit_transaction_value_contracts,
    operation::evm_submit_transaction_authored_program,
};

/// Fixed support member containing the wallet entry point's unit configuration.
pub const EVM_SUBMIT_TRANSACTION_UNIT_CONFIG_MEMBER_PATH: &str = "framework/unit-config";

/// Product-owned identities required to register the wallet entry point.
pub struct EvmSubmitTransactionEntryPointArtifacts {
    planner_contract_ref: ContentRef,
    planner_implementation_ref: ContentRef,
    object_evidence_contract_ref: ContentRef,
    unit_config_contract: RetainedValueContract,
}

impl EvmSubmitTransactionEntryPointArtifacts {
    /// Constructs and validates the complete wallet entry-point artifact set.
    pub fn new(
        planner_contract_ref: ContentRef,
        planner_implementation_ref: ContentRef,
        object_evidence_contract_ref: ContentRef,
        unit_config_contract: RetainedValueContract,
    ) -> Result<Self> {
        let expected_unit = unit_config_value_contract(
            unit_config_contract.role().clone(),
            object_evidence_contract_ref.clone(),
        )?;
        if unit_config_contract != expected_unit {
            return Err(ProgramError::Registry(
                "EVM wallet entry point requires the exact retained UnitConfig contract".to_owned(),
            ));
        }
        Ok(Self {
            planner_contract_ref,
            planner_implementation_ref,
            object_evidence_contract_ref,
            unit_config_contract,
        })
    }
}

/// Returns the fixed wallet unit-configuration support member path.
pub fn evm_submit_transaction_unit_config_member_path() -> Result<FieldPath> {
    FieldPath::new(EVM_SUBMIT_TRANSACTION_UNIT_CONFIG_MEMBER_PATH)
        .map_err(|error| ProgramError::Codec(error.to_string()))
}

/// Builds the published wallet entry registration and deterministic author.
pub fn evm_submit_transaction_entry_point_registration(
    artifacts: EvmSubmitTransactionEntryPointArtifacts,
) -> Result<QualifiedEntryPointRegistration> {
    let contracts = evm_submit_transaction_value_contracts(artifacts.object_evidence_contract_ref)?;
    let entry_point = evm_submit_transaction_entry_point_contract(
        artifacts.planner_contract_ref,
        artifacts.planner_implementation_ref,
    )?;
    let input_contract = QualifiedEntryPointInputContract::new(
        QualifiedSourceContract::new(contracts.selector().clone(), Vec::new())?,
        QualifiedSourceContract::new(contracts.request().clone(), Vec::new())?,
        None,
        None,
        None,
        Vec::new(),
    )?;
    QualifiedEntryPointRegistration::new(
        entry_point,
        input_contract,
        contracts.outcome().clone(),
        vec![QualifiedAuthoringRootRequirement::new(
            evm_submit_transaction_unit_config_member_path()?,
            QualifiedSourceContract::new(artifacts.unit_config_contract, Vec::new())?,
        )],
        author_evm_submit_transaction,
    )
}

fn author_evm_submit_transaction(
    inputs: &QualifiedAuthoringInputs<'_>,
) -> Result<mfm_spec::CanonicalAuthoredProgram> {
    decode_boundary::<crate::EvmSubmitTransactionRequest>(inputs.configured().canonical())?;
    let unit_path = evm_submit_transaction_unit_config_member_path()?;
    let unit = inputs.root(&unit_path).ok_or_else(|| {
        ProgramError::Registry("EVM wallet authoring omitted framework UnitConfig".to_owned())
    })?;
    let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(unit.bytes())
        .map_err(|error| ProgramError::Codec(error.to_string()))?;
    decode_boundary::<UnitConfig>(&canonical)?;
    evm_submit_transaction_authored_program(unit.value_ref().clone())
}
