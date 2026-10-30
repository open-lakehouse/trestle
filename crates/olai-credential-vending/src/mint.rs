//! Minting a downscoped credential.

use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use olai_storage_credentials::{
    AccessLevel, StorageCredentials, StorageProvider, StorageScope, VendedCredential,
};

use crate::error::{Error, Result};
use crate::{
    AwsStsMinter, AzureAccountKeySasMinter, AzureUserDelegationSasMinter, GcsDownscopedMinter,
    RegisteredCredential,
};

/// What to mint: access to one scope, for a while.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MintRequest {
    /// The storage the credential must cover, and nothing outside it.
    pub scope: StorageScope,
    /// What the credential may do there.
    pub access: AccessLevel,
    /// How long it should live; the minter's default when `None`. Minters clamp it to
    /// what the cloud allows, which can lengthen a short request: STS issues at least
    /// 15 minutes.
    ///
    /// Through a [`CachingMinter`](crate::CachingMinter), a credential may come from
    /// the cache with less left: at least half the lifetime it was minted with. Clients
    /// should refresh on [`VendedCredential::expires_at`], not on the requested `ttl`.
    pub ttl: Option<Duration>,
    /// Who the credential is for, for audit: the STS `RoleSessionName`, for example.
    pub session_name: Option<String>,
}

impl MintRequest {
    /// A request for `access` on `scope` with the minter's default lifetime.
    pub fn new(scope: StorageScope, access: AccessLevel) -> Self {
        Self {
            scope,
            access,
            ttl: None,
            session_name: None,
        }
    }

    /// Sets the lifetime.
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = Some(ttl);
        self
    }

    /// Sets the audit session name.
    pub fn with_session_name(mut self, session_name: impl Into<String>) -> Self {
        self.session_name = Some(session_name.into());
        self
    }
}

/// Mints downscoped credentials from one registered principal.
///
/// A minted credential covers exactly [`MintRequest::scope`] at
/// [`MintRequest::access`]; the cloud enforces that (an STS session policy, a SAS scoped
/// to a container or directory, a Credential Access Boundary).
#[async_trait]
pub trait CredentialMinter: Debug + Send + Sync + 'static {
    /// Mints a credential for `request`.
    async fn mint(&self, request: &MintRequest) -> Result<VendedCredential>;
}

/// The minter for `credential`, with default settings: [`AwsStsMinter`] for IAM roles,
/// [`AzureUserDelegationSasMinter`] for service principals and managed identities,
/// [`AzureAccountKeySasMinter`] for account keys, and [`GcsDownscopedMinter`] for GCP
/// service accounts.
///
/// Build the minters directly to set endpoints, client options or audit fields.
///
/// [`AwsStsMinter`]: crate::AwsStsMinter
/// [`AzureUserDelegationSasMinter`]: crate::AzureUserDelegationSasMinter
/// [`AzureAccountKeySasMinter`]: crate::AzureAccountKeySasMinter
/// [`GcsDownscopedMinter`]: crate::GcsDownscopedMinter
pub fn minter_for(credential: &RegisteredCredential) -> Result<Arc<dyn CredentialMinter>> {
    use RegisteredCredential as R;
    Ok(match credential {
        R::AwsIamRole(role) => Arc::new(AwsStsMinter::new(role)?),
        R::AzureServicePrincipal(principal) => Arc::new(
            AzureUserDelegationSasMinter::from_service_principal(principal)?,
        ),
        R::AzureManagedIdentity(identity) => Arc::new(
            AzureUserDelegationSasMinter::from_managed_identity(identity)?,
        ),
        R::AzureStorageKey(key) => Arc::new(AzureAccountKeySasMinter::new(key)),
        R::GcpServiceAccount(account) => Arc::new(GcsDownscopedMinter::new(account)?),
    })
}

/// Mints for local (`file://`) and in-memory storage, which need no secret.
///
/// No [`RegisteredCredential`](crate::RegisteredCredential) governs local storage, so
/// neither [`minter_for`] nor [`Vendor`](crate::Vendor) returns this minter: a server
/// that serves local storage calls it directly.
#[derive(Debug, Clone, Copy, Default)]
pub struct LocalMinter;

#[async_trait]
impl CredentialMinter for LocalMinter {
    async fn mint(&self, request: &MintRequest) -> Result<VendedCredential> {
        match request.scope.location().provider() {
            StorageProvider::Local | StorageProvider::Memory => Ok(VendedCredential::new(
                StorageCredentials::None,
                request.scope.clone(),
                request.access,
            )),
            _ => Err(Error::unsupported(
                &request.scope,
                "the local minter only serves file:// and memory:// storage",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RegisteredCredential;
    use crate::registered::{
        AwsIamRole, AzureClientSecret, AzureManagedIdentity, AzureServicePrincipal,
        AzureStorageKey, GcpServiceAccount, GcpServiceAccountKey,
    };

    #[test]
    fn each_registered_credential_gets_its_minter() {
        let cases = [
            (
                RegisteredCredential::AwsIamRole(AwsIamRole {
                    role_arn: "arn:aws:iam::1:role/r".into(),
                    access_key_id: Some("AKID".into()),
                    secret_access_key: Some("SECRET".into()),
                    ..AwsIamRole::default()
                }),
                "AwsStsMinter",
            ),
            (
                RegisteredCredential::AzureServicePrincipal(AzureServicePrincipal {
                    directory_id: "tenant".into(),
                    application_id: "app".into(),
                    secret: AzureClientSecret::ClientSecret("SECRET".into()),
                }),
                "AzureUserDelegationSasMinter",
            ),
            (
                RegisteredCredential::AzureManagedIdentity(AzureManagedIdentity::default()),
                "AzureUserDelegationSasMinter",
            ),
            (
                RegisteredCredential::AzureStorageKey(AzureStorageKey {
                    account_name: "acct".into(),
                    account_key: "a2V5".into(),
                }),
                "AzureAccountKeySasMinter",
            ),
            (
                RegisteredCredential::GcpServiceAccount(GcpServiceAccount {
                    email: "sa@p.iam.gserviceaccount.com".into(),
                    key: Some(GcpServiceAccountKey::ServiceAccountKey(
                        r#"{"client_email":"sa@p.iam.gserviceaccount.com"}"#.into(),
                    )),
                }),
                "GcsDownscopedMinter",
            ),
        ];
        for (credential, minter) in cases {
            let debug = format!("{:?}", minter_for(&credential).unwrap());
            assert!(debug.starts_with(minter), "{}: {debug}", credential.kind());
        }
    }

    #[tokio::test]
    async fn local_minter_serves_local_storage_only() {
        let local = MintRequest::new("file:///data/tab".parse().unwrap(), AccessLevel::ReadWrite);
        let vended = LocalMinter.mint(&local).await.unwrap();
        assert_eq!(vended.credentials, StorageCredentials::None);
        assert_eq!(vended.scope, local.scope);
        assert_eq!(vended.expires_at, None);

        let cloud = MintRequest::new("s3://bucket/tab".parse().unwrap(), AccessLevel::Read);
        let err = LocalMinter.mint(&cloud).await.unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }), "{err}");
    }
}
