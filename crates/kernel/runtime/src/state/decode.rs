use super::*;
use mfm_values::{NativeCause, ObjectSeed};
use serde::de::{
    DeserializeSeed, EnumAccess, Error as _, IgnoredAny, MapAccess, SeqAccess, VariantAccess,
    Visitor,
};
use serde::Deserializer;
use std::{fmt, result::Result};

#[cfg(test)]
mod tests;

// These visitors construct the current payload types directly. Checked Object admission uses
// the inner result; only wire parsing uses the Deserializer's error type.
macro_rules! field_value {
    ($map:ident, plain($ty:ty)) => {
        $map.next_value::<$ty>().map(Ok)
    };
    ($map:ident, seed($seed:expr)) => {
        $map.next_value_seed($seed)
    };
}
macro_rules! native_struct {
    ($seed:ident, $visitor:ident, $ty:ident $(::$variant:ident)? {
        $($field:ident: $kind:ident($($arg:tt)*)),+ $(,)?
    }) => {
        pub(crate) struct $seed;
        struct $visitor;
        impl<'de> DeserializeSeed<'de> for $seed {
            type Value = Result<$ty, NativeCause>;
            fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
                deserializer.deserialize_struct(stringify!($ty), &[$(stringify!($field)),+], $visitor)
            }
        }
        impl<'de> Visitor<'de> for $visitor {
            type Value = Result<$ty, NativeCause>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(stringify!($ty)) }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                $(let mut $field = (false, None);)+
                let mut native = None;
                let mut wire = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        $(stringify!($field) => {
                            if $field.0 {
                                wire.get_or_insert_with(|| A::Error::duplicate_field(stringify!($field)));
                            }
                            $field.0 = true;
                            if native.is_some() || wire.is_some() {
                                map.next_value::<IgnoredAny>()?;
                            } else {
                                match field_value!(map, $kind($($arg)*))? {
                                    Ok(value) => $field.1 = Some(value),
                                    Err(cause) => native = Some(cause),
                                }
                            }
                        })+
                        _ => {
                            wire.get_or_insert_with(|| A::Error::unknown_field(&key, &[$(stringify!($field)),+]));
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                $(if !$field.0 { wire.get_or_insert_with(|| A::Error::missing_field(stringify!($field))); })+
                // A checked failure remains primary after the enclosing map has been consumed.
                // Later structural rejections do not replace it; a drain parser error stays outer.
                if let Some(cause) = native { return Ok(Err(cause)); }
                if let Some(error) = wire { return Err(error); }
                Ok(Ok($ty $(::$variant)? {
                    $($field: $field.1.ok_or_else(|| A::Error::missing_field(stringify!($field)))?),+
                }))
            }
        }
    };
}
macro_rules! variant_value {
    ($value:ident, $ty:ident, $variant:ident, wrap($seed:ident)) => {
        $value
            .newtype_variant_seed($seed)
            .map(|result| result.map($ty::$variant))
    };
    ($value:ident, $ty:ident, $variant:ident, direct($seed:ident)) => {
        $value.newtype_variant_seed($seed)
    };
}
macro_rules! native_enum {
    ($seed:ident, $visitor:ident, $ty:ident {
        $($variant:ident: $kind:ident($child:ident) => $wire:literal),+ $(,)?
    }) => {
        pub(crate) struct $seed;
        struct $visitor;
        impl<'de> DeserializeSeed<'de> for $seed {
            type Value = Result<$ty, NativeCause>;
            fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
                deserializer.deserialize_enum(stringify!($ty), &[$($wire),+], $visitor)
            }
        }
        impl<'de> Visitor<'de> for $visitor {
            type Value = Result<$ty, NativeCause>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(stringify!($ty)) }
            fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<Self::Value, A::Error> {
                let (name, value) = data.variant::<String>()?;
                match name.as_str() {
                    $($wire => variant_value!(value, $ty, $variant, $kind($child)),)+
                    _ => Err(A::Error::unknown_variant(&name, &[$($wire),+])),
                }
            }
        }
    };
}

