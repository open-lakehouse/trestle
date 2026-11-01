//! Glue shared by the sqlx-backed backends ([`sqlite`](super::sqlite) and
//! [`postgres`](super::postgres)).
//!
//! Everything here is **dialect-independent**: it never names a concrete
//! `sqlx::Database`, so it is defined exactly once regardless of which (or both)
//! of the `sqlite` / `postgres` features are enabled. The SQL-bearing code — the
//! `op_*` functions with their compile-time-checked `sqlx::query!` literals, which
//! *are* dialect-bound — lives in each backend module.

use std::sync::Arc;

use crate::filter::{CompareOp, Filter, Predicate};
use crate::index::PropertyIndex;
use crate::{Error, Result};

/// The current span's OpenTelemetry status/error fields, recorded on failure.
///
/// The span must declare `otel.status_code` and `error.type` as
/// [`tracing::field::Empty`]. Only the error *kind* ([`Error::kind_str`]) is
/// recorded — never a payload or message body.
pub(crate) fn record_err<T>(result: &Result<T>) {
    if let Err(e) = result {
        let span = tracing::Span::current();
        span.record("otel.status_code", "ERROR");
        span.record("error.type", e.kind_str());
    }
}

/// Resolves an edge label to its paired inverse label, if any (see
/// [`InMemoryStore`](crate::InMemoryStore)).
pub type InverseResolver = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// Map an [`sqlx::Error`] onto the store's [`Error`], collapsing the two cases the
/// store gives dedicated variants (`RowNotFound` → [`Error::NotFound`], a unique
/// violation → [`Error::AlreadyExists`]) and treating everything else as a generic
/// backend error. Dialect-independent: `is_unique_violation()` is honored by every
/// sqlx driver.
impl From<sqlx::Error> for Error {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::RowNotFound => Error::NotFound,
            sqlx::Error::Database(db) if db.is_unique_violation() => Error::AlreadyExists,
            other => Error::generic(other.to_string()),
        }
    }
}

/// Merge a backend's embedded schema migrations with a consumer's own into one
/// ordered ledger.
///
/// This is the shared body behind each backend's `migrator_with`. Two
/// `sqlx::migrate!` migrators would share sqlx's single hardcoded
/// `_sqlx_migrations` ledger, forcing non-overlapping version ranges and
/// `set_ignore_missing(true)` on both; merging into one migrator sidesteps that.
/// Versions across the combined set must be unique and are applied in ascending
/// order, so a consumer numbers its migrations above the backend's low range.
pub(crate) fn merge(
    base: sqlx::migrate::Migrator,
    extra: impl IntoIterator<Item = sqlx::migrate::Migration>,
) -> sqlx::migrate::Migrator {
    let mut migrations: Vec<sqlx::migrate::Migration> = base.migrations.iter().cloned().collect();
    migrations.extend(extra);
    migrations.sort_by_key(|m| m.version);
    sqlx::migrate::Migrator {
        migrations: std::borrow::Cow::Owned(migrations),
        ..sqlx::migrate::Migrator::DEFAULT
    }
}

/// An index-usable conjunct implied by a search filter, for a declared [`PropertyIndex`].
///
/// The sqlx backends prepend each hint to the exact filter SQL
/// (`WHERE label = ? AND <hints> AND <filter>`). A hint holds whenever the exact predicate holds,
/// so it never changes the result set, but unlike the exact form (path bound as a parameter,
/// wrapped in a `COALESCE` type guard) it matches the index expression the backend created.
#[derive(Debug, PartialEq)]
pub(crate) enum Hint<'f> {
    /// `<expr> <op> value`, from a string comparison on an indexed path.
    Cmp {
        index: &'f PropertyIndex,
        op: CompareOp,
        value: &'f str,
    },
    /// `<expr> IN (values…)`, from an `Or` of string `Eq`s on one indexed path.
    In {
        index: &'f PropertyIndex,
        values: Vec<&'f str>,
    },
}

/// Collect the index hints implied by `filter` for searches on `label`.
///
/// Only top-level conjuncts are considered (the filter itself, or the members of a top-level
/// `And`, recursively), so a hint is always implied by the whole filter. Comparisons against
/// non-string values, `Ne`/`Contains`/`Exists`, and paths not declared for `label` yield none.
pub(crate) fn index_hints<'f>(
    filter: &'f Filter,
    label: &str,
    indexes: &'f [PropertyIndex],
) -> Vec<Hint<'f>> {
    let mut hints = Vec::new();
    collect_hints(filter, label, indexes, &mut hints);
    hints
}

