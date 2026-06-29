//! The typed connection a resource demand negotiates ([`Connection`]).
//!
//! When a module demands a resource (a relational database, an object-store bucket, …),
//! the only thing that has to be negotiated is *how to connect*: a URL/endpoint plus a
//! credential. The credential shape is closed per flavour — an S3 store needs an access
//! key id and secret (plus region); an Azure Blob store needs a connection string; a
//! relational store folds auth into its URL. There are only a handful, so they are a
//! **typed enum** the compiler enforces, not an open string→value map.
//!
//! This is the deliberate trade the crate makes: a new resource *flavour* (a message
//! queue, GCS, MySQL) is a typed addition *here*, not free-form catalog data. In return,
//! a provider cannot declare an incomplete connection (the variant's fields are
//! mandatory), and a consumer binds to typed [`ConnectionField`]s rather than re-spelling
//! coordinate names by hand — so the runtime "does this provider render every required
//! coordinate?" check the old open model needed is gone.
//!
//! # Templates and resolution
//!
//! A provider declares a [`ConnectionTemplate`]: a [`Connection`] whose string fields may
//! contain the `{name}` placeholder. The planner [`resolve`](ConnectionTemplate::resolve)s
//! it per demand, substituting `{name}` with the demanded resource name. Compose-style
//! `${VAR}` refs are left untouched (compose resolves them at run time), exactly as before.

use serde::{Deserialize, Serialize};

/// A fully-resolved, typed connection to a provisioned resource.
///
/// One variant per resource *flavour*. Every field is a final string value — the planner
/// has already substituted `{name}` — though values may still carry compose `${VAR}` refs
/// that compose resolves at run time.
///
/// `#[non_exhaustive]`: a future flavour (a message queue, GCS, …) can be added without
/// breaking downstream `match`es.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
#[non_exhaustive]
pub enum Connection {
    /// An S3/Blob-style object store. Addressing (`uri`/`bucket`/`endpoint`) is
    /// flavour-independent; the [`credential`](Connection::ObjectStore::credential) carries
    /// the flavour-specific auth.
    ObjectStore {
        /// Client-addressable URI for the resource (`s3://{name}`, `wasbs://…`).
        uri: String,
        /// The bucket/container name.
        bucket: String,
        /// The in-network service endpoint (`http://seaweedfs:8333`).
        endpoint: String,
        /// The credential needed to authenticate to the store.
        credential: ObjectStoreCredential,
    },
    /// A relational database. The credential is embedded in the connection URL (matching
    /// how Postgres-style clients consume it), so there is no separate credential field.
    RelationalDb {
        /// The full connection URL, e.g. `postgresql://user:pass@db:5432/{name}`.
        url: String,
    },
}

/// The credential for an [`ObjectStore`](Connection::ObjectStore) — closed per flavour.
///
/// `#[non_exhaustive]` so a future object-store flavour (GCS, …) is not a breaking change
/// for downstream `match`es.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "flavour")]
#[non_exhaustive]
pub enum ObjectStoreCredential {
    /// S3-style static credentials.
    S3 {
        /// `AWS_ACCESS_KEY_ID`.
        access_key_id: String,
        /// `AWS_SECRET_ACCESS_KEY`.
        secret_access_key: String,
        /// The default region.
        region: String,
    },
    /// An Azure Blob connection string (carries account name + key + endpoint).
    AzureBlob {
        /// The full `AZURE_STORAGE_CONNECTION_STRING` value.
        connection_string: String,
    },
}

/// A provider's connection *template*: a [`Connection`] whose string fields may contain
/// the `{name}` placeholder, substituted per demand by the planner.
///
/// Stored on a provider module's
/// [`Provides::resource_kinds`](crate::Provides::resource_kinds), keyed by the role string.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConnectionTemplate(pub Connection);

impl ConnectionTemplate {
    /// Resolve this template to a concrete [`Connection`] for the resource named `name`,
    /// substituting every `{name}` placeholder. `${VAR}` compose refs are left untouched.
    pub fn resolve(&self, name: &str) -> Connection {
        let sub = |s: &str| s.replace("{name}", name);
        match &self.0 {
            Connection::ObjectStore {
                uri,
                bucket,
                endpoint,
                credential,
            } => Connection::ObjectStore {
                uri: sub(uri),
                bucket: sub(bucket),
                endpoint: sub(endpoint),
                credential: credential.resolve(name),
            },
            Connection::RelationalDb { url } => Connection::RelationalDb { url: sub(url) },
        }
    }
}

