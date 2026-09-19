//! Purpose-specific native envelopes. Exact protocol interpretation remains with the native owner.

use mfm_program_derive::MfmValue;
use mfm_values::Object;
use serde::{Deserialize, Serialize};

/// Structurally checked native ledger identity; no network or protocol validation is implied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.chain",
    name = "ledger-identity",
    version = "1",
    schema = "mfm.chain-ledger-identity"
)]
pub struct LedgerIdentity {
    native: Object,
}
impl LedgerIdentity {
    /// Wraps an already-qualified Object; the selected native owner must check its exact contract.
    pub fn new(native: Object) -> Self {
        Self { native }
    }
    /// Exact native ledger value for owner-qualified decoding.
    pub fn native(&self) -> &Object {
        &self.native
    }
}

macro_rules! envelope {
    ($name:ident, $contract:literal, $schema:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
        #[serde(deny_unknown_fields)]
        #[mfm(namespace = "mfm.chain", name = $contract, version = "1", schema = $schema)]
        pub struct $name {
            ledger: LedgerIdentity,
            native: Object,
        }
        impl $name {
            /// Wraps qualified values without certifying their native type or ledger correspondence.
            /// The selected native implementation must establish those facts before use.
            pub fn new(ledger: LedgerIdentity, native: Object) -> Self {
                Self { ledger, native }
            }
            /// Claimed ledger, checked by the selected native owner.
            pub fn ledger(&self) -> &LedgerIdentity {
                &self.ledger
            }
            /// Exact native value for owner-qualified decoding.
            pub fn native(&self) -> &Object {
                &self.native
            }
        }
    };
}
envelope!(ContractArtifact, "contract-artifact", "mfm.chain-contract-artifact", "A public contract artifact envelope; only its native owner can qualify supported artifact semantics.");
envelope!(
    ContractLocator,
    "contract-locator",
    "mfm.chain-contract-locator",
    "A contract's ledger and exact native location."
);
envelope!(
    TransactionIdentity,
    "transaction-identity",
    "mfm.chain-transaction-identity",
    "A transaction's ledger and exact native identity."
);
envelope!(
    ObservationPoint,
    "observation-point",
    "mfm.chain-observation-point",
    "A ledger observation point retaining its full native number/hash representation."
);
envelope!(
    BalanceTarget,
    "balance-target",
    "mfm.chain-balance-target",
    "A balance account and asset envelope qualified by its selected native owner."
);