fn collect_hints<'f>(
    filter: &'f Filter,
    label: &str,
    indexes: &'f [PropertyIndex],
    out: &mut Vec<Hint<'f>>,
) {
    match filter {
        Filter::And(fs) => {
            for f in fs {
                collect_hints(f, label, indexes, out);
            }
        }
        Filter::Predicate(_) => {
            if let Some((index, op, value)) = string_comparison(filter, label, indexes)
                && op != CompareOp::Ne
                && op != CompareOp::Contains
            {
                out.push(Hint::Cmp { index, op, value });
            }
        }
        Filter::Or(fs) => {
            // An empty `Or` is FALSE; nothing to hint (and an empty IN list isn't valid SQL).
            let Some((first, rest)) = fs.split_first() else {
                return;
            };
            let Some((index, CompareOp::Eq, value)) = string_comparison(first, label, indexes)
            else {
                return;
            };
            let mut values = vec![value];
            for f in rest {
                match string_comparison(f, label, indexes) {
                    Some((i, CompareOp::Eq, v)) if i == index => values.push(v),
                    _ => return,
                }
            }
            out.push(Hint::In { index, values });
        }
        Filter::Not(_) => {}
    }
}

/// The declared index, operator and string comparand of a `Compare` predicate on an indexed
/// path for `label`, or `None` for anything else.
fn string_comparison<'f>(
    filter: &'f Filter,
    label: &str,
    indexes: &'f [PropertyIndex],
) -> Option<(&'f PropertyIndex, CompareOp, &'f str)> {
    let Filter::Predicate(Predicate::Compare { path, op, value }) = filter else {
        return None;
    };
    let value = value.as_str()?;
    let index = indexes
        .iter()
        .find(|i| i.label() == label && i.path() == path)?;
    Some((index, *op, value))
}

/// Every declared index with a distinct path, in declaration order. Labels that declare the same
/// path share one `(label, <expr>)` index.
pub(crate) fn distinct_paths(indexes: &[PropertyIndex]) -> Vec<&PropertyIndex> {
    let mut out: Vec<&PropertyIndex> = Vec::new();
    for idx in indexes {
        if !out.iter().any(|o| o.path() == idx.path()) {
            out.push(idx);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conformance::ConformanceLabel;

    fn indexes() -> Vec<PropertyIndex> {
        vec![
            PropertyIndex::new(ConformanceLabel::Node, "location_key").unwrap(),
            PropertyIndex::new(ConformanceLabel::Node, "owner").unwrap(),
        ]
    }

    #[test]
    fn eq_and_range_hint() {
        let idx = indexes();
        let eq = Filter::eq("location_key", "s3://b/");
        let hints = index_hints(&eq, "node", &idx);
        assert_eq!(
            hints,
            [Hint::Cmp {
                index: &idx[0],
                op: CompareOp::Eq,
                value: "s3://b/"
            }]
        );

        let range = Filter::all([
            Filter::ge("location_key", "s3://b/"),
            Filter::all([Filter::lt("location_key", "s3://b0")]),
            Filter::gt("size", 1),
        ]);
        let hints = index_hints(&range, "node", &idx);
        assert_eq!(
            hints,
            [
                Hint::Cmp {
                    index: &idx[0],
                    op: CompareOp::Ge,
                    value: "s3://b/"
                },
                Hint::Cmp {
                    index: &idx[0],
                    op: CompareOp::Lt,
                    value: "s3://b0"
                },
            ]
        );
    }

    #[test]
    fn or_of_eq_on_one_path_becomes_in() {
        let idx = indexes();
        let f = Filter::any([
            Filter::eq("location_key", "a"),
            Filter::eq("location_key", "b"),
        ]);
        assert_eq!(
            index_hints(&f, "node", &idx),
            [Hint::In {
                index: &idx[0],
                values: vec!["a", "b"]
            }]
        );
    }

    #[test]
    fn no_hint_when_not_implied_or_not_declared() {
        let idx = indexes();
        let none = [
            // Mixed paths in an Or.
            Filter::any([Filter::eq("location_key", "a"), Filter::eq("owner", "b")]),
            // Non-Eq member in an Or.
            Filter::any([
                Filter::eq("location_key", "a"),
                Filter::gt("location_key", "b"),
            ]),
            Filter::any([]),
            // Not a top-level conjunct.
            Filter::eq("location_key", "a").negate(),
            Filter::any([Filter::all([Filter::eq("location_key", "a")])]),
            // Non-string comparand, unsupported op, Exists, undeclared path.
            Filter::eq("location_key", 1),
            Filter::ne("location_key", "a"),
            Filter::contains("location_key", "a"),
            Filter::exists("location_key"),
            Filter::eq("other", "a"),
        ];
        for f in none {
            assert!(index_hints(&f, "node", &idx).is_empty(), "{f:?}");
        }
        // Declared path, but for another label.
        assert!(index_hints(&Filter::eq("location_key", "a"), "other", &idx).is_empty());
    }

    #[test]
    fn distinct_paths_dedupes_across_labels() {
        let idx = [
            PropertyIndex::new(ConformanceLabel::Node, "k").unwrap(),
            PropertyIndex::new(ConformanceLabel::Other, "k").unwrap(),
            PropertyIndex::new(ConformanceLabel::Other, "j").unwrap(),
        ];
        let paths: Vec<_> = distinct_paths(&idx)
            .iter()
            .map(|i| i.index_name())
            .collect();
        assert_eq!(paths, ["objects_prop_k", "objects_prop_j"]);
    }
}
