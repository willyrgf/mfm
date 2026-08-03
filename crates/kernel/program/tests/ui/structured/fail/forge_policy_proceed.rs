#[path = "../support.rs"]
mod support;

use std::marker::PhantomData;

use mfm_program::structured::{Never, PolicyProceed};
use support::{Input, Output};

fn forge() -> PolicyProceed<Input, Output, Never> {
    PolicyProceed {
        _input: PhantomData,
        _output: PhantomData,
        _failure: PhantomData,
    }
}

fn main() {}
