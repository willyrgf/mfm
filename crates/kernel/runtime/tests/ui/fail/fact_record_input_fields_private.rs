fn take<T>(input: mfm_runtime::FactRecordInput<T>)
where
    T: mfm_program::MfmFactType,
{
    let _ = input.fact;
    let _ = input.visibility;
    let _ = input.observed_at;
}

fn main() {}
