//! A secret discovery service (SDS) server for Envoy and other consumers, and a key service
//! for data keys.
//!
//! The server serves, on one Unix socket per consumer:
//!
//! - Envoy's v3 `SecretDiscoveryService` (`StreamSecrets` and `FetchSecrets`,
//!   `generic_secret` resources only), for tokens and other values a consumer uses as is.
//! - `olai.keys.v1.KeyService` (`Wrap`, `Unwrap`, `GetKey`, `WatchKey`; see
//!   [`olai_key_service`]), for key encryption keys a consumer must never hold: it sends its
//!   data keys to be wrapped and unwrapped under them instead ([`keys`], [`keyring`]).
//!
//! A connection's consumer is the socket it arrived on ([`caller`]), and each consumer sees
//! only the secrets and keys its listener lists ([`Policy`]). Values and keyrings come from a
//! source ([`source`]): the process environment, read once at startup, or a host endpoint
//! the server follows live, so a value changed there reaches open streams without a restart.
//! The configuration file only names variables.
//!
//! ```no_run
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! use olai_secret_server::{Config, HostOptions, resolve, serve};
//!
//! let config = Config::load("secret-server.yaml".as_ref())?;
//! let resolved = resolve(&config, |name| std::env::var_os(name), HostOptions::default()).await?;
//! let _watching = resolved.watcher.map(|w| w.spawn());
//! serve(&config, resolved.policy, async { let _ = tokio::signal::ctrl_c().await; }).await?;
//! # Ok(())
//! # }
//! ```
//!
//! See the crate README for the configuration file and the Envoy side.

pub mod caller;
pub mod config;
pub mod keyring;
pub mod keys;
pub mod policy;
pub mod sds;
pub mod secret;
pub mod server;
pub mod source;

pub use caller::{CallerId, CallerResolver, ConsumerConn, SocketCaller};
pub use config::{
    Config, ConfigError, InvalidKeyring, KeyRef, ListenerConfig, SecretRef, SourceConfig,
};
pub use keyring::{KeyHandle, Keyring, KeyringError};
pub use keys::KeyService;
pub use policy::{Policy, SecretCell};
pub use sds::SdsService;
pub use secret::SecretValue;
pub use server::{ServeError, bind_socket, serve, serve_listener};
pub use source::{HostOptions, HostWatcher, Resolved, WatchGuard, resolve};
