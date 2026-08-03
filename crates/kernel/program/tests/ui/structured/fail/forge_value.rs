use std::marker::PhantomData;

use mfm_program::structured::Value;
use mfm_spec::structured::LexicalSlot;

fn forge<T>(slot: LexicalSlot) -> Value<T> {
    Value {
        slot,
        _value: PhantomData,
    }
}

fn main() {}
