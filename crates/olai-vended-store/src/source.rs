//! Where vended credentials come from.

use std::fmt::Debug;

use async_trait::async_trait;
use olai_storage_credentials::VendedCredential;

use crate::error::Result;

/// Vends fresh credentials for one securable at one access level.
///
/// A source is bound to its securable (a table, a volume, a path, ...) and access level
/// when it is built, and every [`fetch`](Self::fetch) replays exactly that request.
/// That is what keeps a refresh from ever widening a store's grant.
///
/// Protocol modules implement this once per securable kind. On `wasm32` the returned
/// future need not be `Send`, because browser HTTP futures are not;
/// [`RefreshingCredentialProvider`](crate::RefreshingCredentialProvider) bridges that
/// to object_store's `Send` requirement.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait CredentialSource: Debug + Send + Sync + 'static {
    /// Vend a fresh credential for the securable this source is bound to.
    async fn fetch(&self) -> Result<VendedCredential>;
}
