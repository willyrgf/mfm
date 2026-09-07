use mfm_program_derive::MfmContext;

#[derive(MfmContext)]
struct MissingNamespace<T> { value: T }

#[derive(MfmContext)]
#[context(namespace = "mfm.test.tuple")]
struct Tuple<T>(T);

#[derive(MfmContext)]
#[context(namespace = "mfm.test.enum")]
enum Sum<T> { Value(T) }

#[derive(MfmContext)]
#[context(namespace = "mfm.test.nested")]
struct Nested<T> { value: Vec<T> }

#[derive(MfmContext)]
#[context(namespace = "mfm.test.repeated")]
struct Repeated<T> { first: T, second: T }

#[derive(MfmContext)]
#[context(namespace = "mfm.test.hidden")]
struct Hidden<T> { first: T, second: Vec<Option<T>> }

#[derive(MfmContext)]
#[context(namespace = "mfm.test.bound")]
struct Bound<T: Clone> { value: T }

#[derive(MfmContext)]
#[context(namespace = "mfm.test.default")]
struct Defaulted<T = u64> { value: T }

#[derive(MfmContext)]
#[context(namespace = "mfm.test.where")]
struct Constrained<T> where T: Clone { value: T }

#[derive(MfmContext)]
#[context(namespace = "mfm.test.lifetime")]
struct Borrowed<'a, T> { value: T, sibling: &'a str }

#[derive(MfmContext)]
#[context(namespace = "mfm.test.const")]
struct Sized<T, const N: usize> { value: T, sibling: [u8; N] }

fn main() {}
