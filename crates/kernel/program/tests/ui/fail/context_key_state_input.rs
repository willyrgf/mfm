struct ContextKey(String);

fn main() {
    let key = ContextKey("invalid.context.key".to_owned());
    let _ = <ContextKey as mfm_program::IntoStateInput<'static, 'static, ContextKey>>::into_binding(
        key,
    );
}
