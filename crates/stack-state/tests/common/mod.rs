//! The two places a store's shared tables can live, for tests that run against both.
//!
//! [`Harness::sqlite`] keeps everything in one `state.db`. [`Harness::postgres`] (feature
//! `postgres`) puts the shared tables in a schema of their own on the database
//! `DATABASE_URL_PG` names, attached to a local `state.db`, and drops the schema when done;
//! it is `None`, and the test skips, when the variable is unset (`just test-state-pg` sets
//! it).

#![allow(dead_code)]

use std::path::{Path, PathBuf};

#[cfg(feature = "postgres")]
use olai_stack_state::{AttachmentId, RemoteLocation};
use olai_stack_state::{Draft, Store};
use olai_stack_topology::{EnvManifest, PlanCtx, Selection};

/// A draft listing `modules`.
pub fn draft(modules: &[&str]) -> Draft {
    Draft::new(EnvManifest::new(
        Selection::modules(modules.iter().copied()),
        PlanCtx::default(),
    ))
}

/// A store's local file and, on Postgres, the database its shared tables are in.
pub struct Harness {
    dir: tempfile::TempDir,
    #[cfg(feature = "postgres")]
    remote: Option<Remote>,
}

#[cfg(feature = "postgres")]
struct Remote {
    /// The connection string, its `search_path` set to the test's schema.
    url: String,
    /// The connection string as given.
    admin: String,
    schema: String,
    attachment: AttachmentId,
}

impl Harness {
    /// Everything in one SQLite file.
    pub fn sqlite() -> Self {
        Harness {
            dir: tempfile::tempdir().unwrap(),
            #[cfg(feature = "postgres")]
            remote: None,
        }
    }

    /// The shared tables in a fresh schema on `DATABASE_URL_PG`, or `None` when it's unset.
    #[cfg(feature = "postgres")]
    pub fn postgres() -> Option<Self> {
        let Ok(admin) = std::env::var("DATABASE_URL_PG") else {
            eprintln!("DATABASE_URL_PG is not set: skipping the Postgres run");
            return None;
        };
        let schema = format!("stack_state_test_{}", uuid::Uuid::new_v4().simple());
        raw(&admin, &format!("CREATE SCHEMA {schema}")).unwrap();
        let separator = if admin.contains('?') { '&' } else { '?' };
        let url = format!("{admin}{separator}options=-csearch_path%3D{schema}");
        Store::init_remote(&url).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let attachment = attach(&dir.path().join("state.db"));
        Some(Harness {
            dir,
            remote: Some(Remote {
                url,
                admin,
                schema,
                attachment,
            }),
        })
    }

    /// Whether the shared tables are on Postgres.
    pub fn is_postgres(&self) -> bool {
        #[cfg(feature = "postgres")]
        return self.remote.is_some();
        #[cfg(not(feature = "postgres"))]
        false
    }

    /// The local file.
    pub fn path(&self) -> PathBuf {
        self.dir.path().join("state.db")
    }

    /// A new handle on the store.
    pub fn store(&self) -> Store {
        self.store_at(&self.path())
    }

    /// A new handle on the store, with `path` as its local file: another machine, when it
    /// isn't [`path`](Self::path). On SQLite there's nothing to share, so it is a store of
    /// its own.
    pub fn store_at(&self, path: &Path) -> Store {
        #[cfg(feature = "postgres")]
        if let Some(remote) = &self.remote {
            let attachment = if path == self.path() {
                remote.attachment.clone()
            } else {
                attach(path)
            };
            let url = remote.url.clone();
            return Store::open_attached(path, &attachment, |_| {
                Ok::<_, std::convert::Infallible>(url)
            })
            .unwrap();
        }
        Store::open(path).unwrap()
    }

    /// The connection string of the shared tables' database.
    #[cfg(feature = "postgres")]
    pub fn url(&self) -> Option<&str> {
        self.remote.as_ref().map(|r| r.url.as_str())
    }

    /// Run `sql` straight on the database holding the shared tables, past the store.
    pub fn raw(&self, sql: &str) -> Result<(), String> {
        #[cfg(feature = "postgres")]
        if let Some(remote) = &self.remote {
            return raw(&remote.url, sql);
        }
        let conn = rusqlite::Connection::open(self.path()).map_err(|e| e.to_string())?;
        conn.execute_batch(sql).map_err(|e| e.to_string())
    }
}

/// Record the test database as an attachment of the local store at `path`.
#[cfg(feature = "postgres")]
fn attach(path: &Path) -> AttachmentId {
    Store::open(path)
        .unwrap()
        .attach(
            "team",
            RemoteLocation::Neon {
                org: None,
                project: "test".into(),
                branch: None,
                database: "trestle_state".into(),
            },
        )
        .unwrap()
        .id
}

#[cfg(feature = "postgres")]
fn raw(url: &str, sql: &str) -> Result<(), String> {
    // On a thread of its own: async tests call it from inside a runtime.
    std::thread::scope(|scope| scope.spawn(|| raw_blocking(url, sql)).join().unwrap())
}

#[cfg(feature = "postgres")]
fn raw_blocking(url: &str, sql: &str) -> Result<(), String> {
    use sqlx::Connection as _;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut conn = sqlx::PgConnection::connect(url)
            .await
            .map_err(|e| e.to_string())?;
        let result = sqlx::raw_sql(sql).execute(&mut conn).await;
        let _ = conn.close().await;
        result.map(|_| ()).map_err(|e| {
            e.as_database_error()
                .map_or_else(|| e.to_string(), |d| d.message().to_owned())
        })
    })
}

#[cfg(feature = "postgres")]
impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(remote) = &self.remote {
            let _ = raw(
                &remote.admin,
                &format!("DROP SCHEMA {} CASCADE", remote.schema),
            );
        }
    }
}

/// Run each named test, a `fn(&Harness)` in the enclosing module, on SQLite and on Postgres.
#[macro_export]
macro_rules! on_both {
    ($($name:ident),* $(,)?) => {
        mod sqlite {
            $(
                #[test]
                fn $name() {
                    super::$name(&$crate::common::Harness::sqlite());
                }
            )*
        }

        #[cfg(feature = "postgres")]
        mod postgres {
            $(
                #[test]
                fn $name() {
                    if let Some(harness) = $crate::common::Harness::postgres() {
                        super::$name(&harness);
                    }
                }
            )*
        }
    };
}
