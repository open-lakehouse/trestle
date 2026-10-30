//! The Unity Catalog Delta API (feature `delta-api`): table credentials for
//! catalog-managed tables.
//!
//! The table is loaded once for its location
//! (`GET delta/v1/catalogs/{c}/schemas/{s}/tables/{t}`), and every vend asks
//! `GET .../tables/{t}/credentials?operation=...`. A response may carry several
//! `storage-credentials` with different prefixes; the source keeps the most specific one
//! that covers the table at the requested access, and never one for unrelated storage.

use async_trait::async_trait;
use olai_storage_credentials::location::longest_match;
use olai_storage_credentials::wire::VendContext;
use olai_storage_credentials::wire::delta_api::{CredentialsResponse, Operation};
use olai_storage_credentials::{
    AccessLevel, StorageCredentials, StorageLocation, StorageProvider, StorageScope, StoreHints,
    VendedCredential,
};
use serde::Deserialize;
use url::Url;

use super::UnityClient;
use crate::error::{Error, Result};
use crate::source::CredentialSource;

/// The fields of a Delta API table response vending needs.
#[derive(Debug, Clone, Deserialize)]
struct LoadTable {
    metadata: Metadata,
}

#[derive(Debug, Clone, Deserialize)]
struct Metadata {
    location: String,
}

/// Vends Delta API credentials for one table at one access level.
#[derive(Debug, Clone)]
pub struct DeltaApiSource {
    client: UnityClient,
    catalog: String,
    schema: String,
    table: String,
    scope: StorageScope,
    hints: StoreHints,
    access: AccessLevel,
}

impl DeltaApiSource {
    /// Loads `catalog.schema.table` for its location and binds a source to it.
    pub async fn load(
        client: UnityClient,
        catalog: &str,
        schema: &str,
        table: &str,
        access: AccessLevel,
    ) -> Result<Self> {
        let loaded: LoadTable = client
            .get_segments(&table_path(catalog, schema, table), &[])
            .await?;
        let location = &loaded.metadata.location;
        let url = Url::parse(location).map_err(|e| {
            Error::InvalidReference(format!(
                "the Delta API returned an invalid table location `{location}`: {e}"
            ))
        })?;
        let (location, hints) = StorageLocation::parse_with_hints(&url)?;
        Ok(Self {
            client,
            catalog: catalog.to_owned(),
            schema: schema.to_owned(),
            table: table.to_owned(),
            scope: StorageScope::new(location),
            hints,
            access,
        })
    }

    /// The table's location.
    pub fn scope(&self) -> &StorageScope {
        &self.scope
    }

    /// The same table at another access level.
    pub fn with_access(mut self, access: AccessLevel) -> Self {
        self.access = access;
        self
    }
}

/// The path segments of a Delta API table.
fn table_path<'a>(catalog: &'a str, schema: &'a str, table: &'a str) -> Vec<&'a str> {
    vec![
        "delta", "v1", "catalogs", catalog, "schemas", schema, "tables", table,
    ]
}

/// The credential to use for `ctx`: of those whose prefix covers the requested scope
/// at the requested access, the one with the most specific prefix, clamped to the scope.
fn select(response: &CredentialsResponse, ctx: &VendContext) -> Result<VendedCredential> {
    let mut candidates = Vec::new();
    for credential in &response.storage_credentials {
        let prefix: StorageScope = credential.prefix.parse()?;
        if AccessLevel::from(credential.operation).satisfies(ctx.access) {
            candidates.push((prefix, credential));
        }
    }
    let chosen = longest_match(
        candidates.iter().map(|(prefix, c)| (prefix, *c)),
        ctx.scope.location(),
    )
    .ok_or_else(|| {
        Error::Credentials(olai_storage_credentials::Error::InvalidCredential(format!(
            "no credential covers {} with {:?} access",
            ctx.scope, ctx.access
        )))
    })?;
    // A prefix covering the scope always relates to it, so this is `Some`.
    chosen.to_vended(ctx)?.ok_or_else(|| {
        Error::Credentials(olai_storage_credentials::Error::InvalidCredential(format!(
            "credential for `{}` does not relate to {}",
            chosen.prefix, ctx.scope
        )))
    })
}

