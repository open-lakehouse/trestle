//! The [`CredentialSource`] for shared volumes and skills.

use async_trait::async_trait;
use olai_storage_credentials::wire::VendContext;
use olai_storage_credentials::{AccessLevel, StorageScope, StoreHints, VendedCredential};

use super::client::{AssetKind, SharingClient};
use crate::api::{local_credential, storage_root};
use crate::error::{Error, Result};
use crate::source::CredentialSource;

/// Vends read credentials for one shared volume or skill.
///
/// Built by [`OpenSharing`](super::OpenSharing), which looks the asset up once for its
/// storage location; every [`fetch`](CredentialSource::fetch) then replays the same
/// `temporary-*-credentials` request. Sharing is read-only, so the credential is
/// always for [`AccessLevel::Read`].
#[derive(Debug, Clone)]
pub struct SharingSource {
    client: SharingClient,
    kind: AssetKind,
    share: String,
    schema: String,
    name: String,
    scope: StorageScope,
    hints: StoreHints,
}

impl SharingSource {
    /// Looks up `share.schema.name` for its storage location and binds a source to it.
    pub async fn load(
        client: SharingClient,
        kind: AssetKind,
        share: &str,
        schema: &str,
        name: &str,
    ) -> Result<Self> {
        let info = client.get_asset(kind, share, schema, name).await?;
        let location = info.storage_location.ok_or_else(|| {
            Error::InvalidReference(format!(
                "Open Sharing returned no storage location for `{share}.{schema}.{name}`"
            ))
        })?;
        let (scope, hints) = storage_root("Open Sharing", &location)?;
        Ok(Self {
            client,
            kind,
            share: share.to_owned(),
            schema: schema.to_owned(),
            name: name.to_owned(),
            scope,
            hints,
        })
    }

    /// The asset's storage root.
    pub fn scope(&self) -> &StorageScope {
        &self.scope
    }

    /// The kind of asset this source vends for.
    pub fn kind(&self) -> AssetKind {
        self.kind
    }
}

#[async_trait]
impl CredentialSource for SharingSource {
    async fn fetch(&self) -> Result<VendedCredential> {
        if let Some(local) = local_credential(&self.scope, AccessLevel::Read) {
            return Ok(local);
        }
        let wire = self
            .client
            .temporary_credentials(self.kind, &self.share, &self.schema, &self.name)
            .await?;
        let mut vended =
            wire.to_vended(&VendContext::new(self.scope.clone(), AccessLevel::Read))?;
        vended.hints.fill_from(&self.hints);
        Ok(vended)
    }
}
