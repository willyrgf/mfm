use mfm_runtime::RunSession;

fn requires_clone<T: Clone>() {}

fn main() {
    requires_clone::<RunSession>();
}
