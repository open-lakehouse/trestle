//! `PgNeonUserStatus` against Neon Auth's `user` table layout, on a live Postgres.
//!
//! Runs only when `DATABASE_URL_PG` is set (`just test-auth-pg` starts a container and
//! sets it); otherwise the tests pass without checking anything. Each test creates and
//! drops its own schema.

use std::time::{SystemTime, UNIX_EPOCH};

use olai_auth::{PgNeonUserStatus, UserStatus, UserStatusStore};
use sqlx::PgPool;

const ISSUER: &str = "https://auth.example.com/neondb/auth";

const ADMIN: UserStatus = UserStatus {
    banned: false,
    admin: true,
};
const USER: UserStatus = UserStatus {
    banned: false,
    admin: false,
};
const BANNED: UserStatus = UserStatus {
    banned: true,
    admin: false,
};

/// A database to test against and a fresh schema name, or `None` to skip.
async fn database(tag: &str) -> Option<(PgPool, String)> {
    let Ok(url) = std::env::var("DATABASE_URL_PG") else {
        eprintln!("DATABASE_URL_PG is not set: skipping the Postgres Neon user status test");
        return None;
    };
    let pool = PgPool::connect(&url).await.unwrap();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    Some((pool, format!("olai_auth_neon_{tag}_{nanos}")))
}

/// The columns of Neon Auth's `user` table the store reads, plus one it must not need,
/// in `schema`.
async fn neon_auth(pool: &PgPool, schema: &str) {
    sqlx::raw_sql(&format!(
        r#"CREATE SCHEMA "{schema}";
           CREATE TABLE "{schema}"."user" (id uuid PRIMARY KEY, email text, role text,
             banned bool, "banExpires" timestamptz);
           INSERT INTO "{schema}"."user" VALUES
             ('00000000-0000-0000-0000-000000000001', 'admin@example.com', 'admin', false, NULL),
             ('00000000-0000-0000-0000-000000000002', 'user@example.com', NULL, NULL, NULL),
             ('00000000-0000-0000-0000-000000000003', 'multi@example.com', 'user, admin', NULL, NULL),
             ('00000000-0000-0000-0000-000000000004', 'banned@example.com', 'user', true, NULL),
             ('00000000-0000-0000-0000-000000000005', 'lapsed@example.com', 'user', true, now() - interval '1 day'),
             ('00000000-0000-0000-0000-000000000006', 'until@example.com', 'user', true, now() + interval '1 day');"#
    ))
    .execute(pool)
    .await
    .unwrap();
}

async fn drop_schema(pool: &PgPool, schema: &str) {
    sqlx::raw_sql(&format!(r#"DROP SCHEMA "{schema}" CASCADE"#))
        .execute(pool)
        .await
        .unwrap();
}

async fn status(store: &PgNeonUserStatus, id: u8) -> Option<UserStatus> {
    store
        .status(ISSUER, &format!("00000000-0000-0000-0000-00000000000{id}"))
        .await
        .unwrap()
}

#[tokio::test]
async fn reads_role_and_ban() {
    let Some((pool, schema)) = database("roles").await else {
        return;
    };
    neon_auth(&pool, &schema).await;
    let store = PgNeonUserStatus::new(pool.clone()).with_schema(&schema);

    assert_eq!(status(&store, 1).await, Some(ADMIN));
    assert_eq!(status(&store, 2).await, Some(USER), "NULL role and ban");
    assert_eq!(status(&store, 3).await, Some(ADMIN), "a role list");
    assert_eq!(status(&store, 4).await, Some(BANNED));
    // A ban that has expired no longer counts; one that hasn't yet does.
    assert_eq!(status(&store, 5).await, Some(USER));
    assert_eq!(status(&store, 6).await, Some(BANNED));
    // Unknown users, and subjects that aren't user ids at all.
    assert_eq!(status(&store, 9).await, None);
    assert_eq!(store.status(ISSUER, "not-a-uuid").await.unwrap(), None);
    // The issuer is not consulted.
    assert_eq!(
        store
            .status(
                "https://other.example.com",
                "00000000-0000-0000-0000-000000000001"
            )
            .await
            .unwrap(),
        Some(ADMIN)
    );

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
async fn a_missing_table_is_an_error_not_an_unknown_user() {
    let Some((pool, schema)) = database("missing").await else {
        return;
    };
    // No such schema: an error, so a misconfigured store does not reject every user as
    // unknown without saying why.
    let store = PgNeonUserStatus::new(pool.clone()).with_schema(&schema);
    assert!(store.status(ISSUER, "x").await.is_err());

    // The schema, but no `user` table in it.
    sqlx::raw_sql(&format!(r#"CREATE SCHEMA "{schema}""#))
        .execute(&pool)
        .await
        .unwrap();
    assert!(store.status(ISSUER, "x").await.is_err());

    drop_schema(&pool, &schema).await;
}
