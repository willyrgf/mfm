use mfm_store::structured::{ExportRunEvidence, PublicRunEvidence};

fn require_export(_evidence: &ExportRunEvidence) {}

fn main() {
    fn check(evidence: &PublicRunEvidence) {
        // Public-read evidence must not satisfy export-only APIs.
        require_export(evidence);
    }
    let _ = check;
}
