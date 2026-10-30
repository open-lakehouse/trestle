//! Neon object storage (feature `neon`, native only).
//!
//! Neon branches carry S3-compatible object storage. Unlike the other protocols, its
//! issuer is a control plane, not a catalog, and it cannot downscope: a branch
//! credential is long-lived, never expires, and reaches every bucket of the branch
//! (and its descendants). This module is operator tooling for the branch's owner, not
//! a vending protocol: a vending server must never hand these keys to clients, and
//! presigns with them instead.
//!
//! The control plane (`https://console.neon.tech/api/v2`, authenticated with a Neon API
//! key) supplies the branch's endpoint, region and addressing style, and the
//! credential: revealed by its `token_id`, or created when explicitly allowed.
//!
//! ```no_run
//! # async fn run() -> olai_vended_store::Result<()> {
//! use object_store::ObjectStore;
//! use olai_vended_store::neon::NeonBuilder;
//!
//! let neon = NeonBuilder::new()
//!     .with_options([
//!         ("neon_api_key", "<api key>"),
//!         ("neon_project_id", "<project>"),
//!         ("neon_branch_id", "br-winter-pond-aptw82ef"),
//!         ("neon_token_id", "nak_live_..."),
//!     ])
//!     .build()?;
//! let bucket = neon.store("s3://my-bucket/landing").await?;
//! let listing = bucket.prefixed().list(None);
//! # Ok(()) }
//! ```

mod client;
mod config;

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use olai_storage_credentials::{
    AccessLevel, StorageCredentials, StorageLocation, StorageProvider, StorageScope, StoreHints,
    VendedCredential,
};
use tokio::sync::OnceCell;
use url::Url;

pub use self::client::{BranchStorage, CredentialSecret, DEFAULT_API_URL, NeonClient};
pub use self::config::NeonBuilder;
use crate::api::ApiError;
use crate::error::{Error, Result};
use crate::source::CredentialSource;
use crate::store::{ScopedStore, StoreOptions, build_store};

/// Where a branch's S3 credential comes from.
///
/// `Debug` redacts the secret key.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NeonCredentials {
    /// A key pair the caller already holds.
    Keys {
        /// The access key id (the credential's `token_id`).
        access_key_id: String,
        /// The secret access key. Secret.
        secret_access_key: String,
    },
    /// Reveal the secret of the existing credential `token_id`.
    Reveal {
        /// The credential id.
        token_id: String,
    },
    /// Create a credential with the scopes the access level needs. Each
    /// [`NeonStorage`] creates at most one, and never deletes it.
    Create {
        /// A label for the credential.
        name: Option<String>,
    },
}

impl fmt::Debug for NeonCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Keys { access_key_id, .. } => f
                .debug_struct("Keys")
                .field("access_key_id", access_key_id)
                .field("secret_access_key", &"<redacted>")
                .finish(),
            Self::Reveal { token_id } => f
                .debug_struct("Reveal")
                .field("token_id", token_id)
                .finish(),
            Self::Create { name } => f.debug_struct("Create").field("name", name).finish(),
        }
    }
}

/// Where a branch's endpoint, region and addressing come from.
#[derive(Debug, Clone)]
enum Discovery {
    /// Asked of the control plane.
    Api,
    /// Given by the caller.
    Given(BranchStorage),
}

/// Builds stores for the buckets of one Neon branch.
///
/// The branch's storage and its credential are resolved once, on the first store, and
/// shared by every store built afterwards. Cheap to clone; clones share them.
#[derive(Debug, Clone)]
pub struct NeonStorage {
    inner: Arc<Inner>,
    options: StoreOptions,
}

#[derive(Debug)]
struct Inner {
    client: Option<NeonClient>,
    project: String,
    branch: String,
    discovery: Discovery,
    credentials: NeonCredentials,
    access: AccessLevel,
    resolved: OnceCell<(StoreHints, StorageCredentials)>,
}

impl NeonStorage {
    /// Builds stores for `project`'s `branch` through `client`, with `credentials`,
    /// declared for `access`.
    ///
    /// The credential is not downscoped: `access` only chooses the scopes of a created
    /// credential (`storage:read`, plus `storage:write` for read-write) and labels the
    /// stores.
    pub fn new(
        client: NeonClient,
        project: impl Into<String>,
        branch: impl Into<String>,
        credentials: NeonCredentials,
        access: AccessLevel,
    ) -> Self {
        Self::with_parts(
            Some(client),
            project.into(),
            branch.into(),
            Discovery::Api,
            credentials,
            access,
        )
    }