native_struct!(
    CallSeed,
    CallVisitor,
    Call {
        position: plain(ExecutionPosition),
        input: seed(ObjectSeed),
    }
);
native_struct!(
    EffectCallSeed,
    EffectCallVisitor,
    EffectCall {
        call: seed(CallSeed),
        effect_id: plain(EffectId),
        command: seed(ObjectSeed),
    }
);
native_struct!(
    SettlementSeed,
    SettlementVisitor,
    Settlement {
        effect: seed(EffectCallSeed),
        evidence: seed(ObjectSeed),
    }
);
native_struct!(
    ReadCallSeed,
    ReadCallVisitor,
    StateCall::Read {
        call: seed(CallSeed),
        intent: seed(ObjectSeed),
        evidence: seed(ObjectSeed),
    }
);
native_enum!(StateCallSeed, StateCallVisitor, StateCall {
    Pure: wrap(CallSeed) => "pure",
    Read: direct(ReadCallSeed) => "read",
    Effect: wrap(SettlementSeed) => "effect",
});
native_struct!(
    DomainFailureSeed,
    DomainFailureVisitor,
    DomainFailure {
        call: seed(StateCallSeed),
        original: seed(ObjectSeed),
    }
);
native_struct!(
    ReadFailureSeed,
    ReadFailureVisitor,
    ReadFailure {
        call: seed(CallSeed),
        intent: seed(ObjectSeed),
        original: seed(ObjectSeed),
    }
);
native_struct!(
    PendingFailureSeed,
    PendingFailureVisitor,
    Failure::PendingEffect {
        effect: seed(EffectCallSeed),
        original: seed(ObjectSeed),
    }
);
native_enum!(FailureSeed, FailureVisitor, Failure {
    Domain: wrap(DomainFailureSeed) => "domain",
    Read: wrap(ReadFailureSeed) => "read",
    PendingEffect: direct(PendingFailureSeed) => "pending_effect",
});
native_struct!(
    TerminalDomainSeed,
    TerminalDomainVisitor,
    TerminalFailure::Domain {
        failure: seed(DomainFailureSeed),
        reason: plain(StopReason),
        root: seed(ObjectSeed),
    }
);
native_struct!(
    TerminalReadSeed,
    TerminalReadVisitor,
    TerminalFailure::Read {
        failure: seed(ReadFailureSeed),
        reason: plain(StopReason),
    }
);
native_enum!(TerminalSeed, TerminalVisitor, TerminalFailure {
    Domain: direct(TerminalDomainSeed) => "domain",
    Read: direct(TerminalReadSeed) => "read",
});
native_enum!(PhaseSeed, PhaseVisitor, Phase {
    Runnable: wrap(CallSeed) => "runnable",
    EffectPending: wrap(EffectCallSeed) => "effect_pending",
    AwaitingInterpretation: wrap(SettlementSeed) => "awaiting_interpretation",
    AwaitingRecovery: wrap(FailureSeed) => "awaiting_recovery",
    Succeeded: wrap(ObjectSeed) => "succeeded",
    Failed: wrap(TerminalSeed) => "failed",
});
native_struct!(
    AdmittedSeed,
    AdmittedVisitor,
    OperationFacts::Admitted {
        program: seed(ObjectSeed),
        initial: seed(ObjectSeed),
    }
);
native_struct!(
    SucceededSeed,
    SucceededVisitor,
    OperationFacts::Succeeded {
        call: seed(StateCallSeed),
        output: seed(ObjectSeed),
    }
);
native_struct!(
    RecoveredSeed,
    RecoveredVisitor,
    OperationFacts::Recovered {
        failure: seed(FailureSeed),
        classification: plain(Classification),
        request: plain(RecoveryRequest),
        decision: plain(RecoveryDecision),
    }
);
native_enum!(FactsSeed, FactsVisitor, OperationFacts {
    Admitted: direct(AdmittedSeed) => "admitted",
    Succeeded: direct(SucceededSeed) => "succeeded",
    Failed: wrap(FailureSeed) => "failed",
    EffectPrepared: wrap(EffectCallSeed) => "effect_prepared",
    EffectSettled: wrap(SettlementSeed) => "effect_settled",
    Recovered: direct(RecoveredSeed) => "recovered",
});
native_struct!(
    CheckpointSeed,
    CheckpointVisitor,
    Checkpoint {
        position: plain(StatePosition),
        input: seed(ObjectSeed),
    }
);
struct CheckpointsSeed;
impl<'de> DeserializeSeed<'de> for CheckpointsSeed {
    type Value = Result<Vec<Checkpoint>, NativeCause>;
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        struct CheckpointsVisitor;
        impl<'de> Visitor<'de> for CheckpointsVisitor {
            type Value = Result<Vec<Checkpoint>, NativeCause>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("active checkpoints")
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let mut checkpoints = Vec::new();
                while let Some(checkpoint) = sequence.next_element_seed(CheckpointSeed)? {
                    match checkpoint {
                        Ok(checkpoint) => checkpoints.push(checkpoint),
                        Err(cause) => {
                            while sequence.next_element::<IgnoredAny>()?.is_some() {}
                            return Ok(Err(cause));
                        }
                    }
                }
                Ok(Ok(checkpoints))
            }
        }
        deserializer.deserialize_seq(CheckpointsVisitor)
    }
}
native_struct!(RunStateSeed, RunStateVisitor, RunState {
    phase: seed(PhaseSeed), checkpoints: seed(CheckpointsSeed),
    usage: plain(Vec<StateUsage>), effect_barrier: plain(Option<StatePosition>),
});
native_struct!(
    RunCommitSeed,
    RunCommitVisitor,
    RunCommit {
        program_ref: plain(ContentRef),
        state: seed(RunStateSeed),
        facts: seed(FactsSeed),
    }
);
