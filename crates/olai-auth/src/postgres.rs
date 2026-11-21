use async_trait::async_trait;
use sqlx::PgPool;

use crate::api_key::{ApiKeyRecord, ApiKeyStore};
use crate::error::BoxError;
use crate::user_status::{UserStatus, UserStatusStore};

/// The schema the Postgres stores use unless configured otherwise.
pub(crate) const DEFAULT_SCHEMA: &str = "authz";

/// `name` quoted as a Postgres identifier.
pub(crate) fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// An [`ApiKeyStore`] that reads keys from Postgres.
///
/// It expects two tables in one schema (`authz` by default, see
/// [`with_schema`](Self::with_schema)), as the `authz` migrations
/// ([`migrate`](crate::migrate)) create them:
///
/// ```sql
/// CREATE TABLE authz.service_accounts (
///     id          uuid        PRIMARY KEY,
///     name        text        NOT NULL UNIQUE,
///     disabled_at timestamptz
/// );
/// CREATE TABLE authz.api_keys (
///     id          text        PRIMARY KEY,           -- the key's 16 hex characters
///     account_id  uuid        NOT NULL REFERENCES authz.service_accounts (id),
///     sha256      bytea       NOT NULL,              -- SHA-256 of the whole key string
///     scopes      text[]      NOT NULL DEFAULT '{}',
///     expires_at  timestamptz,
///     revoked_at  timestamptz
/// );
/// ```
///
/// A key is live while it is not revoked, not expired, and its account is not disabled.
/// The role the pool connects as needs only `SELECT` on both tables. Other columns
/// are ignored.
///
/// sqlx is linked with only its `postgres` feature: the host enables a runtime and
/// TLS backend for its own pool.
#[derive(Debug, Clone)]
pub struct PgApiKeyStore {
    pool: PgPool,
    query: String,
}

impl PgApiKeyStore {
    /// A store reading the `authz` schema through `pool`.
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            query: lookup_query(DEFAULT_SCHEMA),
        }
    }

    /// Reads the tables from `schema` instead of `authz`.
    pub fn with_schema(mut self, schema: &str) -> Self {
        self.query = lookup_query(schema);
        self
    }
}

/// The lookup query against `schema`, quoted as an identifier.
fn lookup_query(schema: &str) -> String {
    let schema = quote_ident(schema);
    format!(
        "SELECT k.sha256, a.id::text, a.name, k.scopes \
         FROM {schema}.api_keys k JOIN {schema}.service_accounts a ON a.id = k.account_id \
         WHERE k.id = $1 AND k.revoked_at IS NULL \
           AND (k.expires_at IS NULL OR k.expires_at > now()) \
           AND a.disabled_at IS NULL"
    )
}

#[async_trait]
impl ApiKeyStore for PgApiKeyStore {
    async fn lookup(&self, id: &str) -> Result<Option<ApiKeyRecord>, BoxError> {
        let row: Option<(Vec<u8>, String, String, Vec<String>)> = sqlx::query_as(&self.query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        let Some((sha256, account_id, account_name, scopes)) = row else {
            return Ok(None);
        };
        let sha256 = <[u8; 32]>::try_from(sha256)
            .map_err(|bytes| format!("API key {id} has a {}-byte digest", bytes.len()))?;
        Ok(Some(ApiKeyRecord {
            sha256,
            account_id,
            account_name,
            scopes,
        }))
    }
}

/// The schema Neon Auth keeps its users in.
pub(crate) const DEFAULT_NEON_AUTH_SCHEMA: &str = "neon_auth";

/// A [`UserStatusStore`] that reads Neon Auth's users from Postgres.
///
/// Neon Auth keeps them in `neon_auth."user"` (see [`with_schema`](Self::with_schema)
/// for another schema). Of its columns, this reads only these:
///
/// ```sql
/// CREATE TABLE neon_auth."user" (
///     id           uuid PRIMARY KEY,  -- the token's `sub`
///     role         text,              -- `admin`, or a comma-separated list
///     banned       boolean,
///     "banExpires" timestamptz,
///     ...
/// );
/// ```
///
/// A user is an admin when their `role` is, or lists, `admin` (see
/// [`UserStatus::from_role`]), and banned while `banned` is set and `banExpires` is
/// unset or in the future. A user who is not in the table, including a `sub` that is
/// not a user id at all, is unknown (`None`), so their token is rejected. A missing
/// table is an error. The issuer is not consulted: one service trusts one Neon Auth
/// instance, whose users are all in this table.
///
/// The role the pool connects as needs only `SELECT (id, role, banned, "banExpires")`.
#[derive(Debug, Clone)]
pub struct PgNeonUserStatus {
    pool: PgPool,
    query: String,
}

impl PgNeonUserStatus {
    /// A store reading `neon_auth."user"` through `pool`.
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            query: user_status_query(DEFAULT_NEON_AUTH_SCHEMA),
        }
    }

    /// Reads the `user` table from `schema` instead of `neon_auth`.
    pub fn with_schema(mut self, schema: &str) -> Self {
        self.query = user_status_query(schema);
        self
    }
}

/// The status lookup against `schema`, quoted as an identifier.
fn user_status_query(schema: &str) -> String {
    let schema = quote_ident(schema);
    format!(
        "SELECT role, \
                COALESCE(banned, false) AND (\"banExpires\" IS NULL OR \"banExpires\" > now()) \
         FROM {schema}.\"user\" WHERE id::text = $1"
    )
}

#[async_trait]
impl UserStatusStore for PgNeonUserStatus {
    async fn status(&self, _issuer: &str, sub: &str) -> Result<Option<UserStatus>, BoxError> {
        let row: Option<(Option<String>, bool)> = sqlx::query_as(&self.query)
            .bind(sub)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|(role, banned)| UserStatus::from_role(role.as_deref(), banned)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_the_users_schema() {
        let default = user_status_query(DEFAULT_NEON_AUTH_SCHEMA);
        assert!(default.contains(r#"FROM "neon_auth"."user""#), "{default}");
        let odd = user_status_query(r#"a"b"#);
        assert!(odd.contains(r#"FROM "a""b"."user""#), "{odd}");
    }

    #[test]
    fn quotes_the_schema() {
        assert!(lookup_query("authz").contains(r#"FROM "authz".api_keys"#));
        let odd = lookup_query(r#"my"schema"#);
        assert!(odd.contains(r#"FROM "my""schema".api_keys"#), "{odd}");
        assert!(
            odd.contains(r#"JOIN "my""schema".service_accounts"#),
            "{odd}"
        );
    }
}