    fn with_parts(
        client: Option<NeonClient>,
        project: String,
        branch: String,
        discovery: Discovery,
        credentials: NeonCredentials,
        access: AccessLevel,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                client,
                project,
                branch,
                discovery,
                credentials,
                access,
                resolved: OnceCell::new(),
            }),
            options: StoreOptions::default(),
        }
    }

    /// A builder configured from string keys; see [`NeonBuilder`].
    pub fn builder() -> NeonBuilder {
        NeonBuilder::new()
    }

    /// Configures the stores this builds.
    pub fn with_store_options(mut self, options: StoreOptions) -> Self {
        self.options = options;
        self
    }

    /// A source for `url`, an `s3://bucket[/prefix]` location on the branch.
    pub fn source(&self, url: &str) -> Result<NeonSource> {
        let parsed = Url::parse(url).map_err(|e| {
            Error::InvalidReference(format!("invalid Neon storage location `{url}`: {e}"))
        })?;
        let location = StorageLocation::parse(&parsed)?;
        if location.provider() != StorageProvider::Aws {
            return Err(Error::InvalidReference(format!(
                "`{url}`: Neon storage is addressed as `s3://bucket[/prefix]`"
            )));
        }
        Ok(NeonSource {
            storage: self.clone(),
            scope: StorageScope::new(location),
        })
    }

    /// Builds the store for `url`, an `s3://bucket[/prefix]` location on the branch.
    pub async fn store(&self, url: &str) -> Result<ScopedStore> {
        let source = self.source(url)?;
        let seed = source.fetch().await?;
        build_store(&seed, Arc::new(source), &self.options).await
    }

    /// The branch's store hints and credential, resolved once.
    async fn resolve(&self) -> Result<&(StoreHints, StorageCredentials)> {
        self.inner
            .resolved
            .get_or_try_init(|| async {
                let storage = self.branch_storage().await?;
                let hints = StoreHints {
                    endpoint: Some(storage.s3_endpoint),
                    region: Some(storage.region),
                    path_style: Some(storage.force_path_style),
                    ..StoreHints::default()
                };
                Ok((hints, self.credentials().await?))
            })
            .await
    }

    async fn branch_storage(&self) -> Result<BranchStorage> {
        let inner = &self.inner;
        let storage = match &inner.discovery {
            Discovery::Given(storage) => storage.clone(),
            Discovery::Api => self
                .client()?
                .branch_storage(&inner.project, &inner.branch)
                .await
                .map_err(|err| match ApiError::find(&err) {
                    Some(api) if api.status == Some(404) => Error::Unsupported {
                        location: self.branch_name(),
                        reason: format!("object storage is not available: {}", api.message),
                    },
                    _ => err,
                })?,
        };
        if !storage.enabled {
            return Err(Error::Unsupported {
                location: self.branch_name(),
                reason: "object storage is not enabled".to_owned(),
            });
        }
        Ok(storage)
    }

    async fn credentials(&self) -> Result<StorageCredentials> {
        let inner = &self.inner;
        let secret = match &inner.credentials {
            NeonCredentials::Keys {
                access_key_id,
                secret_access_key,
            } => CredentialSecret {
                token_id: access_key_id.clone(),
                s3_secret_access_key: secret_access_key.clone(),
            },
            NeonCredentials::Reveal { token_id } => self
                .client()?
                .reveal_credential(&inner.project, &inner.branch, token_id)
                .await
                .map_err(|err| match ApiError::find(&err) {
                    Some(api) if api.status == Some(409) => Error::Unsupported {
                        location: self.branch_name(),
                        reason: format!(
                            "credential `{token_id}` was issued before secrets could be \
                             revealed; rotate it (`neon credentials rotate {token_id}`) and \
                             reveal it again"
                        ),
                    },
                    _ => err,
                })?,
            NeonCredentials::Create { name } => {
                let scopes: &[&str] = match inner.access {
                    AccessLevel::Read => &["storage:read"],
                    AccessLevel::ReadWrite => &["storage:read", "storage:write"],
                };
                self.client()?
                    .create_credential(&inner.project, &inner.branch, scopes, name.as_deref())
                    .await?
            }
        };
        Ok(StorageCredentials::aws(
            secret.token_id,
            secret.s3_secret_access_key,
            None,
        ))
    }

    fn client(&self) -> Result<&NeonClient> {
        self.inner.client.as_ref().ok_or_else(|| {
            Error::Config(
                "a Neon API key (`neon_api_key`) is needed to discover the branch's storage \
                 or to reveal or create its credential"
                    .to_owned(),
            )
        })
    }

    fn branch_name(&self) -> String {
        format!(
            "neon branch `{}` of project `{}`",
            self.inner.branch, self.inner.project
        )
    }
}

