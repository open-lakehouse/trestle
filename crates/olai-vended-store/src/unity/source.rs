//! [`CredentialSource`]s for Unity Catalog tables, volumes and paths.

use async_trait::async_trait;
use olai_storage_credentials::wire::VendContext;
use olai_storage_credentials::{
    AccessLevel, StorageCredentials, StorageLocation, StorageProvider, StorageScope, StoreHints,
    VendedCredential,
};
use url::Url;

use super::client::UnityClient;
use crate::error::{Error, Result};
use crate::source::CredentialSource;

/// Vends credentials for one Unity Catalog securable at one access level.
///
/// Built by [`UnityCatalog`](super::UnityCatalog), which resolves names to ids and
/// storage locations once; every [`fetch`](CredentialSource::fetch) then replays the
/// same `temporary-*-credentials` request. Local (`file://`) storage needs no vend and
/// gets [`StorageCredentials::None`].
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
        let url = Url::parse(storage_location).map_err(|e| {
            Error::InvalidReference(format!(
                "Unity Catalog returned an invalid storage location `{storage_location}`: {e}"
            ))
        })?;
        let (location, hints) = StorageLocation::parse_with_hints(&url)?;
        Ok(Self {
            client,
            securable,
            scope: StorageScope::new(location),
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
        if self.scope.location().provider() == StorageProvider::Local {
            return Ok(VendedCredential::new(
                StorageCredentials::None,
                self.scope.clone(),
                self.access,
            ));
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
        // What the storage URL itself implies (an emulator endpoint, a regional S3
        // host) fills what the response left out.
        let hints = &mut vended.hints;
        hints.endpoint = hints
            .endpoint
            .take()
            .or_else(|| self.hints.endpoint.clone());
        hints.region = hints.region.take().or_else(|| self.hints.region.clone());
        hints.access_point = hints
            .access_point
            .take()
            .or_else(|| self.hints.access_point.clone());
        hints.path_style = hints.path_style.or(self.hints.path_style);
        hints.allow_http = hints.allow_http.or(self.hints.allow_http);
        Ok(vended)
    }
}
