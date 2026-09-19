//! Per-source planning facts without native interpretation or live configuration.
use super::{BalanceRead, BalanceSource, DecimalScale, ObserveBalance};
use mfm_ids::ContentRef;
use mfm_program::{OperationDefinition, Plan, Read};
use mfm_program_derive::MfmValue;
use mfm_values::{MfmValue as Value, Object};
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;

/// Caller-owned route expectation and exact public native execution descriptor.
///
/// Structural admission does not certify native agreement. The native owner must derive the route
/// from the descriptor and qualify it against this expectation before binding resources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.chain",
    name = "balance-execution-config",
    version = "1",
    schema = "mfm.chain-balance-execution-config"
)]
pub struct BalanceExecutionConfig {
    route_ref: ContentRef,
    native: Object,
}
impl BalanceExecutionConfig {
    /// Retains checked public identities; native qualification belongs to the native owner.
    pub fn new(route_ref: ContentRef, native: Object) -> Self {
        Self { route_ref, native }
    }
    /// Caller-owned expected route, independent of the selected binding.
    pub fn route_ref(&self) -> &ContentRef {
        &self.route_ref
    }
    /// Exact native public execution descriptor, without a live handle.
    pub fn native(&self) -> &Object {
        &self.native
    }
}

/// Known source, position, scale and execution facts for one shared balance Read.
pub struct BalanceSourceDefinition<K> {
    source: BalanceSource,
    ordinal: u32,
    scale: DecimalScale,
    execution: BalanceExecutionConfig,
    caller: PhantomData<fn() -> K>,
}
impl<K> BalanceSourceDefinition<K> {
    /// Retains the collection's checked declaration-order planning facts.
    pub fn new(
        source: BalanceSource,
        ordinal: u32,
        scale: DecimalScale,
        execution: BalanceExecutionConfig,
    ) -> Self {
        Self {
            source,
            ordinal,
            scale,
            execution,
            caller: PhantomData,
        }
    }
    /// Exact semantic source to qualify through native resolution.
    pub fn source(&self) -> &BalanceSource {
        &self.source
    }
    /// Declared source position, independent of a future completed prefix.
    pub fn ordinal(&self) -> u32 {
        self.ordinal
    }
    /// Collection scaling policy.
    pub fn scale(&self) -> DecimalScale {
        self.scale
    }
    /// Retained native execution descriptor and independent route expectation.
    pub fn execution(&self) -> &BalanceExecutionConfig {
        &self.execution
    }
}
impl<K: Value> OperationDefinition for BalanceSourceDefinition<K> {
    type Body = Read<ObserveBalance<K>, BalanceRead>;
}
impl<K: Value, Parent: ?Sized> Plan<Parent> for BalanceSourceDefinition<K> {
    type Config = Self;
    fn plan<'a>(&'a self, _: &'a Parent) -> mfm_program::Result<(&'a Self, Self::Body)> {
        Ok((self, Read::default()))
    }
}
