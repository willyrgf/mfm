use mfm_capabilities::{CapabilityError, ReadCapabilityContract};
use mfm_ids::StableId;
use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Intent {
    correlation: u64,
}

#[derive(Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Evidence {
    correlation: u64,
}

struct ExactRead;

impl ReadCapabilityContract for ExactRead {
    type Intent = Intent;
    type Evidence = Evidence;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test/read-contract@1").map_err(|_| CapabilityError::InvalidContract)
    }

    fn bind_evidence(
        intent: &Self::Intent,
        evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        (intent.correlation == evidence.correlation)
            .then_some(())
            .ok_or(CapabilityError::EvidenceBinding)
    }
}

struct InvalidRead;

impl ReadCapabilityContract for InvalidRead {
    type Intent = Intent;
    type Evidence = Evidence;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Err(CapabilityError::InvalidContract)
    }

    fn bind_evidence(
        _intent: &Self::Intent,
        _evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        Ok(())
    }
}

#[test]
fn evidence_binding_is_closed_and_exact() {
    let intent = Intent { correlation: 7 };
    assert_eq!(
        ExactRead::bind_evidence(&intent, &Evidence { correlation: 7 }),
        Ok(())
    );
    assert_eq!(
        ExactRead::bind_evidence(&intent, &Evidence { correlation: 8 }),
        Err(CapabilityError::EvidenceBinding)
    );
    assert_eq!(
        InvalidRead::contract_id(),
        Err(CapabilityError::InvalidContract)
    );
}
