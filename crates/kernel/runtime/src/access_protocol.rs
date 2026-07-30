//! Private affine stages for the one runtime-owned ambient-access bracket.

use std::marker::PhantomData;

mod sealed {
    pub trait Sealed {}
}

pub(crate) enum Read {}
pub(crate) enum Ensure {}
pub(crate) enum FactSelection {}
#[cfg(test)]
pub(crate) enum ObservationCommitProbe {}

impl sealed::Sealed for Read {}
impl sealed::Sealed for Ensure {}
impl sealed::Sealed for FactSelection {}
#[cfg(test)]
impl sealed::Sealed for ObservationCommitProbe {}

pub(crate) trait AccessKind: sealed::Sealed {
    type PreparedInvocation;
    type BoundaryReturn;
    type PendingMaterial;
}

#[cfg(test)]
impl AccessKind for ObservationCommitProbe {
    type PreparedInvocation = ();
    type BoundaryReturn = ();
    type PendingMaterial = u64;
}

pub(crate) struct Prepared<K: AccessKind> {
    invocation: K::PreparedInvocation,
    _kind: PhantomData<fn(K) -> K>,
}

impl<K: AccessKind> Prepared<K> {
    pub(crate) const fn new(invocation: K::PreparedInvocation) -> Self {
        Self {
            invocation,
            _kind: PhantomData,
        }
    }

    pub(crate) fn into_invocation(self) -> K::PreparedInvocation {
        self.invocation
    }
}

pub(crate) struct Authorized<K: AccessKind> {
    invocation: K::BoundaryReturn,
    _kind: PhantomData<fn(K) -> K>,
}

impl<K: AccessKind> Authorized<K> {
    pub(crate) const fn new(invocation: K::BoundaryReturn) -> Self {
        Self {
            invocation,
            _kind: PhantomData,
        }
    }

    pub(crate) fn into_invocation(self) -> K::BoundaryReturn {
        self.invocation
    }
}

pub(crate) struct PendingObservation<
    K: AccessKind,
    LogicalIdentity = mfm_store::LogicalObservationIdentity,
    PhysicalIdentity = mfm_store::PhysicalAppendIdentity,
> {
    material: K::PendingMaterial,
    logical_identity: Option<LogicalIdentity>,
    physical_identity: Option<PhysicalIdentity>,
    _kind: PhantomData<fn(K) -> K>,
}

impl<K: AccessKind, LogicalIdentity, PhysicalIdentity>
    PendingObservation<K, LogicalIdentity, PhysicalIdentity>
{
    pub(crate) const fn new(material: K::PendingMaterial) -> Self {
        Self {
            material,
            logical_identity: None,
            physical_identity: None,
            _kind: PhantomData,
        }
    }

    pub(crate) const fn material(&self) -> &K::PendingMaterial {
        &self.material
    }

    pub(crate) const fn logical_identity(&self) -> Option<&LogicalIdentity> {
        self.logical_identity.as_ref()
    }

    pub(crate) const fn physical_identity(&self) -> Option<&PhysicalIdentity> {
        self.physical_identity.as_ref()
    }

    pub(crate) fn remember_prepared(
        &mut self,
        logical: LogicalIdentity,
        physical: PhysicalIdentity,
    ) -> Result<(), crate::RuntimeError>
    where
        LogicalIdentity: PartialEq,
        PhysicalIdentity: PartialEq,
    {
        if self
            .logical_identity
            .as_ref()
            .is_some_and(|retained| retained != &logical)
        {
            return Err(crate::RuntimeError::ObservationConflict);
        }
        if let Some(retained) = &self.physical_identity {
            if retained != &physical {
                return Err(crate::RuntimeError::ObservationConflict);
            }
        }
        self.logical_identity.get_or_insert(logical);
        self.physical_identity = Some(physical);
        Ok(())
    }

    pub(crate) fn discard_stale_physical(&mut self) {
        self.physical_identity = None;
    }
}

pub(crate) struct CommittedObservation<
    K: AccessKind,
    T,
    ObservationRef = mfm_journal::ObservationRef,
> {
    observation_ref: ObservationRef,
    outcome: T,
    _kind: PhantomData<fn(K) -> K>,
}

impl<K: AccessKind, T, ObservationRef> CommittedObservation<K, T, ObservationRef> {
    pub(crate) fn new(observation_ref: ObservationRef, outcome: T) -> Self {
        Self {
            observation_ref,
            outcome,
            _kind: PhantomData,
        }
    }

    pub(crate) const fn observation_ref(&self) -> &ObservationRef {
        &self.observation_ref
    }

    pub(crate) const fn outcome(&self) -> &T {
        &self.outcome
    }
}

pub(crate) enum PreparedAccess {
    Read(Prepared<Read>),
    Ensure(Prepared<Ensure>),
    FactSelection(Prepared<FactSelection>),
}
