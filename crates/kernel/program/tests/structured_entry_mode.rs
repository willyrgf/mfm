//! Worked non-EVM entry-mode fixtures and their authoring boundaries.
//!
//! EVM's entry key is minted by a reservation authority, which is the easy
//! shape. These fixtures pin both halves of the contract over an ordinary
//! relational insert, and none of them is exercised by the EVM path.

use mfm_capabilities::{
    EffectCapabilityContract, EntryAbsorbing, EntryKeyed, EntryOnce, NoRefresh,
};
use mfm_ids::StableId;
use mfm_program::structured::{
    runtime_effect_capability_contract, EntryAbsorbingBinding, EntryOnceBinding, NoRefreshBinding,
    RuntimeEffectCapability,
};
use mfm_program_derive::MfmValue;
use mfm_spec::structured::{
    structured_value_contract_ref, StructuredComponentKind, StructuredEffectEntryContract,
    StructuredLiveComponentContract,
};
use mfm_values::CanonicalJsonPersistedSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.fixture",
    name = "insert_key",
    version = "1",
    schema = "mfm.fixture.insert_key"
)]
struct InsertKey {
    value: String,
}

/// An insert whose external system keys the row on a caller-supplied value.
///
/// The key is a mandatory field rather than an option, which is what makes
/// [`EntryKeyed`] implementable honestly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.fixture",
    name = "insert_request",
    version = "1",
    schema = "mfm.fixture.insert_request"
)]
struct InsertRequest {
    table: String,
    columns: Vec<String>,
    values: Vec<String>,
    idempotency_key: InsertKey,
}

impl EntryKeyed for InsertRequest {
    type EntryKey = InsertKey;

    fn entry_key(&self) -> Self::EntryKey {
        self.idempotency_key.clone()
    }
}

/// The post-state the insert leaves behind, keyed and re-readable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.fixture",
    name = "insert_outcome",
    version = "1",
    schema = "mfm.fixture.insert_outcome"
)]
struct InsertOutcome {
    key: InsertKey,
    row: String,
}

/// A count of rows one exchange happened to touch.
///
/// This is the shape the post-state-functional rule forbids as a `Returned`:
/// `RowsAffected(1)` is a property of one exchange, not of the external
/// system's post-state, so an absorbed repeat would report `0` where the first
/// attempt reported `1` and settlement would disagree with itself. Nothing in
/// the kernel can check this; [`RowsAffectedInsertCapability`] declares
/// [`EntryOnce`] because of it, and this fixture is the closest thing to a check
/// that exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.fixture",
    name = "rows_affected",
    version = "1",
    schema = "mfm.fixture.rows_affected"
)]
struct RowsAffected {
    rows: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.fixture",
    name = "insert_failure",
    version = "1",
    schema = "mfm.fixture.insert_failure"
)]
struct InsertFailure {
    code: String,
}

fn insert_adapter_contract(name: &str) -> mfm_program::Result<StructuredLiveComponentContract> {
    StructuredLiveComponentContract::new(
        StructuredComponentKind::Adapter,
        StableId::new(name).expect("adapter id"),
        Vec::new(),
    )
    .map_err(Into::into)
}

macro_rules! insert_capability {
    ($name:ident, $returned:ty, $entry:ty, $binding:ty, $id:literal, $adapter:literal) => {
        enum $name {}

        impl EffectCapabilityContract for $name {
            type Request = InsertRequest;
            type Returned = $returned;
            type SafeFailure = InsertFailure;
            type Refresh = NoRefresh;
            type Entry = $entry;
        }

        impl RuntimeEffectCapability for $name {
            type RefreshBinding = NoRefreshBinding;
            type EntryBinding = $binding;

            fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
                StructuredLiveComponentContract::new_effect_capability_no_refresh(
                    StableId::new($id).expect("capability id"),
                    structured_value_contract_ref::<InsertRequest>()?,
                    structured_value_contract_ref::<$returned>()?,
                    structured_value_contract_ref::<InsertFailure>()?,
                    <$binding as RuntimeEffectEntryBinding<$entry, InsertRequest>>::contract()?,
                    insert_adapter_contract($adapter)?.content_ref()?,
                )
                .map_err(Into::into)
            }
        }
    };
}

use mfm_program::structured::RuntimeEffectEntryBinding;

insert_capability!(
    InsertRowCapability,
    InsertOutcome,
    EntryAbsorbing<3>,
    EntryAbsorbingBinding<3>,
    "mfm.fixture/insert-row",
    "mfm.fixture/insert-row-adapter"
);
insert_capability!(
    RowsAffectedInsertCapability,
    RowsAffected,
    EntryOnce,
    EntryOnceBinding,
    "mfm.fixture/rows-affected-insert",
    "mfm.fixture/rows-affected-insert-adapter"
);
insert_capability!(
    ZeroBudgetInsertCapability,
    InsertOutcome,
    EntryAbsorbing<0>,
    EntryAbsorbingBinding<0>,
    "mfm.fixture/zero-budget-insert",
    "mfm.fixture/zero-budget-insert-adapter"
);

#[test]
fn an_absorbing_capability_certifies_its_exact_entry_key_and_budget() {
    let contract = runtime_effect_capability_contract::<InsertRowCapability>()
        .expect("absorbing capability contract");
    let entry = contract
        .capability_protocol
        .as_ref()
        .and_then(|protocol| protocol.entry_contract())
        .expect("Effect entry contract");
    let StructuredEffectEntryContract::EntryAbsorbing {
        entry_key_contract_ref,
        max_entries,
    } = entry
    else {
        panic!("declared absorption must certify as absorbing")
    };
    assert_eq!(
        entry_key_contract_ref.as_ref(),
        &structured_value_contract_ref::<InsertKey>().expect("entry key contract"),
        "the certified reference is the key's contract, not the request's"
    );
    assert_eq!(max_entries.get(), 3);
}

#[test]
fn a_capability_with_no_recoverable_returned_value_certifies_entry_once() {
    let contract = runtime_effect_capability_contract::<RowsAffectedInsertCapability>()
        .expect("entry-once capability contract");
    assert_eq!(
        contract
            .capability_protocol
            .as_ref()
            .and_then(|protocol| protocol.entry_contract()),
        Some(&StructuredEffectEntryContract::EntryOnce {})
    );
}

#[test]
fn a_zero_entry_budget_is_an_authoring_rejection() {
    // A const generic cannot express non-zero, so the illegal state is rejected
    // at authoring time and never reaches a certified document.
    let error = runtime_effect_capability_contract::<ZeroBudgetInsertCapability>()
        .expect_err("an absorbing capability must admit at least one entry");
    assert!(
        matches!(error, mfm_program::ProgramError::Authoring(_)),
        "expected an authoring rejection, got {error:?}"
    );
}

#[test]
fn an_entry_key_is_a_deterministic_projection_of_committed_request_bytes() {
    let request = InsertRequest {
        table: "ledger".to_owned(),
        columns: vec!["id".to_owned(), "amount".to_owned()],
        values: vec!["7".to_owned(), "11".to_owned()],
        idempotency_key: InsertKey {
            value: "exactly-once".to_owned(),
        },
    };
    assert_eq!(request.entry_key(), request.entry_key());
    assert_eq!(request.entry_key(), request.clone().entry_key());
    assert_eq!(
        request.entry_key(),
        InsertKey {
            value: "exactly-once".to_owned()
        }
    );
}
