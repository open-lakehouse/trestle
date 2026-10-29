//! Containerized cloud storage emulators for integration tests.
//!
//! Each emulator starts a throwaway container through `testcontainers`, pinned to one
//! image tag, and stops it when dropped. Tests that use them need Docker, so consumers
//! gate them behind an `emulator-it` feature and the default `cargo test` stays
//! container-free:
//!
//! ```toml
//! [features]
//! emulator-it = []
//! ```
//!
//! ```ignore
//! #![cfg(feature = "emulator-it")]
//! let rustfs = olai_test_emulators::RustFs::start().await;
//! rustfs.create_bucket("data").await;
//! ```
//!
//! Locally, any Docker-compatible runtime works (Docker Desktop, OrbStack, colima with
//! `DOCKER_HOST` set). CI runs these in the `emulator-tests` job.

pub mod azurite;
pub mod rustfs;

pub use azurite::Azurite;
pub use rustfs::RustFs;
