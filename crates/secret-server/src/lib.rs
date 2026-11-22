//! A secret discovery service (SDS) server for Envoy and other consumers.
//!
//! The server serves Envoy's v3 `SecretDiscoveryService` (`StreamSecrets` and
//! `FetchSecrets`, `generic_secret` resources only) on one Unix socket per consumer. A
//! connection's consumer is the socket it arrived on ([`caller`]), and each consumer sees only
//! the secrets its listener lists ([`Policy`]). Values come from the process environment,
//! read once at startup ([`Config::resolve`]); the configuration file only names variables.
//!
//! ```no_run
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! use olai_secret_server::{Config, serve};
//!
//! let config = Config::load("secret-server.yaml".as_ref())?;
//! let policy = config.resolve(|name| std::env::var_os(name))?;
//! serve(&config, policy, async { let _ = tokio::signal::ctrl_c().await; }).await?;
//! # Ok(())
//! # }
//! ```
//!
//! See the crate README for the configuration file and the Envoy side.

pub mod caller;
pub mod config;
pub mod policy;
pub mod sds;
pub mod secret;
pub mod server;

pub use caller::{CallerId, CallerResolver, ConsumerConn, SocketCaller};
pub use config::{Config, ConfigError, ListenerConfig, SecretRef};
pub use policy::Policy;
pub use sds::SdsService;
pub use secret::SecretValue;
pub use server::{ServeError, bind_socket, serve, serve_listener};
