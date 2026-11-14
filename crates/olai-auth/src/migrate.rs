use sqlx::PgPool;
use sqlx::migrate::Migrator;

use crate::error::BoxError;
use crate::postgres::quote_ident;

/// The `authz` migrations, embedded at compile time: `service_accounts` and `api_keys`
/// ([`PgApiKeyStore`](crate::PgApiKeyStore)), `groups`, `memberships` and `grants`
/// ([`PgGrantStore`](crate::PgGrantStore)), and an `authz_reader` role with `SELECT` on
/// them.
///
/// Their names are unqualified: run them with [`migrate`], which sets the schema.
pub static MIGRATOR: Migrator = sqlx::migrate!("migrations/authz");

/// Applies the pending [`MIGRATOR`] migrations to `schema` (`authz` for the stores'
/// default), creating it if needed.
///
/// The migrations and their ledger (`<schema>._sqlx_migrations`) live in `schema`, so they
/// don't mix with the host's own migrations. Runs on one connection of `pool` with
/// `search_path` set to `schema`, reset afterwards. The migrator holds a session advisory
/// lock, so use a direct connection, not a transaction-mode pooler.
pub async fn migrate(pool: &PgPool, schema: &str) -> Result<(), BoxError> {
    let mut conn = pool.acquire().await?;
    let schema = quote_ident(schema);
    sqlx::raw_sql(&format!(
        "CREATE SCHEMA IF NOT EXISTS {schema}; SET search_path TO {schema}"
    ))
    .execute(&mut *conn)
    .await?;
    let migrated = MIGRATOR.run(&mut *conn).await;
    let reset = sqlx::raw_sql("RESET search_path").execute(&mut *conn).await;
    migrated?;
    reset?;
    Ok(())
}
