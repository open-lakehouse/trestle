//! `--json` output for the store-backed read commands.
//!
//! Every document is an envelope, `{ "version": 1, "kind": "<kind>", "data": … }`, so a reader
//! can tell what it holds and refuse a version it doesn't know. Field names are stable within
//! a version.

use serde::Serialize;

use crate::error::{Error, Result};

/// The envelope's version.
pub(super) const JSON_VERSION: u32 = 1;

#[derive(Serialize)]
struct Envelope<'a, T> {
    version: u32,
    kind: &'a str,
    data: &'a T,
}

/// Print `data` as a `kind` JSON document when `json`, else run `human`.
pub(super) fn emit<T: Serialize>(
    json: bool,
    kind: &str,
    data: &T,
    human: impl FnOnce(&T),
) -> Result<()> {
    if json {
        let doc = serde_json::to_string_pretty(&Envelope {
            version: JSON_VERSION,
            kind,
            data,
        })
        .map_err(|e| Error::other(format!("serializing {kind}: {e}")))?;
        println!("{doc}");
    } else {
        human(data);
    }
    Ok(())
}
