#![warn(missing_docs)]
//! Capability-owned intent, evidence, and entry-discipline contracts.
//!
//! This crate contains no provider client and no execution callback.  A capability describes the
//! exact canonical intent and closed evidence value that an adapter may exchange at the Runtime
//! boundary.  The three sealed entry modes make retry authority a durable contract rather than a
//! generic error policy.

pub mod single_trust;

pub use single_trust::{
    AccessCapabilityContract, AccessEvidenceValue, AccessMode, CapabilityError, EffectEntryMode,
    EffectMode, EntryAbsorbing, EntryOnce, FactSelectionMode, NoPriorFacts, PriorRunFacts,
    ProposedStateOutcome, QualifiedRecordedEvidence, ReadMode, Result,
};
