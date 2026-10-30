//! [`CredentialSource`]s for Unity Catalog tables, volumes and paths.

use async_trait::async_trait;
use olai_storage_credentials::wire::VendContext;
use olai_storage_credentials::{AccessLevel, StorageScope, StoreHints, VendedCredential};
use url::Url;

use super::client::UnityClient;
use crate::api::{local_credential, storage_root};
use crate::error::Result;
use crate::source::CredentialSource;

/// Vends credentials for one Unity Catalog securable at one access level.
///
/// Built by [`UnityCatalog`](super::UnityCatalog), which resolves names to ids and
/// storage locations once; every [`fetch`](CredentialSource::fetch) then replays the
/// same `temporary-*-credentials` request. Local (`file://`) storage needs no vend and
/// gets [`StorageCredentials::None`](olai_storage_credentials::StorageCredentials::None).
#[derive(Debug, Clone)]
pub struct UnitySource {
    client: UnityClient,
    securable: Securable,
    scope: StorageScope,
    hints: StoreHints,
    access: AccessLevel,
}

/// What a [`UnitySource`] vends for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Securable {
    /// A table, by id.
    Table(String),
    /// A volume, by id.
    Volume(String),
    /// A storage URL governed by an external location.
    Path(Url),
}

impl UnitySource {
    /// A source for `securable`, whose storage root is `storage_location`.
    pub fn new(
        client: UnityClient,
        securable: Securable,
        storage_location: &str,
        access: AccessLevel,
    ) -> Result<Self> {
        let (scope, hints) = storage_root("Unity Catalog", storage_location)?;
        Ok(Self {
            client,
            securable,
            scope,
            hints,
            access,
        })
    }

    /// What this source vends for.
    pub fn securable(&self) -> &Securable {
        &self.securable
    }

    /// The securable's storage root.
    pub fn scope(&self) -> &StorageScope {
        &self.scope
    }

    /// The same securable at another access level.
    pub fn with_access(mut self, access: AccessLevel) -> Self {
        self.access = access;
        self
    }

    /// The access level every vend asks for.
    pub fn access(&self) -> AccessLevel {
        self.access
    }
}

#[async_trait]
impl CredentialSource for UnitySource {
    async fn fetch(&self) -> Result<VendedCredential> {
        if let Some(local) = local_credential(&self.scope, self.access) {
            return Ok(local);
        }
        let wire = match &self.securable {
            Securable::Table(id) => {
                self.client
                    .temporary_table_credentials(id, self.access.into())
                    .await?
            }
            Securable::Volume(id) => {
                self.client
                    .temporary_volume_credentials(id, self.access.into())
                    .await?
            }
            Securable::Path(url) => {
                self.client
                    .temporary_path_credentials(url, self.access.into())
                    .await?
            }
        };
        let mut vended = wire.to_vended(&VendContext::new(self.scope.clone(), self.access))?;
        // What the storage URL itself implies fills what the response left out.
        vended.hints.fill_from(&self.hints);
        Ok(vended)
    }
}
