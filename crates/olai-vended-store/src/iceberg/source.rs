//! The [`CredentialSource`] for Iceberg REST tables.

use async_trait::async_trait;
use olai_storage_credentials::wire::iceberg::StorageCredential;
use olai_storage_credentials::wire::{VendContext, select_covering};
use olai_storage_credentials::{AccessLevel, StorageScope, StoreHints, VendedCredential};

use super::client::{IcebergClient, LoadTable};
use crate::api::{ApiError, local_credential};
use crate::error::{Error, Result};
use crate::source::CredentialSource;
use crate::store::unsupported;

/// Vends credentials for one Iceberg table, for one prefix of its storage.
///
/// Built by [`IcebergCatalog`](super::IcebergCatalog), which loads the table once for
/// its location. Every [`fetch`](CredentialSource::fetch) asks the table's
/// `credentials` endpoint, or replays `loadTable` on servers without one, and keeps
/// the most specific credential covering the source's scope.
///
/// Iceberg REST has no way to ask for an access level: the catalog grants what the
/// caller's principal may do. The source's access level is what the caller declares
/// it needs, and labels the credential.
#[derive(Debug, Clone)]
pub struct IcebergSource {
    client: IcebergClient,
    namespace: Vec<String>,
    table: String,
    scope: StorageScope,
    hints: StoreHints,
    access: AccessLevel,
}

impl IcebergSource {
    pub(super) fn new(
        client: IcebergClient,
        namespace: Vec<String>,
        table: String,
        scope: StorageScope,
        hints: StoreHints,
        access: AccessLevel,
    ) -> Self {
        Self {
            client,
            namespace,
            table,
            scope,
            hints,
            access,
        }
    }

    /// The storage this source vends for: the table location, or a prefix within it.
    pub fn scope(&self) -> &StorageScope {
        &self.scope
    }

    /// The access level credentials are labelled with.
    pub fn access(&self) -> AccessLevel {
        self.access
    }

    /// What the table location implies for its stores.
    pub(super) fn hints(&self) -> &StoreHints {
        &self.hints
    }

    fn context(&self) -> VendContext {
        VendContext::new(self.scope.clone(), self.access)
    }
}

#[async_trait]
impl CredentialSource for IcebergSource {
    async fn fetch(&self) -> Result<VendedCredential> {
        if let Some(local) = local_credential(&self.scope, self.access) {
            return Ok(local);
        }
        let ctx = self.context();
        let mut vended = match self
            .client
            .load_credentials(&self.namespace, &self.table)
            .await
        {
            Ok(response) => select(&response.storage_credentials, &ctx)?,
            Err(err) if ApiError::find(&err).is_some_and(ApiError::is_not_found) => {
                let table = self.client.load_table(&self.namespace, &self.table).await?;
                vend_from_table(&table, &ctx)?
            }
            Err(err) => return Err(err),
        };
        vended.hints.fill_from(&self.hints);
        Ok(vended)
    }
}

/// The credential a `loadTable` response vends for `ctx`.
///
/// Refuses remote signing and responses without credentials: a store built from
/// nothing would send unsigned requests.
pub(super) fn vend_from_table(table: &LoadTable, ctx: &VendContext) -> Result<VendedCredential> {
    let credentials = &table.credentials;
    if !credentials.storage_credentials.is_empty() {
        return select(&credentials.storage_credentials, ctx);
    }
    if credentials.remote_signing() {
        return Err(unsupported(
            &ctx.scope,
            "the catalog signs requests remotely instead of vending credentials",
        ));
    }
    credentials
        .to_vended(ctx)?
        .into_iter()
        .next()
        .ok_or_else(|| unsupported(&ctx.scope, "the catalog vended no storage credentials"))
}

/// Of `credentials`, the most specific covering all of `ctx.scope`, clamped to it.
pub(super) fn select(
    credentials: &[StorageCredential],
    ctx: &VendContext,
) -> Result<VendedCredential> {
    let prefixes = credentials
        .iter()
        .map(|credential| credential.prefix.parse::<StorageScope>())
        .collect::<std::result::Result<Vec<_>, _>>()?;
    // Iceberg REST does not say what a credential grants; each is taken to grant
    // what was asked for.
    let offered = prefixes
        .iter()
        .zip(credentials)
        .map(|(prefix, credential)| (prefix, ctx.access, credential));
    let chosen = select_covering(offered, ctx).ok_or_else(|| {
        Error::Credentials(olai_storage_credentials::Error::InvalidCredential(format!(
            "no vended credential covers {}",
            ctx.scope
        )))
    })?;
    // A prefix covering the scope always relates to it, so this is `Some`.
    chosen.to_vended(ctx)?.ok_or_else(|| {
        Error::Credentials(olai_storage_credentials::Error::InvalidCredential(format!(
            "credential for `{}` does not relate to {}",
            chosen.prefix, ctx.scope
        )))
    })
}
