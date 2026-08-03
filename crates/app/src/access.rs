use async_trait::async_trait;
use mfm_ids::{InvocationIdentity, RunId, StableId, StoreScopeId, TenantScopeId};
use zeroize::{Zeroize, Zeroizing};

/// Maximum opaque credential size admitted by the application boundary.
pub const MAX_SECRET_CREDENTIAL_BYTES: usize = 64 * 1024;

/// Opaque caller credential accepted by the run-facing application boundary.
///
/// The credential is moved into one facade call and retained only for that call. It deliberately
/// implements no cloning, formatting, or serialization traits. The injected policy may borrow the
/// bytes for each independently required authorization decision without copying them.
pub struct SecretCredential(Zeroizing<Vec<u8>>);

impl SecretCredential {
    /// Moves non-empty bounded opaque credential bytes into zeroizing storage.
    pub fn new(mut bytes: Vec<u8>) -> Result<Self, SecretCredentialError> {
        if bytes.is_empty() || bytes.len() > MAX_SECRET_CREDENTIAL_BYTES {
            bytes.zeroize();
            return Err(SecretCredentialError);
        }
        Ok(Self(Zeroizing::new(bytes)))
    }

    /// Borrows the opaque bytes for an injected authentication policy.
    ///
    /// Policy implementations must not log, format, persist, or copy this value.
    pub fn expose_to_policy(&self) -> &[u8] {
        self.0.as_slice()
    }
}

/// Error returned when credential bytes are empty or exceed the application bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("credential bytes are empty or exceed the application limit")]
pub struct SecretCredentialError;

/// Closed run-access grants understood by the application policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunAccessGrant {
    /// Admit one exact logical invocation.
    Admit,
    /// Execute at most one run action.
    Drive,
    /// Perform one replay operation.
    Replay,
    /// Read the exact public view for one run.
    ReadPublic,
    /// Inspect transition trace material.
    InspectTrace,
    /// Inspect the safe access audit.
    InspectAudit,
    /// Export one portable run stream.
    Export,
}

/// Exact target supplied to the injected access policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessTarget {
    /// Admission target, before a run id exists.
    AdmitTarget {
        /// Store whose admission authority will be used.
        store_scope_id: StoreScopeId,
        /// Stable unversioned operation identity.
        entry_point_operation_id: StableId,
        /// Exact configured target selected by the public admission input.
        configured_target: StableId,
        /// Canonical caller-supplied UUIDv4 invocation identity.
        invocation_identity: InvocationIdentity,
    },
    /// Existing exact-run target.
    RunTarget {
        /// Store whose run authority will be used.
        store_scope_id: StoreScopeId,
        /// Exact typed run id.
        run_id: RunId,
    },
}

/// Trusted tenant and authenticated-principal result returned by the injected policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedTenant {
    tenant_scope_id: TenantScopeId,
    authenticated_principal_id: StableId,
}

impl AuthorizedTenant {
    /// Constructs the trusted result inside a deployment policy implementation.
    ///
    /// `authenticated_principal_id` must be derived from the authenticated credential, never
    /// from admission input or configured semantic values.
    pub fn new(tenant_scope_id: TenantScopeId, authenticated_principal_id: StableId) -> Self {
        Self {
            tenant_scope_id,
            authenticated_principal_id,
        }
    }

    /// Returns the immutable tenant selected by the trusted policy.
    pub const fn tenant_scope_id(&self) -> &TenantScopeId {
        &self.tenant_scope_id
    }

    /// Returns the stable principal derived from the authenticated credential.
    pub const fn authenticated_principal_id(&self) -> &StableId {
        &self.authenticated_principal_id
    }

    pub(crate) fn into_parts(self) -> (TenantScopeId, StableId) {
        (self.tenant_scope_id, self.authenticated_principal_id)
    }
}

/// Closed policy rejection classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AccessPolicyError {
    /// The credential is missing, malformed, expired, revoked, or otherwise invalid.
    #[error("authentication required")]
    AuthenticationRequired,
    /// The credential is valid but the exact grant and target are denied.
    #[error("grant denied")]
    GrantDenied,
}

/// Deployment-supplied authentication and authorization boundary.
///
/// The application calls this policy for every facade operation and again for every separately
/// protected cross-run source or export dependency. Implementations must map one credential to one
/// immutable tenant and stable authenticated principal and must not retain an application session.
#[async_trait]
pub trait RunAccessPolicy: Send + Sync {
    /// Authenticates and authorizes one exact grant and target.
    async fn authorize(
        &self,
        credential: &SecretCredential,
        grant: RunAccessGrant,
        target: &AccessTarget,
    ) -> Result<AuthorizedTenant, AccessPolicyError>;
}

#[cfg(test)]
mod tests {
    use super::{SecretCredential, MAX_SECRET_CREDENTIAL_BYTES};
    use static_assertions::assert_not_impl_any;

    assert_not_impl_any!(
        SecretCredential:
            Clone,
            Copy,
            std::fmt::Debug,
            std::fmt::Display,
            serde::Serialize,
            serde::de::DeserializeOwned
    );

    #[test]
    fn credential_size_is_enforced_at_the_application_boundary() {
        assert!(SecretCredential::new(vec![1; MAX_SECRET_CREDENTIAL_BYTES]).is_ok());
        assert!(SecretCredential::new(vec![1; MAX_SECRET_CREDENTIAL_BYTES + 1]).is_err());
    }
}