#[async_trait]
impl CredentialSource for DeltaApiSource {
    async fn fetch(&self) -> Result<VendedCredential> {
        if self.scope.location().provider() == StorageProvider::Local {
            return Ok(VendedCredential::new(
                StorageCredentials::None,
                self.scope.clone(),
                self.access,
            ));
        }
        let mut path = table_path(&self.catalog, &self.schema, &self.table);
        path.push("credentials");
        let operation = match Operation::from(self.access) {
            Operation::Read => "READ",
            Operation::ReadWrite => "READ_WRITE",
        };
        let response: CredentialsResponse = self
            .client
            .get_segments(&path, &[("operation", operation)])
            .await?;
        let mut vended = select(
            &response,
            &VendContext::new(self.scope.clone(), self.access),
        )?;
        if vended.hints.is_empty() {
            vended.hints = self.hints.clone();
        }
        Ok(vended)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use olai_storage_credentials::wire::delta_api::StorageCredential;

    use super::*;

    fn credential(prefix: &str, operation: Operation, key: &str) -> StorageCredential {
        StorageCredential {
            prefix: prefix.to_owned(),
            operation,
            config: BTreeMap::from([
                ("s3.access-key-id".to_owned(), key.to_owned()),
                ("s3.secret-access-key".to_owned(), "secret".to_owned()),
            ]),
            expiration_time_ms: None,
        }
    }

    fn ctx(scope: &str, access: AccessLevel) -> VendContext {
        VendContext::new(scope.parse().unwrap(), access)
    }

    fn key(vended: &VendedCredential) -> &str {
        match &vended.credentials {
            StorageCredentials::Aws { key_id, .. } => key_id,
            other => panic!("expected AWS credentials, got {other:?}"),
        }
    }

    /// A catalog-managed table through `open`, falling back to read when write is
    /// denied, and keeping the most specific of the vended prefixes.
    #[cfg(feature = "aws")]
    #[tokio::test]
    async fn open_vends_through_the_delta_api() {
        use mockito::Matcher;
        use serde_json::json;

        let mut server = mockito::Server::new_async().await;
        let table = "/api/2.1/unity-catalog/delta/v1/catalogs/main/schemas/default/tables/orders";
        let load = server
            .mock("GET", table)
            .with_body(json!({ "metadata": { "location": "s3://bucket/wh/orders" } }).to_string())
            .expect(1)
            .create_async()
            .await;
        let write = server
            .mock("GET", format!("{table}/credentials").as_str())
            .match_query(Matcher::UrlEncoded("operation".into(), "READ_WRITE".into()))
            .with_status(403)
            .with_body(json!({ "error_code": "PERMISSION_DENIED" }).to_string())
            .create_async()
            .await;
        let read = server
            .mock("GET", format!("{table}/credentials").as_str())
            .match_query(Matcher::UrlEncoded("operation".into(), "READ".into()))
            .with_body(
                json!({ "storage-credentials": [
                    { "prefix": "s3://bucket/", "operation": "READ",
                      "config": { "s3.access-key-id": "bucket", "s3.secret-access-key": "s" } },
                    { "prefix": "s3://bucket/wh/orders/", "operation": "READ",
                      "config": { "s3.access-key-id": "table", "s3.secret-access-key": "s" },
                      "expiration-time-ms": 4_102_444_800_000_i64 },
                ]})
                .to_string(),
            )
            .create_async()
            .await;
        let catalog = crate::unity::UnityBuilder::new()
            .with_options([
                (
                    "unity_catalog_url",
                    format!("{}/api/2.1/unity-catalog", server.url()),
                ),
                ("unity_token", "t".to_owned()),
                ("unity_delta_api", "true".to_owned()),
                ("aws_region", "us-east-1".to_owned()),
            ])
            .build()
            .unwrap();
        let store = catalog.open("uc://main.default.orders").await.unwrap();
        assert_eq!(store.prefix().as_ref(), "wh/orders");
        load.assert_async().await;
        write.assert_async().await;
        read.assert_async().await;

        let source = catalog
            .delta_table_source("main", "default", "orders", AccessLevel::Read)
            .await
            .unwrap();
        let vended = source.fetch().await.unwrap();
        assert_eq!(key(&vended), "table");
        assert!(vended.expires_at.is_some());
    }

    #[test]
    fn the_most_specific_covering_prefix_wins() {
        let response = CredentialsResponse {
            storage_credentials: vec![
                credential("s3://bucket/", Operation::ReadWrite, "bucket"),
                credential("s3://bucket/warehouse/", Operation::ReadWrite, "warehouse"),
                // A sibling sharing the string prefix covers nothing here.
                credential(
                    "s3://bucket/warehouse/tab2/",
                    Operation::ReadWrite,
                    "sibling",
                ),
            ],
        };
        let vended = select(
            &response,
            &ctx("s3://bucket/warehouse/tab", AccessLevel::Read),
        )
        .unwrap();
        assert_eq!(key(&vended), "warehouse");
        // Clamped to the table, never wider.
        assert_eq!(vended.scope.to_string(), "s3://bucket/warehouse/tab");
    }

    #[test]
    fn read_only_credentials_do_not_serve_writes() {
        let response = CredentialsResponse {
            storage_credentials: vec![
                credential("s3://bucket/warehouse/tab/", Operation::Read, "read"),
                credential("s3://bucket/", Operation::ReadWrite, "write"),
            ],
        };
        let vended = select(
            &response,
            &ctx("s3://bucket/warehouse/tab", AccessLevel::ReadWrite),
        )
        .unwrap();
        assert_eq!(key(&vended), "write");
    }

    #[test]
    fn unrelated_storage_is_never_used() {
        let response = CredentialsResponse {
            storage_credentials: vec![credential("s3://other/", Operation::ReadWrite, "other")],
        };
        assert!(select(&response, &ctx("s3://bucket/tab", AccessLevel::Read)).is_err());
        let narrower = CredentialsResponse {
            storage_credentials: vec![credential(
                "s3://bucket/tab/part/",
                Operation::ReadWrite,
                "narrow",
            )],
        };
        assert!(select(&narrower, &ctx("s3://bucket/tab", AccessLevel::Read)).is_err());
    }
}
