//! Conversion between store-sealed frames and producer-free callback material.

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_program::{QualifiedSettlement, VerifiedStateFrameMaterial, VerifiedValueMaterial};
use mfm_spec::CertifiedSettlementContract;
use mfm_store::{
    PreparedFrame, PreparedValue, ProducedObjectRoot, ProducedOutputSlot, SettlementMaterial,
};

use crate::{Result, RuntimeError};

pub(crate) fn verified_callback_frame(frame: &PreparedFrame) -> Result<VerifiedStateFrameMaterial> {
    Ok(VerifiedStateFrameMaterial::new(
        verified_value(frame.config())?,
        frame.context().map(verified_value).transpose()?,
        verified_value(frame.input())?,
    ))
}

fn verified_value(value: &PreparedValue) -> Result<VerifiedValueMaterial> {
    Ok(VerifiedValueMaterial::new(
        PlainCanonicalJsonBytes::from_canonical_json_slice(value.bytes())?,
        value.value_ref().clone(),
    ))
}

pub(crate) fn settlement_material(
    settlement: QualifiedSettlement,
    contract: &CertifiedSettlementContract,
) -> Result<SettlementMaterial> {
    Ok(match settlement {
        QualifiedSettlement::Succeeded {
            output_slots,
            facts,
        } => SettlementMaterial::Succeeded {
            output_roots: output_slots
                .into_iter()
                .map(|slot| {
                    let root = ProducedObjectRoot::new(
                        slot.value_contract().clone(),
                        slot.value().canonical().clone(),
                    );
                    ProducedOutputSlot::new(slot.output_ordinal(), slot.field_path().clone(), root)
                })
                .collect(),
            fact_roots: facts.into_vec(),
        },
        QualifiedSettlement::Failed { failure } => {
            let failure_contract = contract
                .typed_failure_contract()
                .ok_or(RuntimeError::InvalidCallbackResult)?;
            SettlementMaterial::Failed {
                typed_failure_root: Box::new(ProducedObjectRoot::new(
                    failure_contract.clone(),
                    failure.canonical().clone(),
                )),
            }
        }
    })
}