/// The [`CredentialSource`] for a location on a Neon branch.
///
/// Branch credentials do not expire, so a store built from one never refreshes.
#[derive(Debug, Clone)]
pub struct NeonSource {
    storage: NeonStorage,
    scope: StorageScope,
}

impl NeonSource {
    /// The location this source serves.
    pub fn scope(&self) -> &StorageScope {
        &self.scope
    }
}

#[async_trait]
impl CredentialSource for NeonSource {
    async fn fetch(&self) -> Result<VendedCredential> {
        let (hints, credentials) = self.storage.resolve().await?;
        Ok(VendedCredential::new(
            credentials.clone(),
            self.scope.clone(),
            self.storage.inner.access,
        )
        .with_hints(hints.clone()))
    }
}

#[cfg(test)]
mod tests {
    use mockito::{Matcher, Server, ServerGuard};
    use serde_json::json;

    use super::*;

    const BRANCH: &str = "/projects/proj/branches/br-1";

    /// `getProjectBranchStorage`, as Neon's API reference documents it.
    async fn storage(server: &mut ServerGuard) -> mockito::Mock {
        server
            .mock("GET", format!("{BRANCH}/storage").as_str())
            .match_header("authorization", "Bearer napi-key")
            .with_body(
                json!({
                    "enabled": true,
                    "s3_endpoint": "https://br-1.storage.c-2.us-east-2.aws.neon.tech",
                    "region": "us-east-2",
                    "force_path_style": true,
                })
                .to_string(),
            )
            .expect(1)
            .create_async()
            .await
    }

    fn builder(server: &ServerGuard) -> NeonBuilder {
        NeonBuilder::new().with_options([
            ("neon_api_url", server.url()),
            ("neon_api_key", "napi-key".to_owned()),
            ("neon_project_id", "proj".to_owned()),
            ("neon_branch_id", "br-1".to_owned()),
        ])
    }

