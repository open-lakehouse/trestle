//! [`Watcher`]: notice writes other processes made to the store.

#[cfg(feature = "postgres")]
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::Stream;
use rusqlite::Connection;

use crate::error::Result;
#[cfg(feature = "postgres")]
use crate::pg::{self, PgSession};
use crate::store::{Store, connect};

/// Notices commits to the store made through any other connection, in this process or
/// another (the CLI writing while the app is open, say), and, for an attached store, every
/// commit to its shared database (a colleague's edit).
///
/// For the local file it holds its own connection and compares SQLite's
/// `PRAGMA data_version`, which changes exactly when someone else commits. For a shared
/// database it polls a counter every write there bumps, this store's own writes included.
/// It says *that* something changed, not what: a consumer re-reads what it shows.
pub struct Watcher {
    conn: Connection,
    version: i64,
    #[cfg(feature = "postgres")]
    remote: Option<(Arc<Mutex<PgSession>>, i64)>,
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watcher").finish_non_exhaustive()
    }
}

impl Store {
    /// A [`Watcher`] for this store: its local file, and its shared database when attached.
    pub fn watcher(&self) -> Result<Watcher> {
        let conn = connect(self.path())?;
        let version = data_version(&conn)?;
        #[cfg(feature = "postgres")]
        let remote = match self.remote_session() {
            Some(session) => {
                let seq = pg::change_seq(&session.lock().unwrap_or_else(|e| e.into_inner()))?;
                Some((session, seq))
            }
            None => None,
        };
        Ok(Watcher {
            conn,
            version,
            #[cfg(feature = "postgres")]
            remote,
        })
    }
}

fn data_version(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("PRAGMA data_version", [], |r| r.get(0))?)
}

impl Watcher {
    /// Whether another connection committed since the last call (or since the watcher was
    /// made).
    pub fn changed(&mut self) -> Result<bool> {
        let version = data_version(&self.conn)?;
        #[cfg_attr(not(feature = "postgres"), allow(unused_mut))]
        let mut changed = version != self.version;
        self.version = version;
        #[cfg(feature = "postgres")]
        if let Some((session, seen)) = &mut self.remote {
            let seq = pg::change_seq(&session.lock().unwrap_or_else(|e| e.into_inner()))?;
            changed |= seq != *seen;
            *seen = seq;
        }
        Ok(changed)
    }

    /// A stream yielding once per poll (every `interval`) that saw a change. Needs a Tokio
    /// runtime with the timer enabled.
    pub fn changes(self, interval: Duration) -> impl Stream<Item = Result<()>> {
        futures::stream::unfold(self, move |mut watcher| async move {
            loop {
                tokio::time::sleep(interval).await;
                match watcher.changed() {
                    Ok(true) => return Some((Ok(()), watcher)),
                    Ok(false) => continue,
                    Err(e) => return Some((Err(e), watcher)),
                }
            }
        })
    }
}
