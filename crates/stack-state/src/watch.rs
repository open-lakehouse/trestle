//! [`Watcher`]: notice writes other processes made to the store.

use std::time::Duration;

use futures::Stream;
use rusqlite::Connection;

use crate::error::Result;
use crate::store::{Store, connect};

/// Notices commits to the store made through any other connection, in this process or
/// another (the CLI writing while the app is open, say). It holds its own connection and
/// compares SQLite's `PRAGMA data_version`, which changes exactly when someone else commits.
/// It says *that* something changed, not what: a consumer re-reads what it shows.
pub struct Watcher {
    conn: Connection,
    version: i64,
}

impl Store {
    /// A [`Watcher`] for this store's file.
    pub fn watcher(&self) -> Result<Watcher> {
        let conn = connect(self.path())?;
        let version = data_version(&conn)?;
        Ok(Watcher { conn, version })
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
        let changed = version != self.version;
        self.version = version;
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
