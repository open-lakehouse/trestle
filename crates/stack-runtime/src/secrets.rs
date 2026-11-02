//! Resolving [`SecretRef`]s: the [`SecretProvider`] trait and its providers.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use olai_stack_state::SecretRef;

/// A secret's value. Its `Debug` and `Display` print `<redacted>`.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(String);

impl SecretValue {
    /// Wrap `value`.
    pub fn new(value: impl Into<String>) -> Self {
        SecretValue(value.into())
    }

    /// The value.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl fmt::Display for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Why a secret couldn't be resolved.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    /// No provider of that name is configured. There is no fallback to another provider.
    #[error("no secret provider `{0}` is configured")]
    UnknownProvider(String),
    /// The provider has no value under the key.
    #[error("secret `{key}` is not set in provider `{provider}`")]
    Missing {
        /// The provider.
        provider: String,
        /// The key.
        key: String,
    },
    /// The provider failed (e.g. the keychain refused access).
    #[error("secret provider `{provider}` failed for `{key}`: {reason}")]
    Provider {
        /// The provider.
        provider: String,
        /// The key.
        key: String,
        /// What went wrong.
        reason: String,
    },
}

/// Where secret values live. The store holds [`SecretRef`]s naming a provider and a key; the
/// provider resolves them at launch.
pub trait SecretProvider: Send + Sync {
    /// The name [`SecretRef::provider`] uses for it.
    fn name(&self) -> &str;

    /// The value stored under `key`.
    fn get(&self, key: &str) -> Result<SecretValue, SecretError>;
}

/// Secrets from this process's environment: the key is a variable name. Named `env`.
#[derive(Clone, Debug, Default)]
pub struct EnvProvider;

impl SecretProvider for EnvProvider {
    fn name(&self) -> &str {
        "env"
    }

    fn get(&self, key: &str) -> Result<SecretValue, SecretError> {
        std::env::var(key)
            .map(SecretValue::new)
            .map_err(|_| SecretError::Missing {
                provider: self.name().into(),
                key: key.into(),
            })
    }
}

/// Secrets in the OS credential store (macOS Keychain, Windows Credential Manager, the Linux
/// kernel keyring), under one service name; the key is the account. Named `keychain`.
#[cfg(feature = "keychain")]
#[derive(Clone, Debug)]
pub struct KeychainProvider {
    service: String,
}

#[cfg(feature = "keychain")]
impl KeychainProvider {
    /// The provider for entries under `service` (e.g. `trestle`).
    pub fn new(service: impl Into<String>) -> Self {
        KeychainProvider {
            service: service.into(),
        }
    }

    fn entry(&self, key: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(&self.service, key).map_err(|e| self.failed(key, e))
    }

    fn failed(&self, key: &str, e: keyring::Error) -> SecretError {
        SecretError::Provider {
            provider: "keychain".into(),
            key: key.into(),
            reason: e.to_string(),
        }
    }

    /// Store `value` under `key`.
    pub fn set(&self, key: &str, value: &SecretValue) -> Result<(), SecretError> {
        self.entry(key)?
            .set_password(value.expose())
            .map_err(|e| self.failed(key, e))
    }

    /// Remove the entry under `key`, if any.
    pub fn delete(&self, key: &str) -> Result<(), SecretError> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(self.failed(key, e)),
        }
    }
}

#[cfg(feature = "keychain")]
impl SecretProvider for KeychainProvider {
    fn name(&self) -> &str {
        "keychain"
    }

    fn get(&self, key: &str) -> Result<SecretValue, SecretError> {
        match self.entry(key)?.get_password() {
            Ok(value) => Ok(SecretValue::new(value)),
            Err(keyring::Error::NoEntry) => Err(SecretError::Missing {
                provider: self.name().into(),
                key: key.into(),
            }),
            Err(e) => Err(self.failed(key, e)),
        }
    }
}

/// The providers a runtime resolves secrets with, by name. A reference to a provider that
/// isn't registered is an error: a missing keychain never silently falls back to another
/// provider.
#[derive(Clone, Default)]
pub struct SecretProviders(BTreeMap<String, Arc<dyn SecretProvider>>);

impl fmt::Debug for SecretProviders {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.0.keys()).finish()
    }
}

impl SecretProviders {
    /// No providers.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `provider` under its [`name`](SecretProvider::name), replacing one of the
    /// same name.
    pub fn with(mut self, provider: impl SecretProvider + 'static) -> Self {
        self.0
            .insert(provider.name().to_owned(), Arc::new(provider));
        self
    }

    /// Resolve `secret` with its provider.
    pub fn resolve(&self, secret: &SecretRef) -> Result<SecretValue, SecretError> {
        self.0
            .get(&secret.provider)
            .ok_or_else(|| SecretError::UnknownProvider(secret.provider.clone()))?
            .get(&secret.key)
    }
}
