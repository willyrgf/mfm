use mfm_certify::PersistedSpecCertificateParts;
use mfm_runtime::CertifiedRuntimeSpec;

fn main() {
    let persisted_parts =
        PersistedSpecCertificateParts::from_untrusted_bytes(Vec::new(), Vec::new());
    let parsed = persisted_parts.parse_untrusted().unwrap();
    let _ = CertifiedRuntimeSpec::new(parsed);
}
