//! Frozen recoverability-v3 journal model.

mod access;
mod codec;
mod commit;
mod fact;
mod input;
mod object;
mod record;
mod refs;
mod transition;

pub use self::access::*;
pub use self::codec::{JournalError, PersistedJournalValue, Result};
pub use self::commit::*;
pub use self::fact::*;
pub use self::input::*;
pub use self::object::*;
pub use self::record::*;
pub use self::refs::*;
pub use self::transition::*;
pub use mfm_values::RetainedValueContract;

fn runtime_retained_contract(
    schema_contract: &str,
    semantic_name: &str,
    role: &str,
) -> Result<RetainedValueContract> {
    let semantic_type_id = mfm_ids::SemanticTypeId::new(
        "mfm.recoverability",
        semantic_name,
        "1",
        mfm_ids::DigestAlgorithm::Sha256JcsV1,
        mfm_canonical::sha256_digest_bytes(
            format!("semantic:mfm.recoverability:{semantic_name}:1").as_bytes(),
        ),
    )?;
    RetainedValueContract::new(
        codec::contract()?.schema_id(schema_contract)?.clone(),
        semantic_type_id,
        mfm_ids::StableId::new(role)?,
        "application/json",
        mfm_values::component_object_evidence_contract_ref()?,
    )
    .map_err(Into::into)
}
