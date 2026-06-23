use std::sync::Arc;

use bytes::Bytes;
use uuid::Uuid;

use crate::Result;

/// A trait for managing secrets.
///
/// All sensitive data that needs to be stored in the system should be stored as a secret.
///
/// The secret manager is responsible for fetching the secret value from the secret store.
/// The secret store can be a key-value store, a secret manager service, or any other secret store.
#[async_trait::async_trait]
pub trait SecretManager: Send + Sync + 'static {
    /// Returns the current value of the named secret along with its version.
    ///
    /// Secrets are identified by a unique name. The returned [`Uuid`] is the
    /// version identifier of the value just read; it changes every time the secret
    /// is updated, so a caller can detect whether the value has changed since it
    /// was last fetched (and pass it to [`get_secret_version`] to re-read that
    /// exact value later).
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`](crate::Error::NotFound) if no secret with `secret_name` exists.
    ///
    /// [`get_secret_version`]: SecretManager::get_secret_version
    async fn get_secret(&self, secret_name: &str) -> Result<(Uuid, Bytes)>;

    /// Returns the value of a specific version of the named secret.
    ///
    /// `version` is a version identifier previously returned by [`get_secret`] or
    /// a mutating call, letting a caller re-read an exact historical value rather
    /// than the current one.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`](crate::Error::NotFound) if no secret with `secret_name` and `version`
    /// exists.
    ///
    /// [`get_secret`]: SecretManager::get_secret
    async fn get_secret_version(&self, secret_name: &str, version: Uuid) -> Result<Bytes>;

    /// Creates a new secret with the given name and value.
    ///
    /// Returns the version identifier of the newly stored value.
    ///
    /// # Errors
    ///
    /// - [`Error::AlreadyExists`](crate::Error::AlreadyExists) if a secret with `secret_name` already exists.
    /// - [`Error::InvalidArgument`](crate::Error::InvalidArgument) if `secret_value` is invalid.
    async fn create_secret(&self, secret_name: &str, secret_value: Bytes) -> Result<Uuid>;

    /// Updates the value of an existing secret.
    ///
    /// Returns the version identifier of the new value; the previous version
    /// remains addressable through [`get_secret_version`].
    ///
    /// # Errors
    ///
    /// - [`Error::NotFound`](crate::Error::NotFound) if no secret with `secret_name` exists.
    /// - [`Error::InvalidArgument`](crate::Error::InvalidArgument) if `secret_value` is invalid.
    ///
    /// [`get_secret_version`]: SecretManager::get_secret_version
    async fn update_secret(&self, secret_name: &str, secret_value: Bytes) -> Result<Uuid>;

    /// Deletes the named secret and all of its versions.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`](crate::Error::NotFound) if no secret with `secret_name` exists.
    async fn delete_secret(&self, secret_name: &str) -> Result<()>;
}

/// Auxiliary trait for implementing [`SecretManager`] for structs that contain a [`SecretManager`].
///
/// Implement this to forward all [`SecretManager`] calls to a wrapped manager; a
/// blanket impl then provides [`SecretManager`] for the wrapper for free.
pub trait ProvidesSecretManager: Send + Sync + 'static {
    /// Returns the wrapped secret manager that calls are forwarded to.
    fn secret_manager(&self) -> &dyn SecretManager;
}

#[async_trait::async_trait]
impl<T: SecretManager> SecretManager for Arc<T> {
    async fn get_secret(&self, secret_name: &str) -> Result<(Uuid, Bytes)> {
        T::get_secret(self, secret_name).await
    }

    async fn get_secret_version(&self, secret_name: &str, version: Uuid) -> Result<Bytes> {
        T::get_secret_version(self, secret_name, version).await
    }

    async fn create_secret(&self, secret_name: &str, secret_value: Bytes) -> Result<Uuid> {
        T::create_secret(self, secret_name, secret_value).await
    }

    async fn update_secret(&self, secret_name: &str, secret_value: Bytes) -> Result<Uuid> {
        T::update_secret(self, secret_name, secret_value).await
    }

    async fn delete_secret(&self, secret_name: &str) -> Result<()> {
        T::delete_secret(self, secret_name).await
    }
}

#[async_trait::async_trait]
impl<T: ProvidesSecretManager> SecretManager for T {
    async fn get_secret(&self, secret_name: &str) -> Result<(Uuid, Bytes)> {
        self.secret_manager().get_secret(secret_name).await
    }

    async fn get_secret_version(&self, secret_name: &str, version: Uuid) -> Result<Bytes> {
        self.secret_manager()
            .get_secret_version(secret_name, version)
            .await
    }

    async fn create_secret(&self, secret_name: &str, secret_value: Bytes) -> Result<Uuid> {
        self.secret_manager()
            .create_secret(secret_name, secret_value)
            .await
    }

    async fn update_secret(&self, secret_name: &str, secret_value: Bytes) -> Result<Uuid> {
        self.secret_manager()
            .update_secret(secret_name, secret_value)
            .await
    }

    async fn delete_secret(&self, secret_name: &str) -> Result<()> {
        self.secret_manager().delete_secret(secret_name).await
    }
}
