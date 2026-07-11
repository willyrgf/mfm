use super::*;

/// Error returned by typed registry operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    /// State descriptor construction failed.
    #[error("state descriptor error: {0}")]
    Descriptor(String),
    /// No registered state matched the requested kind/version.
    #[error("state {kind}@{version} is not registered")]
    UnregisteredState {
        /// Requested state kind.
        kind: String,
        /// Requested state version.
        version: String,
    },
    /// A different descriptor already owns this kind/version pair.
    #[error("duplicate state registration for {kind}@{version}")]
    DuplicateStateRegistration {
        /// Registered state kind.
        kind: String,
        /// Registered state version.
        version: String,
    },
    /// Registry record and descriptor evidence diverged.
    #[error("state registry descriptor mismatch for {kind}@{version}")]
    DescriptorMismatch {
        /// Registered state kind.
        kind: String,
        /// Registered state version.
        version: String,
    },
    /// No registered operation matched the requested kind/version.
    #[error("operation {kind}@{version} is not registered")]
    UnregisteredOperation {
        /// Requested operation kind.
        kind: String,
        /// Requested operation version.
        version: String,
    },
    /// A different descriptor already owns this operation kind/version pair.
    #[error("duplicate operation registration for {kind}@{version}")]
    DuplicateOperationRegistration {
        /// Registered operation kind.
        kind: String,
        /// Registered operation version.
        version: String,
    },
    /// Registry record and operation descriptor evidence diverged.
    #[error("operation registry descriptor mismatch for {kind}@{version}")]
    OperationDescriptorMismatch {
        /// Registered operation kind.
        kind: String,
        /// Registered operation version.
        version: String,
    },
}

impl From<RegistryError> for PlanError {
    fn from(error: RegistryError) -> Self {
        Self::Registry(error.to_string())
    }
}

/// Returns the validated descriptor identity for a typed state.
pub fn state_descriptor<S>() -> std::result::Result<StateDescriptorIdentity, RegistryError>
where
    S: StateSpec,
    S::Effect: EffectRunner<S>,
    S::Caps: CapabilitySetFor<S::Effect>,
{
    StateDescriptorIdentity::for_state::<S>()
        .map_err(|error| RegistryError::Descriptor(error.to_string()))
}