    fn keys(vended: &VendedCredential) -> (&str, &str) {
        match &vended.credentials {
            StorageCredentials::Aws {
                key_id, secret_key, ..
            } => (key_id, secret_key),
            other => panic!("expected AWS credentials, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn reveals_the_configured_credential_once() {
        let mut server = Server::new_async().await;
        let discovered = storage(&mut server).await;
        let reveal = server
            .mock(
                "POST",
                format!("{BRANCH}/credentials/nak_live_1/reveal").as_str(),
            )
            .with_body(
                json!({
                    "token_id": "nak_live_1",
                    "api_token": "napi_token",
                    "s3_secret_access_key": "nsk_live_1",
                })
                .to_string(),
            )
            .expect(1)
            .create_async()
            .await;
        let neon = builder(&server)
            .with_option("neon_token_id", "nak_live_1")
            .build()
            .unwrap();
        for bucket in ["s3://one/landing", "s3://two"] {
            let vended = neon.source(bucket).unwrap().fetch().await.unwrap();
            assert_eq!(keys(&vended), ("nak_live_1", "nsk_live_1"));
            assert_eq!(vended.expires_at, None);
            assert_eq!(
                vended.hints,
                StoreHints {
                    endpoint: Some("https://br-1.storage.c-2.us-east-2.aws.neon.tech".into()),
                    region: Some("us-east-2".into()),
                    path_style: Some(true),
                    ..StoreHints::default()
                }
            );
        }
        #[cfg(feature = "aws")]
        {
            let store = neon.store("s3://one/landing").await.unwrap();
            assert_eq!(store.prefix().as_ref(), "landing");
        }
        discovered.assert_async().await;
        reveal.assert_async().await;
    }

    #[tokio::test]
    async fn credentials_too_old_to_reveal_ask_for_rotation() {
        let mut server = Server::new_async().await;
        storage(&mut server).await;
        server
            .mock(
                "POST",
                format!("{BRANCH}/credentials/nak_old/reveal").as_str(),
            )
            .with_status(409)
            .with_body(json!({ "code": "", "message": "no recoverable secret" }).to_string())
            .create_async()
            .await;
        let err = builder(&server)
            .with_option("neon_token_id", "nak_old")
            .build()
            .unwrap()
            .source("s3://one")
            .unwrap()
            .fetch()
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }), "{err}");
        assert!(err.to_string().contains("rotate"), "{err}");
    }

    #[tokio::test]
    async fn branches_without_storage_are_unsupported() {
        let mut server = Server::new_async().await;
        server
            .mock("GET", format!("{BRANCH}/storage").as_str())
            .with_status(404)
            .with_body(
                json!({
                    "code": "",
                    "message": "object storage is not enabled",
                    "reason": "org_not_entitled",
                })
                .to_string(),
            )
            .create_async()
            .await;
        let err = builder(&server)
            .with_option("neon_token_id", "nak_live_1")
            .build()
            .unwrap()
            .source("s3://one")
            .unwrap()
            .fetch()
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }), "{err}");
        assert!(err.to_string().contains("org_not_entitled"), "{err}");
    }

    #[tokio::test]
    async fn credentials_are_created_only_when_opted_in() {
        let mut server = Server::new_async().await;
        let create = server
            .mock("POST", format!("{BRANCH}/credentials").as_str())
            .expect(0)
            .create_async()
            .await;
        let err = builder(&server).build().unwrap_err();
        assert!(matches!(err, Error::Config(_)), "{err}");
        assert!(err.to_string().contains("neon_create_credentials"), "{err}");
        create.assert_async().await;

        storage(&mut server).await;
        let create = server
            .mock("POST", format!("{BRANCH}/credentials").as_str())
            .match_body(Matcher::Json(json!({
                "scopes": ["storage:read", "storage:write"],
                "principal_type": "user",
                "name": "etl",
            })))
            .with_status(201)
            .with_body(
                json!({
                    "token_id": "nak_live_new",
                    "token_id_short": "nak_live_new",
                    "api_token": "napi_token",
                    "s3_secret_access_key": "nsk_live_new",
                    "scopes": ["storage:read", "storage:write"],
                    "branch_id": "br-1",
                    "created_at": "2026-10-30T00:00:00Z",
                })
                .to_string(),
            )
            .expect(1)
            .create_async()
            .await;
        let neon = builder(&server)
            .with_options([
                ("neon_create_credentials", "true"),
                ("neon_credential_name", "etl"),
                ("neon_operation", "read_write"),
            ])
            .build()
            .unwrap();
        for _ in 0..2 {
            let vended = neon.source("s3://one").unwrap().fetch().await.unwrap();
            assert_eq!(keys(&vended), ("nak_live_new", "nsk_live_new"));
            assert_eq!(vended.access, AccessLevel::ReadWrite);
        }
        create.assert_async().await;
    }

    #[tokio::test]
    async fn credentials_are_created_without_retries() {
        let mut server = Server::new_async().await;
        storage(&mut server).await;
        // Neon may have created the credential before the gateway failed: a retry would
        // create another.
        let create = server
            .mock("POST", format!("{BRANCH}/credentials").as_str())
            .with_status(502)
            .expect(1)
            .create_async()
            .await;
        let err = builder(&server)
            .with_option("neon_create_credentials", "true")
            .build()
            .unwrap()
            .source("s3://one")
            .unwrap()
            .fetch()
            .await
            .unwrap_err();
        assert_eq!(
            crate::ApiError::find(&err).and_then(|e| e.status),
            Some(502),
            "{err}"
        );
        create.assert_async().await;
    }

    #[tokio::test]
    async fn given_keys_and_storage_need_no_api_key() {
        let neon = NeonBuilder::new()
            .with_options([
                ("neon_project_id", "proj"),
                ("neon_branch_id", "br-1"),
                ("neon_access_key_id", "nak_live_1"),
                ("neon_secret_access_key", "nsk_live_1"),
                (
                    "neon_s3_endpoint",
                    "https://br-1.storage.c-2.us-east-2.aws.neon.tech",
                ),
                ("neon_region", "us-east-2"),
            ])
            .build()
            .unwrap();
        let vended = neon.source("s3://one").unwrap().fetch().await.unwrap();
        assert_eq!(keys(&vended), ("nak_live_1", "nsk_live_1"));
        assert_eq!(vended.hints.path_style, Some(true));
        assert!(!format!("{neon:?}").contains("nsk_live_1"));
        assert!(neon.source("gs://bucket").is_err());
    }
}
