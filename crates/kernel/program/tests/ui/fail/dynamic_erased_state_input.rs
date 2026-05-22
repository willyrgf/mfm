use std::any::Any;

fn main() {
    let erased: Box<dyn Any + Send + Sync> = Box::new(1_u64);
    let _ = <Box<dyn Any + Send + Sync> as mfm_program::IntoStateInput<
        'static,
        'static,
        Box<dyn Any + Send + Sync>,
    >>::into_binding(erased);
}
