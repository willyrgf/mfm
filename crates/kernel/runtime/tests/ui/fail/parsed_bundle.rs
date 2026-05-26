use mfm_certify::CertifiedSpecBundle;
use mfm_runtime::CertifiedRuntimeSpec;

fn main() {
    let bundle = CertifiedSpecBundle::from_untrusted_bytes(Vec::new(), Vec::new());
    let parsed = bundle.parse_untrusted().unwrap();
    let _ = CertifiedRuntimeSpec::new(parsed);
}
