#![warn(missing_docs)]
//! Shared deterministic chain-domain semantics, independent of native networks and Runtime.
//!
//! See the [capability authoring guide](https://github.com/willyrgf/mfm/blob/main/docs/capability-authoring.md)
//! for consumer, State and native implementation responsibilities.

mod scalar;
pub use scalar::{AdditionInputError, AdditionOverflow, CheckedAdd, CheckedAddition};

mod identity;
pub use identity::{
    BalanceTarget, ContractArtifact, ContractLocator, LedgerIdentity, ObservationPoint,
    TransactionIdentity,
};

pub mod balance;
pub mod transaction;
