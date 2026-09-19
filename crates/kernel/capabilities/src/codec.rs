//! Synchronous native codec scopes retain the cause and phase, including codec panics.
use crate::CallbackFailure;
use mfm_values::InvocationDiagnostic;

/// Decodes native material inside an already selected pure invocation job.
pub fn decode<T>(
    job: impl FnOnce() -> Result<T, InvocationDiagnostic>,
) -> Result<T, CallbackFailure> {
    invoke(job, "decode", CallbackFailure::Decode)
}

/// Encodes native material inside an already selected pure invocation job.
pub fn encode<T>(
    job: impl FnOnce() -> Result<T, InvocationDiagnostic>,
) -> Result<T, CallbackFailure> {
    invoke(job, "encode", CallbackFailure::Encode)
}

fn invoke<T>(
    job: impl FnOnce() -> Result<T, InvocationDiagnostic>,
    operation: &'static str,
    phase: fn(InvocationDiagnostic) -> CallbackFailure,
) -> Result<T, CallbackFailure> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(job))
        .map_err(|_| {
            phase(InvocationDiagnostic::from_fields(
                "task_failure",
                operation,
                "panicked",
                None,
            ))
        })?
        .map_err(phase)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn same(actual: InvocationDiagnostic, expected: &InvocationDiagnostic) {
        assert_eq!(actual.code(), expected.code());
        assert_eq!(actual.operation(), expected.operation());
        assert_eq!(actual.details(), expected.details());
        assert_eq!(actual.size(), expected.size());
    }

    #[test]
    fn codec_scopes_retain_supplied_diagnostics_and_report_panics_without_payloads() {
        let diagnostic =
            || InvocationDiagnostic::from_fields("value_error", "native_anchor", &42, None);
        let expected = diagnostic();
        let Err(CallbackFailure::Decode(actual)) = decode::<()>(|| Err(diagnostic())) else {
            panic!("phase")
        };
        same(actual, &expected);
        let Err(CallbackFailure::Encode(actual)) = encode::<()>(|| Err(diagnostic())) else {
            panic!("phase")
        };
        same(actual, &expected);
        let Err(CallbackFailure::Decode(actual)) = decode::<()>(|| panic!("omitted payload"))
        else {
            panic!("phase")
        };
        same(
            actual,
            &InvocationDiagnostic::from_fields("task_failure", "decode", "panicked", None),
        );
        let Err(CallbackFailure::Encode(actual)) = encode::<()>(|| panic!("omitted payload"))
        else {
            panic!("phase")
        };
        same(
            actual,
            &InvocationDiagnostic::from_fields("task_failure", "encode", "panicked", None),
        );
    }
    #[test]
    fn construction_and_state_boundaries_retain_nested_codec_phase_cause_and_size() {
        let size = mfm_values::SizeViolation::Measured {
            resource: mfm_values::SizeResource::CanonicalObject,
            actual: 9,
            limit: 8,
        };
        let cause =
            || InvocationDiagnostic::from_fields("value_error", "native_object", &42, Some(size));
        for (phase, failure) in [
            ("decode", CallbackFailure::Decode(cause())),
            ("encode", CallbackFailure::Encode(cause())),
        ] {
            let retained = failure.into_diagnostic();
            assert_eq!(retained.code(), "native_codec");
            assert_eq!(retained.operation(), phase);
            assert_eq!(retained.size(), Some(size));
            assert_eq!(retained.details().as_value()["code"], "value_error");
            assert_eq!(retained.details().as_value()["operation"], "native_object");
            assert_eq!(retained.details().as_value()["details"], 42);
        }
        same(
            CallbackFailure::Execute(cause()).into_diagnostic(),
            &cause(),
        );
    }
}
