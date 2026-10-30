//! Server-side minting of downscoped cloud storage credentials.
//!
//! A vending server (Unity Catalog, Iceberg REST, Open Sharing) hands clients
//! short-lived credentials scoped to one storage prefix. This crate mints them from the
//! long-lived principals a host registers:
//!
//! - [`RegisteredCredential`]: a registered principal (an IAM role, an Azure service
//!   principal or managed identity, a storage account key, a GCP service account), in
//!   Unity Catalog's JSON shape.
//! - [`CredentialResolver`]: which registered credential governs a location;
//!   [`ExternalLocations`] is the longest-prefix rule over a list.
//! - [`CredentialMinter`]: mints a [`VendedCredential`] for a [`MintRequest`], covering
//!   exactly the requested scope at the requested access. [`AwsStsMinter`] assumes an
//!   IAM role with a session policy, [`AzureUserDelegationSasMinter`] and
//!   [`AzureAccountKeySasMinter`] sign SAS, [`GcsDownscopedMinter`] exchanges a
//!   service account's token for one restricted by a Credential Access Boundary, and
//!   [`LocalMinter`] serves local storage.
//!   [`minter_for`] picks the minter for a registered credential.
//! - [`CachingMinter`]: a bounded cache of minted credentials, keyed by the whole
//!   request. [`Vendor`] resolves, mints and caches per registered credential.
//!
//! The vocabulary (credentials, locations, scopes, downscoping policies) is
//! [`olai_storage_credentials`]; the cloud calls are `olai-http`'s. Native only.
//!
//! See the [credential vending design record] for how this fits with the model and
//! client crates.
//!
//! [credential vending design record]: https://github.com/open-lakehouse/trestle/blob/main/docs/credential-vending.md

pub mod aws;
pub mod azure;
pub mod cache;
pub mod error;
pub mod gcp;
pub mod mint;
pub mod registered;
pub mod resolve;

pub use aws::AwsStsMinter;
pub use azure::{AzureAccountKeySasMinter, AzureUserDelegationSasMinter};
pub use cache::{CachingMinter, Vendor};
pub use error::{Error, Result};
pub use gcp::GcsDownscopedMinter;
pub use mint::{CredentialMinter, LocalMinter, MintRequest, minter_for};
pub use olai_storage_credentials::{
    AccessLevel, StorageCredentials, StorageLocation, StorageScope, VendedCredential,
};
pub use registered::RegisteredCredential;
pub use resolve::{CredentialResolver, ExternalLocations};
