//! The `authz` migrations, `PgApiKeyStore` and `PgGrantStore` against a live Postgres.
//!
//! Runs only when `DATABASE_URL_PG` is set (`just test-auth-pg` starts a container and
//! sets it); otherwise the tests pass without checking anything. Each test migrates and
//! drops its own schema.

mod support;

use std::time::{SystemTime, UNIX_EPOCH};

use olai_auth::{ApiKeyStore, PgApiKeyStore, PgGrantStore, api_key_digest, migrate, parse_api_key};
use sqlx::PgPool;
use support::grant_store::{GROUP_ENG, GROUP_STAFF, conformance};

const LIVE: &str =
    "olk_1111111111111111_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const REVOKED: &str =
    "olk_2222222222222222_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const EXPIRED: &str =
    "olk_3333333333333333_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const EXPIRES_LATER: &str =
    "olk_4444444444444444_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DISABLED_ACCOUNT: &str =
    "olk_5555555555555555_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const UNKNOWN: &str =
    "olk_6666666666666666_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

const LIVE_ACCOUNT: &str = "0b8e6a3c-1f2d-4e5a-9b7c-8d6e5f4a3b2c";
const OFF_ACCOUNT: &str = "9a1b2c3d-4e5f-4a6b-8c7d-0e1f2a3b4c5d";

/// A database to test against and a fresh schema name, or `None` to skip.
async fn database(what: &str, tag: &str) -> Option<(PgPool, String)> {
    let Ok(url) = std::env::var("DATABASE_URL_PG") else {
        eprintln!("DATABASE_URL_PG is not set: skipping the Postgres {what} test");
        return None;
    };
    let pool = PgPool::connect(&url).await.unwrap();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let schema = format!("olai_auth_test_{tag}_{nanos}");
    migrate(&pool, &schema).await.unwrap();
    // Migrating again finds nothing pending.
    migrate(&pool, &schema).await.unwrap();
    Some((pool, schema))
}

async fn drop_schema(pool: &PgPool, schema: &str) {
    sqlx::raw_sql(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(pool)
        .await
        .unwrap();
}

async fn seed(pool: &PgPool, schema: &str) {
    sqlx::raw_sql(&format!(
        "INSERT INTO {schema}.service_accounts (id, name, disabled_at) VALUES
           ('{LIVE_ACCOUNT}', 'ingest-bot', NULL),
           ('{OFF_ACCOUNT}', 'retired-bot', now() - interval '1 day');"
    ))
    .execute(pool)
    .await
    .unwrap();
    let keys = [
        (LIVE, LIVE_ACCOUNT, "NULL", "NULL"),
        (REVOKED, LIVE_ACCOUNT, "NULL", "now() - interval '1 minute'"),
        (EXPIRED, LIVE_ACCOUNT, "now() - interval '1 minute'", "NULL"),
        (
            EXPIRES_LATER,
            LIVE_ACCOUNT,
            "now() + interval '1 day'",
            "NULL",
        ),
        (DISABLED_ACCOUNT, OFF_ACCOUNT, "NULL", "NULL"),
    ];
    for (key, account, expires_at, revoked_at) in keys {
        sqlx::query(&format!(
            "INSERT INTO {schema}.api_keys (id, account_id, sha256, scopes, expires_at, revoked_at)
             VALUES ($1, $2::uuid, $3, ARRAY['ingest', 'read'], {expires_at}, {revoked_at})"
        ))
        .bind(parse_api_key(key).unwrap())
        .bind(account)
        .bind(api_key_digest(key).to_vec())
        .execute(pool)
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn looks_up_live_keys_only() {
    let Some((pool, schema)) = database("API key", "keys").await else {
        return;
    };
    seed(&pool, &schema).await;

    let store = PgApiKeyStore::new(pool.clone()).with_schema(&schema);
    let lookup = |key: &'static str| {
        let store = store.clone();
        async move { store.lookup(parse_api_key(key).unwrap()).await.unwrap() }
    };

    let live = lookup(LIVE).await.expect("live key");
    assert!(live.matches(LIVE));
    assert_eq!(live.account_id, LIVE_ACCOUNT);
    assert_eq!(live.account_name, "ingest-bot");
    assert_eq!(live.scopes, ["ingest", "read"]);
    assert!(lookup(EXPIRES_LATER).await.is_some());

    for dead in [REVOKED, EXPIRED, DISABLED_ACCOUNT, UNKNOWN] {
        assert!(lookup(dead).await.is_none(), "{dead}");
    }

    // Missing tables are an error (a 503 for the caller), not a silent miss.
    let missing = PgApiKeyStore::new(pool.clone()).with_schema(&format!("{schema}_missing"));
    assert!(missing.lookup(parse_api_key(LIVE).unwrap()).await.is_err());

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
async fn grant_store_meets_the_contract() {
    let Some((pool, schema)) = database("grant store", "grants").await else {
        return;
    };
    sqlx::raw_sql(&format!(
        "INSERT INTO {schema}.groups (id, name) VALUES ('{GROUP_STAFF}', 'staff'), ('{GROUP_ENG}', 'eng');
         INSERT INTO {schema}.memberships (group_id, principal_type, principal_id) VALUES
           ('{GROUP_STAFF}', 'group', '{GROUP_ENG}'),
           ('{GROUP_ENG}', 'user', 'alice'),
           ('{GROUP_STAFF}', 'user', 'bob');"
    ))
    .execute(&pool)
    .await
    .unwrap();
    conformance(&PgGrantStore::new(pool.clone()).with_schema(&schema)).await;
    drop_schema(&pool, &schema).await;
}
