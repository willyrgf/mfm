use mfm_program::{Program, StateDeclaration};

fn main() {
    let _ = Program::new;
    let _state = StateDeclaration {};
    let _ = serde_json::from_str::<StateDeclaration>("{}");
}
