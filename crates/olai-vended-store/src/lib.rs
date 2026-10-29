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
pub mod source;

pub use convert::FromStorageCredentials;
pub use error::{Error, Result};
pub use olai_storage_credentials::{
    AccessLevel, StorageCredentials, StorageLocation, StorageScope, StoreHints, VendedCredential,
};
#[cfg(any(feature = "aws", feature = "azure", feature = "gcp"))]
pub use refresh::{Clock, DEFAULT_REFRESH_MARGIN, RefreshingCredentialProvider};
pub use source::CredentialSource;
