#![cfg_attr(docsrs, feature(doc_cfg))]
//! `object_store` stores backed by vended cloud storage credentials.
//!
//! Lakehouse protocols (Unity Catalog REST, the UC Delta API, Iceberg REST, Open
//! Sharing) hand clients short-lived storage credentials scoped to a prefix. This crate
//! is the client side: it turns those credentials into object stores that re-vend
//! before they expire.
//!
//! - [`CredentialSource`]: vends a fresh [`VendedCredential`] for the one securable and
//!   access level it is bound to. Protocol modules implement it.
//! - [`RefreshingCredentialProvider`] (features `aws`, `azure`, `gcp`): an object_store
//!   `CredentialProvider` over a source. It is seeded, single-flight, and re-vends a
//!   configurable margin before expiry.
//! - [`build_store`]: builds the store for a vended credential (S3, R2, Azure, Azurite,
//!   GCS, local, in-memory), configured by its [`StoreHints`] and [`StoreOptions`], as a
//!   [`ScopedStore`].
//! - [`RoutingObjectStore`]: one store for a bucket that routes each request to the
//!   [`ScopedStore`] covering its path, for registries keyed by `scheme://bucket`.
//! - [`FromStorageCredentials`]: the mapping from the canonical credential onto
//!   object_store's per-cloud credential types.
//!
//! The vocabulary (credentials, locations, scopes) is [`olai_storage_credentials`].
//! The core builds for `wasm32-unknown-unknown`, where sources may return `!Send`
//! futures.
//!
//! See the [credential vending design record] for how this fits with the model and
//! server crates.
//!
//! [credential vending design record]: https://github.com/open-lakehouse/trestle/blob/main/docs/credential-vending.md

pub mod convert;
pub mod error;
#[cfg(any(feature = "aws", feature = "azure", feature = "gcp"))]
#[cfg_attr(
    docsrs,
    doc(cfg(any(feature = "aws", feature = "azure", feature = "gcp")))
)]
pub mod refresh;
#[cfg(all(feature = "aws", not(target_arch = "wasm32")))]
#[cfg_attr(docsrs, doc(cfg(all(feature = "aws", not(target_arch = "wasm32")))))]
pub mod region;
pub mod routing;
pub mod source;
pub mod store;

pub use convert::FromStorageCredentials;
pub use error::{Error, Result};
pub use olai_storage_credentials::{
    AccessLevel, StorageCredentials, StorageLocation, StorageScope, StoreHints, VendedCredential,
};
#[cfg(any(feature = "aws", feature = "azure", feature = "gcp"))]
pub use refresh::{Clock, DEFAULT_REFRESH_MARGIN, RefreshingCredentialProvider};
#[cfg(all(feature = "aws", not(target_arch = "wasm32")))]
pub use region::BucketRegions;
pub use routing::RoutingObjectStore;
pub use source::CredentialSource;
pub use store::{ScopedStore, StoreOptions, build_store};
