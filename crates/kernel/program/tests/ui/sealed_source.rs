use mfm_program::{AuthoringSource, NoParams};

struct Unchecked;
impl AuthoringSource for Unchecked {
    type Input = NoParams;
    type Output = NoParams;
}
fn main() {}
