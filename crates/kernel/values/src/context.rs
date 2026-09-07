use mfm_ids::StableId;

use crate::MfmValue;

/// Mechanical access to one named field of an immutable typed context snapshot.
///
/// Replacement moves all siblings unchanged and accepts any [`MfmValue`]. Domain
/// stage types and their checked constructors own preservation of earlier facts;
/// this primitive does not itself prove accumulation or execution provenance.
pub trait ContextSlot<C: MfmValue> {
    /// The selected field's exact current value type.
    type Value: MfmValue;
    /// The complete context type after replacing this field.
    type With<V: MfmValue>: MfmValue;

    /// Borrows the selected field.
    fn get(context: &C) -> &Self::Value;
    /// Replaces the selected field while moving every sibling unchanged.
    fn replace<V: MfmValue>(context: C, value: V) -> Self::With<V>;
    /// Returns the explicit context namespace and stable field identity.
    fn slot_id() -> crate::Result<StableId>;
}
