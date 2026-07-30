use mfm_ids::{EntryPointId, StableId};
use mfm_journal::{ConfiguredValueBinding, ConfiguredValueKey, ProducerBinding, ValueRef};
use mfm_spec::RetainedValueContract;

use super::objects::{derive_value_ref, validate_value_contract, StagedObject};
use super::{
    Admit, AsyncStoreFuture, Result, RunAccessAuthority, RunHistoryReader, RunJournalBackend,
    StoreError,
};

/// Sealed exact configured value frozen for one admission.
///
/// This is not a handle to mutable configuration. It carries the immutable key binding, complete
/// retained authority, and verified bytes that admission copies into its config manifest.
pub struct VerifiedConfiguredValue {
    binding: ConfiguredValueBinding,
    value_ref: ValueRef,
    bytes: Vec<u8>,
}

impl VerifiedConfiguredValue {
    fn new(binding: ConfiguredValueBinding, value_ref: ValueRef, bytes: Vec<u8>) -> Self {
        Self {
            binding,
            value_ref,
            bytes,
        }
    }

    /// Returns the immutable configured-value binding.
    pub const fn binding(&self) -> &ConfiguredValueBinding {
        &self.binding
    }

    /// Returns complete retained configured-value authority.
    pub const fn value_ref(&self) -> &ValueRef {
        &self.value_ref
    }

    /// Returns exact verified configured bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Store-created exact configured-value resolution verifier.
pub struct ConfiguredValueResolveVerifier {
    key: ConfiguredValueKey,
    expected: RetainedValueContract,
}

impl ConfiguredValueResolveVerifier {
    fn new(key: ConfiguredValueKey, expected: RetainedValueContract) -> Self {
        Self { key, expected }
    }

    /// Returns the exact immutable lookup key.
    pub const fn key(&self) -> &ConfiguredValueKey {
        &self.key
    }

    /// Returns the exact certified configured-value contract.
    pub const fn expected_contract(&self) -> &RetainedValueContract {
        &self.expected
    }

    /// Verifies the backend-loaded immutable binding and bytes and seals the result.
    pub fn complete(
        self,
        binding: ConfiguredValueBinding,
        bytes: Vec<u8>,
    ) -> Result<VerifiedConfiguredValue> {
        let binding_fields = binding.fields()?;
        if binding_fields.key != self.key {
            return Err(StoreError::InvalidObjectAuthority {
                message: "configured-value binding key disagrees with the authorized lookup",
            });
        }
        validate_value_contract(&self.expected, &binding_fields.value_ref)?;
        let key = self.key.fields()?;
        let producer = ProducerBinding::configured_value(
            &key.store_scope_id,
            &key.tenant_scope_id,
            &key.entry_point_id,
            &key.target,
        )?;
        let expected_ref = derive_value_ref(&self.expected, &producer, &bytes)?;
        if expected_ref.as_bytes() != binding_fields.value_ref.as_bytes() {
            return Err(StoreError::InvalidObjectAuthority {
                message: "configured-value authority does not rederive from its exact binding",
            });
        }
        StagedObject::new(expected_ref.clone(), bytes.clone())?;
        Ok(VerifiedConfiguredValue::new(binding, expected_ref, bytes))
    }
}

/// Trusted backend seam for resolving one immutable configured-value binding.
pub trait ConfiguredValueBackend: RunJournalBackend {
    /// Loads the exact immutable binding and lets the store verifier seal it.
    fn backend_resolve_configured_value<'a>(
        &'a self,
        verifier: ConfiguredValueResolveVerifier,
    ) -> AsyncStoreFuture<'a, VerifiedConfiguredValue, Self::Error>;
}

impl<B: ConfiguredValueBackend> RunHistoryReader<B> {
    /// Resolves one exact value for the authority's store, tenant, and versioned entry point.
    pub fn resolve_configured_value<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<Admit>,
        entry_point_id: &'a EntryPointId,
        target: &'a StableId,
        expected: &'a RetainedValueContract,
    ) -> AsyncStoreFuture<'a, VerifiedConfiguredValue, B::Error> {
        let checked = (|| {
            let target_authority = self
                .backend()
                .store_authority_context()
                .validate_admit(authority)?;
            if &target_authority.entry_point_id != entry_point_id {
                return Err(StoreError::AdmissionAuthorityMismatch);
            }
            let key = ConfiguredValueKey::new(
                self.backend()
                    .store_authority_context()
                    .store_identity()
                    .store_scope_id(),
                &target_authority.tenant_scope_id,
                entry_point_id,
                target,
            )?;
            Ok(ConfiguredValueResolveVerifier::new(key, expected.clone()))
        })();
        match checked {
            Ok(verifier) => self.backend().backend_resolve_configured_value(verifier),
            Err(error) => Box::pin(async move { Err(error.into()) }),
        }
    }
}
