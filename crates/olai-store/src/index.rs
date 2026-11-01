//! Declared property indexes, so payload searches on hot paths can use a database index.
//!
//! A [`PropertyIndex`] names a string-valued field of the [`properties`](crate::Object::properties)
//! payload that a store should index for one label. The sqlx backends (`SqlStore`, `PgStore`)
//! take a set of them via `with_property_indexes`, create the matching expression indexes with
//! `ensure_property_indexes`, and from then on emit index-usable SQL for string comparisons on
//! those paths in [`search`](crate::ObjectStoreReader::search):
//!
//! - an exact match ([`Filter::eq`](crate::Filter::eq));
//! - an IN-list ([`Filter::any`](crate::Filter::any) of `eq` on the same path);
//! - a range ([`Filter::ge`](crate::Filter::ge) / [`Filter::lt`](crate::Filter::lt) and friends),
//!   e.g. a bytewise prefix scan.
//!
//! Declaring an index never changes search results, only the plan. Comparisons on undeclared
//! paths, on other labels, or against non-string values run exactly as before.
//!
//! # Examples
//!
//! ```
//! use olai_store::PropertyIndex;
//! # use olai_store::conformance::ConformanceLabel;
//!
//! let idx = PropertyIndex::new(ConformanceLabel::Node, "location_key").unwrap();
//! assert_eq!(idx.label(), "node");
//!
//! // Segments must be plain identifiers.
//! assert!(PropertyIndex::new(ConformanceLabel::Node, "bad-key").is_err());
//! ```

use crate::filter::FieldPath;
use crate::label::Label;
use crate::{Error, Result};

/// Prefix of every generated property index name.
const INDEX_PREFIX: &str = "objects_prop_";

/// Postgres truncates identifiers longer than this many bytes, which would make two long paths
/// share an index name, so longer names are rejected up front.
const MAX_IDENTIFIER_LEN: usize = 63;

/// A string-valued payload path to index for one label.
///
/// Construct with [`PropertyIndex::new`], which validates the path so it can be written into
/// SQL as a literal. See the [module documentation](self) for what an index serves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyIndex {
    label: String,
    path: FieldPath,
}

impl PropertyIndex {
    /// Declare an index on `path` for objects labelled `label`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidArgument`] when the path is empty, when a segment is not a plain
    /// identifier (`[A-Za-z_][A-Za-z0-9_]*`), or when the derived index name would exceed
    /// Postgres' 63-byte identifier limit.
    pub fn new<L: Label>(label: L, path: impl Into<FieldPath>) -> Result<Self> {
        let path = path.into();
        if path.segments().is_empty() {
            return Err(Error::invalid_argument("property index path is empty"));
        }
        if let Some(seg) = path.segments().iter().find(|s| !is_identifier(s)) {
            return Err(Error::invalid_argument(format!(
                "property index path segment {seg:?} is not an identifier ([A-Za-z_][A-Za-z0-9_]*)"
            )));
        }
        let idx = Self {
            label: label.as_str().to_owned(),
            path,
        };
        if idx.index_name().len() > MAX_IDENTIFIER_LEN {
            return Err(Error::invalid_argument(format!(
                "property index name {:?} exceeds {MAX_IDENTIFIER_LEN} bytes",
                idx.index_name()
            )));
        }
        Ok(idx)
    }

    /// The label whose searches use this index.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The indexed payload path.
    pub fn path(&self) -> &FieldPath {
        &self.path
    }

    /// The database index name, shared by every label that declares the same path:
    /// `objects_prop_<seg>.<seg>…`.
    ///
    /// Segments are identifiers and so never contain `.`, which keeps the name unambiguous
    /// (`["a_b"]` and `["a", "b"]` get different indexes). The `.` means the name has to be
    /// double-quoted in DDL; it never contains a `"`.
    pub fn index_name(&self) -> String {
        format!("{INDEX_PREFIX}{}", self.path.segments().join("."))
    }
}

/// Whether `s` matches `[A-Za-z_][A-Za-z0-9_]*`, so it needs no quoting as a JSON path segment
/// in either dialect and can't break out of a SQL string literal.
fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conformance::ConformanceLabel;

    #[test]
    fn accepts_identifier_paths() {
        let idx = PropertyIndex::new(ConformanceLabel::Node, "location_key").unwrap();
        assert_eq!(idx.label(), "node");
        assert_eq!(idx.index_name(), "objects_prop_location_key");

        let nested = PropertyIndex::new(ConformanceLabel::Other, "meta._k2").unwrap();
        assert_eq!(nested.path().segments(), ["meta", "_k2"]);
        assert_eq!(nested.index_name(), "objects_prop_meta._k2");
        assert_ne!(
            PropertyIndex::new(ConformanceLabel::Node, FieldPath::new(["a_b"]))
                .unwrap()
                .index_name(),
            PropertyIndex::new(ConformanceLabel::Node, "a.b")
                .unwrap()
                .index_name(),
        );
    }

    #[test]
    fn rejects_unsafe_segments() {
        for bad in ["", "a.", "1a", "a-b", "a'b", "a\"b", "{a}", "a b", "ä"] {
            assert!(
                PropertyIndex::new(ConformanceLabel::Node, FieldPath::new([bad])).is_err(),
                "{bad:?} should be rejected"
            );
        }
        assert!(
            PropertyIndex::new(ConformanceLabel::Node, FieldPath::new(Vec::<String>::new()))
                .is_err(),
            "empty path"
        );
    }

    #[test]
    fn rejects_names_over_the_identifier_limit() {
        let ok = "a".repeat(MAX_IDENTIFIER_LEN - INDEX_PREFIX.len());
        assert!(PropertyIndex::new(ConformanceLabel::Node, ok.as_str()).is_ok());
        let long = "a".repeat(MAX_IDENTIFIER_LEN - INDEX_PREFIX.len() + 1);
        assert!(PropertyIndex::new(ConformanceLabel::Node, long.as_str()).is_err());
    }
}
