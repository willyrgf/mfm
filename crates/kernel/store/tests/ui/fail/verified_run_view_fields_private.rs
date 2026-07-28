use mfm_certify::CertifiedTypedSpec;
use mfm_store::v1::{CommittedRunJournal, VerifiedRunView};

fn forge(journal: CommittedRunJournal, certified_spec: CertifiedTypedSpec) -> VerifiedRunView {
    VerifiedRunView {
        journal,
        certified_spec,
    }
}

fn main() {}
