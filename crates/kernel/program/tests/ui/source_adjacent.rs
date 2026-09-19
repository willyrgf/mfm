#[allow(dead_code)]
#[path = "../phase_a/contracts.rs"]
mod contracts;
use mfm_program as source;
use contracts::*;
use source::*;
fn accepts<S: AuthoringSource>(_: &S) {}
fn requires_prepared<S: AuthoringSource<Input = Prepared>>(_: &S) {}
fn main() {
    accepts(&(Pure::<Add>::default(), Pure::<Validate>::default()));
}