impl ObjectStoreCredential {
    /// `AWS_ACCESS_KEY_ID` — the env var an S3 credential's access key id lands under.
    pub const AWS_ACCESS_KEY_ID: &'static str = "AWS_ACCESS_KEY_ID";
    /// `AWS_SECRET_ACCESS_KEY` — the env var an S3 credential's secret lands under.
    pub const AWS_SECRET_ACCESS_KEY: &'static str = "AWS_SECRET_ACCESS_KEY";
    /// `AWS_DEFAULT_REGION` — the env var an S3 credential's region lands under.
    pub const AWS_DEFAULT_REGION: &'static str = "AWS_DEFAULT_REGION";
    /// `AZURE_STORAGE_CONNECTION_STRING` — the env var an Azure credential lands under.
    pub const AZURE_STORAGE_CONNECTION_STRING: &'static str = "AZURE_STORAGE_CONNECTION_STRING";

    /// The conventional `(env-var, value)` pairs an SDK reads to authenticate to a store of
    /// this flavour — `AWS_*` for [`S3`](ObjectStoreCredential::S3),
    /// `AZURE_STORAGE_CONNECTION_STRING` for [`AzureBlob`](ObjectStoreCredential::AzureBlob).
    ///
    /// The planner folds these into `.env` for the chosen object-store provider, so the
    /// typed credential is the single source for both the values a [`Template`] fragment
    /// reads and the conventional SDK env vars — no hand-listing, no drift.
    ///
    /// [`Template`]: crate::RenderSpec::Template
    pub fn standard_env(&self) -> Vec<(&'static str, String)> {
        match self {
            ObjectStoreCredential::S3 {
                access_key_id,
                secret_access_key,
                region,
            } => vec![
                (Self::AWS_ACCESS_KEY_ID, access_key_id.clone()),
                (Self::AWS_SECRET_ACCESS_KEY, secret_access_key.clone()),
                (Self::AWS_DEFAULT_REGION, region.clone()),
            ],
            ObjectStoreCredential::AzureBlob { connection_string } => vec![(
                Self::AZURE_STORAGE_CONNECTION_STRING,
                connection_string.clone(),
            )],
        }
    }

    /// Resolve `{name}` in every field. Credential values rarely template on `{name}`, but
    /// resolving uniformly keeps [`ConnectionTemplate::resolve`] total.
    fn resolve(&self, name: &str) -> ObjectStoreCredential {
        let sub = |s: &str| s.replace("{name}", name);
        match self {
            ObjectStoreCredential::S3 {
                access_key_id,
                secret_access_key,
                region,
            } => ObjectStoreCredential::S3 {
                access_key_id: sub(access_key_id),
                secret_access_key: sub(secret_access_key),
                region: sub(region),
            },
            ObjectStoreCredential::AzureBlob { connection_string } => {
                ObjectStoreCredential::AzureBlob {
                    connection_string: sub(connection_string),
                }
            }
        }
    }
}

/// One typed part of a resolved [`Connection`] a demand can bind to an environment
/// variable. The typed replacement for the old stringly coordinate name.
///
/// Not every field is present on every variant — [`Connection::field`] returns `None` for a
/// field the variant lacks (e.g. [`Url`](ConnectionField::Url) on an object store), which
/// the planner surfaces as [`PlanError::UnboundConnectionField`](crate::PlanError::UnboundConnectionField).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionField {
    /// [`Connection::ObjectStore::uri`].
    Uri,
    /// [`Connection::ObjectStore::bucket`].
    Bucket,
    /// [`Connection::ObjectStore::endpoint`].
    Endpoint,
    /// [`ObjectStoreCredential::S3::access_key_id`].
    AccessKeyId,
    /// [`ObjectStoreCredential::S3::secret_access_key`].
    SecretAccessKey,
    /// [`ObjectStoreCredential::S3::region`].
    Region,
    /// [`ObjectStoreCredential::AzureBlob::connection_string`].
    ConnectionString,
    /// [`Connection::RelationalDb::url`].
    Url,
}

impl Connection {
    /// The value of one typed [`ConnectionField`], if this connection variant has it.
    ///
    /// Returns `None` for a field absent from the variant (e.g.
    /// [`Url`](ConnectionField::Url) on an object store, or an S3 credential field on an
    /// Azure-backed store).
    pub fn field(&self, field: ConnectionField) -> Option<&str> {
        use ConnectionField as F;
        match (self, field) {
            (Connection::ObjectStore { uri, .. }, F::Uri) => Some(uri),
            (Connection::ObjectStore { bucket, .. }, F::Bucket) => Some(bucket),
            (Connection::ObjectStore { endpoint, .. }, F::Endpoint) => Some(endpoint),
            (Connection::ObjectStore { credential, .. }, _) => credential.field(field),
            (Connection::RelationalDb { url }, F::Url) => Some(url),
            _ => None,
        }
    }