/// Returns the validated descriptor identity for a typed operation.
pub fn operation_descriptor<O>() -> std::result::Result<OperationDescriptorIdentity, RegistryError>
where
    O: Operation,
{
    OperationDescriptorIdentity::for_operation::<O>()
        .map_err(|error| RegistryError::Descriptor(error.to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct StateRegistrationKey {
    kind: String,
    version: String,
}

impl StateRegistrationKey {
    fn from_descriptor(descriptor: &StateDescriptorIdentity) -> Self {
        Self {
            kind: descriptor.kind().as_str().to_owned(),
            version: descriptor.version().as_str().to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StateRegistrationRecord {
    descriptor_id: DescriptorId,
    runner: RunnerKind,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct OperationRegistrationKey {
    kind: String,
    version: String,
}

impl OperationRegistrationKey {
    fn from_descriptor(descriptor: &OperationDescriptorIdentity) -> Self {
        Self {
            kind: descriptor.kind().as_str().to_owned(),
            version: descriptor.version().as_str().to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OperationRegistrationRecord {
    descriptor_id: DescriptorId,
}

/// Immutable state registry snapshot used by typed program builders.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StateRegistrySnapshot {
    records: std::collections::BTreeMap<StateRegistrationKey, StateRegistrationRecord>,
}

impl StateRegistrySnapshot {
    /// Returns true when the registry has no registered states.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Returns the number of registered state kind/version pairs.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Resolves the validated descriptor identity for a state registered in this snapshot.
    pub fn state_descriptor<S>(&self) -> std::result::Result<StateDescriptorIdentity, RegistryError>
    where
        S: StateSpec,
        S::Effect: EffectRunner<S>,
        S::Caps: CapabilitySetFor<S::Effect>,
    {
        let descriptor = state_descriptor::<S>()?;
        let key = StateRegistrationKey::from_descriptor(&descriptor);
        let Some(record) = self.records.get(&key) else {
            return Err(RegistryError::UnregisteredState {
                kind: key.kind,
                version: key.version,
            });
        };
        if record.descriptor_id != *descriptor.descriptor_id()
            || record.runner != descriptor.runner()
        {
            return Err(RegistryError::DescriptorMismatch {
                kind: key.kind,
                version: key.version,
            });
        }
        Ok(descriptor)
    }
}

/// Mutable framework state registry builder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StateRegistryBuilder {
    snapshot: StateRegistrySnapshot,
}

impl StateRegistryBuilder {
    /// Creates an empty registry builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a state after validating descriptor, effect, capability, and runner evidence.
    pub fn register<S>(&mut self) -> std::result::Result<(), RegistryError>
    where
        S: StateSpec,
        S::Effect: EffectRunner<S>,
        S::Caps: CapabilitySetFor<S::Effect>,
    {
        let descriptor = state_descriptor::<S>()?;
        let runner = <S::Effect as EffectRunner<S>>::runner_kind();
        if descriptor.runner() != runner {
            return Err(RegistryError::Descriptor(format!(
                "runner {:?} did not match descriptor runner {:?}",
                runner,
                descriptor.runner()
            )));
        }
        let key = StateRegistrationKey::from_descriptor(&descriptor);
        let record = StateRegistrationRecord {
            descriptor_id: descriptor.descriptor_id().clone(),
            runner,
        };
        if let Some(existing) = self.snapshot.records.get(&key) {
            if existing != &record {
                return Err(RegistryError::DuplicateStateRegistration {
                    kind: key.kind,
                    version: key.version,
                });
            }
        } else {
            self.snapshot.records.insert(key, record);
        }
        Ok(())
    }

    /// Returns an immutable registry snapshot.
    pub fn snapshot(&self) -> StateRegistrySnapshot {
        self.snapshot.clone()
    }

    /// Converts this builder into an immutable registry snapshot.
    pub fn into_snapshot(self) -> StateRegistrySnapshot {
        self.snapshot
    }
}

/// Immutable operation registry snapshot used by typed program builders.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OperationRegistrySnapshot {
    records: std::collections::BTreeMap<OperationRegistrationKey, OperationRegistrationRecord>,
}

impl OperationRegistrySnapshot {
    /// Returns true when the registry has no registered operations.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Returns the number of registered operation kind/version pairs.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Resolves the validated descriptor identity for an operation registered in this snapshot.
    pub fn operation_descriptor<O>(
        &self,
    ) -> std::result::Result<OperationDescriptorIdentity, RegistryError>
    where
        O: Operation,
    {
        let descriptor = operation_descriptor::<O>()?;
        let key = OperationRegistrationKey::from_descriptor(&descriptor);
        let Some(record) = self.records.get(&key) else {
            return Err(RegistryError::UnregisteredOperation {
                kind: key.kind,
                version: key.version,
            });
        };
        if record.descriptor_id != *descriptor.descriptor_id() {
            return Err(RegistryError::OperationDescriptorMismatch {
                kind: key.kind,
                version: key.version,
            });
        }
        Ok(descriptor)
    }
}

/// Mutable framework operation registry builder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OperationRegistryBuilder {
    snapshot: OperationRegistrySnapshot,
}

impl OperationRegistryBuilder {
    /// Creates an empty registry builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an operation after validating descriptor evidence.
    pub fn register<O>(&mut self) -> std::result::Result<(), RegistryError>
    where
        O: Operation,
    {
        let descriptor = operation_descriptor::<O>()?;
        let key = OperationRegistrationKey::from_descriptor(&descriptor);
        let record = OperationRegistrationRecord {
            descriptor_id: descriptor.descriptor_id().clone(),
        };
        if let Some(existing) = self.snapshot.records.get(&key) {
            if existing != &record {
                return Err(RegistryError::DuplicateOperationRegistration {
                    kind: key.kind,
                    version: key.version,
                });
            }
        } else {
            self.snapshot.records.insert(key, record);
        }
        Ok(())
    }

    /// Returns an immutable registry snapshot.
    pub fn snapshot(&self) -> OperationRegistrySnapshot {
        self.snapshot.clone()
    }

    /// Converts this builder into an immutable registry snapshot.
    pub fn into_snapshot(self) -> OperationRegistrySnapshot {
        self.snapshot
    }
}
