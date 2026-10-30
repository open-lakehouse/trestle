//! Server-side minting of downscoped cloud storage credentials.
//!
//! A vending server (Unity Catalog, Iceberg REST, Open Sharing) hands clients
//! short-lived credentials scoped to one storage prefix. This crate mints them from the
//! long-lived principals a host registers:
//!
//! - [`RegisteredCredential`]: a registered principal (an IAM role, an Azure service
//!   principal or managed identity, a storage account key), in Unity Catalog's JSON
//!   shape.
//! - [`CredentialResolver`]: which registered credential governs a location;
//!   [`ExternalLocations`] is the longest-prefix rule over a list.
//! - [`CredentialMinter`]: mints a [`VendedCredential`] for a [`MintRequest`], covering
//!   exactly the requested scope at the requested access. [`AwsStsMinter`] assumes an
//!   IAM role with a session policy; [`LocalMinter`] serves local storage.
//!
//! The vocabulary (credentials, locations, scopes, downscoping policies) is
//! [`olai_storage_credentials`]; the cloud calls are `olai-http`'s. Native only.
//!
//! See the [credential vending design record] for how this fits with the model and
//! client crates.
//!
//! [credential vending design record]: https://github.com/open-lakehouse/trestle/blob/main/docs/credential-vending.md

pub mod aws;
pub mod error;
pub mod mint;
pub mod registered;
pub mod resolve;

pub use aws::AwsStsMinter;
pub use error::{Error, Result};
pub use mint::{CredentialMinter, LocalMinter, MintRequest};
pub use olai_storage_credentials::{
    AccessLevel, StorageCredentials, StorageLocation, StorageScope, VendedCredential,
};
pub use registered::RegisteredCredential;
pub use resolve::{CredentialResolver, ExternalLocations};