    /// The conventional `(env-var, value)` pairs a provider of this connection contributes
    /// to `.env` so an SDK can authenticate — the object store's
    /// [`ObjectStoreCredential::standard_env`]. A [`RelationalDb`](Connection::RelationalDb)
    /// has none: its credential is embedded in the URL a consumer binds, not a stack-wide
    /// env var.
    pub fn standard_env(&self) -> Vec<(&'static str, String)> {
        match self {
            Connection::ObjectStore { credential, .. } => credential.standard_env(),
            Connection::RelationalDb { .. } => Vec::new(),
        }
    }
}

impl ObjectStoreCredential {
    /// The value of a credential field, if this credential flavour has it.
    fn field(&self, field: ConnectionField) -> Option<&str> {
        use ConnectionField as F;
        match (self, field) {
            (ObjectStoreCredential::S3 { access_key_id, .. }, F::AccessKeyId) => {
                Some(access_key_id)
            }
            (
                ObjectStoreCredential::S3 {
                    secret_access_key, ..
                },
                F::SecretAccessKey,
            ) => Some(secret_access_key),
            (ObjectStoreCredential::S3 { region, .. }, F::Region) => Some(region),
            (ObjectStoreCredential::AzureBlob { connection_string }, F::ConnectionString) => {
                Some(connection_string)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relational_template_substitutes_name_and_keeps_compose_refs() {
        let t = ConnectionTemplate(Connection::RelationalDb {
            url: "postgresql://${POSTGRES_USER:-postgres}@db:5432/{name}".into(),
        });
        let c = t.resolve("appdb");
        assert_eq!(
            c.field(ConnectionField::Url),
            Some("postgresql://${POSTGRES_USER:-postgres}@db:5432/appdb"),
            "{{name}} is substituted; ${{VAR}} is left for compose"
        );
    }

    #[test]
    fn object_store_template_resolves_every_field() {
        let t = ConnectionTemplate(Connection::ObjectStore {
            uri: "s3://{name}".into(),
            bucket: "{name}".into(),
            endpoint: "http://seaweedfs:8333".into(),
            credential: ObjectStoreCredential::S3 {
                access_key_id: "seaweedfs".into(),
                secret_access_key: "seaweedfs".into(),
                region: "us-east-1".into(),
            },
        });
        let c = t.resolve("artifacts");
        assert_eq!(c.field(ConnectionField::Uri), Some("s3://artifacts"));
        assert_eq!(c.field(ConnectionField::Bucket), Some("artifacts"));
        assert_eq!(
            c.field(ConnectionField::Endpoint),
            Some("http://seaweedfs:8333")
        );
        assert_eq!(c.field(ConnectionField::AccessKeyId), Some("seaweedfs"));
        assert_eq!(c.field(ConnectionField::Region), Some("us-east-1"));
        // A field the variant lacks resolves to None.
        assert_eq!(c.field(ConnectionField::Url), None);
        assert_eq!(c.field(ConnectionField::ConnectionString), None);
    }

    #[test]
    fn azure_credential_exposes_connection_string_only() {
        let c = Connection::ObjectStore {
            uri: "wasbs://data@acct".into(),
            bucket: "data".into(),
            endpoint: "http://azurite:10000".into(),
            credential: ObjectStoreCredential::AzureBlob {
                connection_string: "Conn=string".into(),
            },
        };
        assert_eq!(
            c.field(ConnectionField::ConnectionString),
            Some("Conn=string")
        );
        // S3-only fields are absent under an Azure credential.
        assert_eq!(c.field(ConnectionField::AccessKeyId), None);
    }

    #[test]
    fn standard_env_derives_the_conventional_sdk_vars() {
        let s3 = Connection::ObjectStore {
            uri: "s3://b".into(),
            bucket: "b".into(),
            endpoint: "http://s:1".into(),
            credential: ObjectStoreCredential::S3 {
                access_key_id: "ak".into(),
                secret_access_key: "sk".into(),
                region: "us-east-1".into(),
            },
        };
        assert_eq!(
            s3.standard_env(),
            vec![
                ("AWS_ACCESS_KEY_ID", "ak".to_string()),
                ("AWS_SECRET_ACCESS_KEY", "sk".to_string()),
                ("AWS_DEFAULT_REGION", "us-east-1".to_string()),
            ]
        );

        let azure = Connection::ObjectStore {
            uri: "wasbs://b@a".into(),
            bucket: "b".into(),
            endpoint: "http://a:1".into(),
            credential: ObjectStoreCredential::AzureBlob {
                connection_string: "Conn=x".into(),
            },
        };
        assert_eq!(
            azure.standard_env(),
            vec![("AZURE_STORAGE_CONNECTION_STRING", "Conn=x".to_string())]
        );

        // A relational connection contributes no provider-side env vars (its credential is
        // embedded in the URL a consumer binds).
        let db = Connection::RelationalDb {
            url: "postgresql://db/x".into(),
        };
        assert!(db.standard_env().is_empty());
    }
}
