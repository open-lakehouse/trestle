#![cfg_attr(docsrs, feature(doc_cfg))]
//! The canonical model for vended cloud storage credentials.
//!
//! Lakehouse protocols (Unity Catalog REST, the UC Delta API, Iceberg REST, Open
//! Sharing) hand clients short-lived storage credentials scoped to a storage prefix.
//! This crate is the protocol-neutral vocabulary for them, shared by the client side
//! (turning credentials into object stores) and the server side (minting them):
//!
//! - [`model`]: [`VendedCredential`], the [`StorageCredentials`] secret it carries, the
//!   [`AccessLevel`] it grants and the [`StoreHints`] for building a store.
//! - [`location`]: canonical [`StorageLocation`]s, the [`StorageScope`] a credential
//!   covers, and segment-boundary [`longest_match`](location::longest_match).
//! - [`sas`]: Azure SAS decoding and expiry.
//!
//! The crate makes no network calls, never reads the clock (callers pass `now`), and
//! builds for `wasm32-unknown-unknown`. Secret-bearing types redact their `Debug`
//! output.
//!
//! See the [credential vending design record] for how this fits with the client and
//! server crates.
//!
//! [credential vending design record]: https://github.com/open-lakehouse/trestle/blob/main/docs/credential-vending.md

pub mod error;
pub mod location;
pub mod model;
pub mod policy;
pub mod sas;

pub use error::{Error, Result};
pub use location::{StorageLocation, StorageProvider, StorageScope};
pub use model::{AccessLevel, StorageCredentials, StoreHints, VendedCredential};
