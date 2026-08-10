#![allow(dead_code)]

use mfm_capabilities::{
    EffectCapabilityContract, EntryKeyed, EntryOnce, NoRefresh, ReadCapabilityContract,
};
use mfm_ids::StableId;
use mfm_program::structured::{
    Direct, Effect, EntryOnceBinding, Never, NoRefreshBinding, Pure, RuntimeEffectCapability,
    RuntimeReadCapability, SafeFailureNotApplicable, SafeFailureSuccessOnly, State,
};
use mfm_program_derive::MfmValue;
use mfm_spec::structured::{
    structured_value_contract_ref, StructuredComponentKind, StructuredLiveComponentContract,
};
use mfm_values::CanonicalJsonPersistedSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.ui",
    name = "structured_policy_input",
    version = "1",
    schema = "mfm.ui.structured_policy_input"
)]
pub struct Input {
    pub value: u64,
}

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.ui",
    name = "structured_policy_output",
    version = "1",
    schema = "mfm.ui.structured_policy_output"
)]
pub struct Output {
    pub value: u64,
}

pub struct PureState;

impl State for PureState {
    type Input = Input;
    type Output = Output;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(stable("mfm.ui/pure"))
    }
}

pub struct EffectState;

pub struct EffectCapability;
pub struct ReadCapability;

impl ReadCapabilityContract for ReadCapability {
    type Request = Input;
    type Returned = Output;
    type SafeFailure = Output;
}

impl RuntimeReadCapability for ReadCapability {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        let adapter_contract_ref = StructuredLiveComponentContract::new(
            StructuredComponentKind::Adapter,
            stable("mfm.ui/read-adapter"),
            Vec::new(),
        )?
        .content_ref()?;
        StructuredLiveComponentContract::new_read_capability(
            stable("mfm.ui/read-capability"),
            structured_value_contract_ref::<Input>()?,
            structured_value_contract_ref::<Output>()?,
            structured_value_contract_ref::<Output>()?,
            adapter_contract_ref,
        )
        .map_err(Into::into)
    }
}

impl EffectCapabilityContract for EffectCapability {
    type Request = Input;
    type Returned = Output;
    type SafeFailure = Output;
    type Refresh = NoRefresh;
    type Entry = EntryOnce;
}

impl RuntimeEffectCapability for EffectCapability {
    type RefreshBinding = NoRefreshBinding;
    type EntryBinding = EntryOnceBinding;

    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        let adapter_contract_ref = StructuredLiveComponentContract::new(
            StructuredComponentKind::Adapter,
            stable("mfm.ui/effect-adapter"),
            Vec::new(),
        )?
        .content_ref()?;
        StructuredLiveComponentContract::new_effect_capability_no_refresh(
            stable("mfm.ui/effect-capability"),
            structured_value_contract_ref::<Input>()?,
            structured_value_contract_ref::<Output>()?,
            structured_value_contract_ref::<Output>()?,
            mfm_spec::structured::StructuredEffectEntryContract::EntryOnce {},
            adapter_contract_ref,
        )
        .map_err(Into::into)
    }
}

impl State for EffectState {
    type Input = Input;
    type Output = Output;
    type Failure = Never;
    type Request = Input;
    type Returned = Output;
    type SafeFailure = Output;
    type Execution = Effect<EffectCapability>;
    type SafeFailureDisposition = SafeFailureSuccessOnly;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(stable("mfm.ui/effect"))
    }
}

pub fn stable(value: &str) -> StableId {
    StableId::new(value).expect("stable UI fixture id")
}

/// An insert whose external system keys the row on a caller-supplied value.
#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.ui",
    name = "structured_keyed_insert_request",
    version = "1",
    schema = "mfm.ui.structured_keyed_insert_request"
)]
pub struct KeyedInsertRequest {
    pub table: String,
    pub idempotency_key: InsertKey,
}

/// The same insert against a table with an autoincrement primary key and no
/// natural one. Nothing distinguishes this effect from an identical one someone
/// else made, so it can name no entry key at all.
#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.ui",
    name = "structured_autoincrement_insert_request",
    version = "1",
    schema = "mfm.ui.structured_autoincrement_insert_request"
)]
pub struct AutoincrementInsertRequest {
    pub table: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.ui",
    name = "structured_insert_key",
    version = "1",
    schema = "mfm.ui.structured_insert_key"
)]
pub struct InsertKey {
    pub value: u64,
}

impl EntryKeyed for KeyedInsertRequest {
    type EntryKey = InsertKey;

    fn entry_key(&self) -> Self::EntryKey {
        self.idempotency_key.clone()
    }
}
