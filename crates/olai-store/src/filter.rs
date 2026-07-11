//! A small, serializable filter AST for searching object and association payloads.
//!
//! [`Filter`] is a boolean combination of [`Predicate`]s over a JSON payload — the
//! [`properties`](crate::Object::properties) of an [`Object`](crate::Object) or
//! [`Association`](crate::Association). It is a structured, `serde`-serializable tree rather
//! than a filter *string*, so it travels over the wire without a parser and maps cleanly to
//! both in-memory [`serde_json`] traversal and (in the SQLite backend) `json_extract`.
//!
//! [`Filter::matches`] is the **reference evaluator**: it defines the exact semantics every
//! backend must reproduce. Backends that push filtering into storage are conformance-tested
//! to return the same results as this evaluator.
//!
//! # Semantics
//!
//! Evaluation is total — it never errors, only matches or doesn't:
//!
//! - A path that does not resolve (a missing object key at any segment) makes the predicate
//!   **not match**; it is never an error.
//! - [`Eq`](CompareOp::Eq) / [`Ne`](CompareOp::Ne) test structural equality across every JSON
//!   type (null, bool, string, number, array, object): `Eq` matches equal values, `Ne` matches
//!   a present-but-unequal value (a value of a different type is unequal, so `Ne` matches it).
//! - For the ordered comparisons ([`Lt`](CompareOp::Lt)/[`Le`](CompareOp::Le)/
//!   [`Gt`](CompareOp::Gt)/[`Ge`](CompareOp::Ge)) a type mismatch — or a value that is not an
//!   orderable scalar — makes the predicate **not match**; it is never an error.
//! - Numbers are compared as [`f64`] (so `42` and `42.0` are equal); strings are compared
//!   lexicographically.
//! - [`Contains`](CompareOp::Contains) is a substring match when the value at the path is a
//!   string, and an element-membership test when it is a JSON array.
//! - [`Exists`](Predicate::Exists) is `true` when the path resolves to any value — **including
//!   JSON `null`** — and `false` when a key along the path is absent.
//! - [`And`](Filter::And) of an empty list is `true` (vacuously); [`Or`](Filter::Or) of an
//!   empty list is `false`.
//!
//! Filtering operates only on the plaintext payload. Sensitive fields (proto
//! `debug_redact = true`) are sealed off the payload by
//! [`ManagedObjectStore`](crate::ManagedObjectStore) and are therefore structurally
//! unsearchable; store-owned `Identifier`/`Managed` fields are injected on read and are
//! likewise absent from the stored, searchable payload.
//!
//! # Examples
//!
//! ```
//! use olai_store::filter::Filter;
//! use serde_json::json;
//!
//! let payload = json!({ "owner": "alice", "size": 42, "tags": ["a", "b"] });
//!
//! // owner == "alice" AND size > 10 AND tags contains "b"
//! let f = Filter::all([
//!     Filter::eq("owner", "alice"),
//!     Filter::gt("size", 10),
//!     Filter::contains("tags", "b"),
//! ]);
//! assert!(f.matches(&payload));
//!
//! // A missing path simply doesn't match — no error.
//! assert!(!Filter::eq("missing", "x").matches(&payload));
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A path to a field within a JSON payload, as a sequence of object-key segments.
///
/// Segments address nested object fields only: `["a", "b"]` selects `payload["a"]["b"]`.
/// Array indexing is intentionally unsupported — use [`CompareOp::Contains`] for array
/// membership. Construct from a dotted string with [`From`] (`"a.b".into()`) or explicitly
/// with [`FieldPath::new`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FieldPath(Vec<String>);

impl FieldPath {
    /// Build a path from an iterator of segments.
    pub fn new<I, S>(segments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        FieldPath(segments.into_iter().map(Into::into).collect())
    }

    /// The path's segments, outermost first.
    pub fn segments(&self) -> &[String] {
        &self.0
    }

    /// Resolve this path against a JSON value, returning the value at the path if every
    /// segment resolves through a JSON object, or `None` if any key is absent.
    ///
    /// A resolved-to `null` returns `Some(&Value::Null)`; only a missing key returns `None`.
    pub fn resolve<'v>(&self, value: &'v Value) -> Option<&'v Value> {
        self.0
            .iter()
            .try_fold(value, |v, seg| v.as_object()?.get(seg))
    }
}

impl From<&str> for FieldPath {
    /// Split a dotted string into segments: `"a.b.c"` → `["a", "b", "c"]`.
    fn from(s: &str) -> Self {
        FieldPath(s.split('.').map(str::to_owned).collect())
    }
}

impl From<String> for FieldPath {
    fn from(s: String) -> Self {
        FieldPath::from(s.as_str())
    }
}

/// A comparison operator for a [`Predicate::Compare`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareOp {
    /// Equal.
    Eq,
    /// Not equal.
    Ne,
    /// Less than.
    Lt,
    /// Less than or equal.
    Le,
    /// Greater than.
    Gt,
    /// Greater than or equal.
    Ge,
    /// Substring (for strings) or element membership (for arrays).
    Contains,
}

/// A leaf test against a single field of the payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Predicate {
    /// Compare the value at `path` against `value` using `op`.
    Compare {
        /// The field to read.
        path: FieldPath,
        /// The comparison to apply.
        op: CompareOp,
        /// The value to compare against.
        value: Value,
    },
    /// Test whether `path` resolves to any value (including JSON `null`).
    Exists {
        /// The field whose presence is tested.
        path: FieldPath,
    },
}

/// A boolean combination of [`Predicate`]s over a JSON payload.
///
/// See the [module documentation](self) for the exact evaluation semantics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Filter {
    /// A single leaf predicate.
    Predicate(Predicate),
    /// Conjunction: matches when every member matches (empty ⇒ `true`).
    And(Vec<Filter>),
    /// Disjunction: matches when any member matches (empty ⇒ `false`).
    Or(Vec<Filter>),
    /// Negation.
    Not(Box<Filter>),
}

impl Filter {
    /// A predicate testing that `path` compares to `value` under `op`.
    pub fn compare(path: impl Into<FieldPath>, op: CompareOp, value: impl Into<Value>) -> Filter {
        Filter::Predicate(Predicate::Compare {
            path: path.into(),
            op,
            value: value.into(),
        })
    }

    /// `path == value`.
    pub fn eq(path: impl Into<FieldPath>, value: impl Into<Value>) -> Filter {
        Filter::compare(path, CompareOp::Eq, value)
    }

    /// `path != value`.
    pub fn ne(path: impl Into<FieldPath>, value: impl Into<Value>) -> Filter {
        Filter::compare(path, CompareOp::Ne, value)
    }

    /// `path < value`.
    pub fn lt(path: impl Into<FieldPath>, value: impl Into<Value>) -> Filter {
        Filter::compare(path, CompareOp::Lt, value)
    }

    /// `path <= value`.
    pub fn le(path: impl Into<FieldPath>, value: impl Into<Value>) -> Filter {
        Filter::compare(path, CompareOp::Le, value)
    }

    /// `path > value`.
    pub fn gt(path: impl Into<FieldPath>, value: impl Into<Value>) -> Filter {
        Filter::compare(path, CompareOp::Gt, value)
    }

    /// `path >= value`.
    pub fn ge(path: impl Into<FieldPath>, value: impl Into<Value>) -> Filter {
        Filter::compare(path, CompareOp::Ge, value)
    }

    /// `path` contains `value` (substring for strings, membership for arrays).
    pub fn contains(path: impl Into<FieldPath>, value: impl Into<Value>) -> Filter {
        Filter::compare(path, CompareOp::Contains, value)
    }

    /// `path` resolves to a value (including JSON `null`).
    pub fn exists(path: impl Into<FieldPath>) -> Filter {
        Filter::Predicate(Predicate::Exists { path: path.into() })
    }

    /// Conjunction of the given filters.
    pub fn all<I: IntoIterator<Item = Filter>>(filters: I) -> Filter {
        Filter::And(filters.into_iter().collect())
    }

    /// Disjunction of the given filters.
    pub fn any<I: IntoIterator<Item = Filter>>(filters: I) -> Filter {
        Filter::Or(filters.into_iter().collect())
    }

    /// Negation of this filter.
    pub fn negate(self) -> Filter {
        Filter::Not(Box::new(self))
    }

    /// Evaluate this filter against a JSON payload — the reference semantics.
    ///
    /// See the [module documentation](self) for the exact rules. Evaluation never errors.
    pub fn matches(&self, payload: &Value) -> bool {
        match self {
            Filter::Predicate(p) => p.matches(payload),
            Filter::And(fs) => fs.iter().all(|f| f.matches(payload)),
            Filter::Or(fs) => fs.iter().any(|f| f.matches(payload)),
            Filter::Not(f) => !f.matches(payload),
        }
    }
}

impl Predicate {
    /// Evaluate this predicate against a JSON payload. Never errors.
    fn matches(&self, payload: &Value) -> bool {
        match self {
            Predicate::Exists { path } => path.resolve(payload).is_some(),
            Predicate::Compare { path, op, value } => match path.resolve(payload) {
                // A missing path never matches.
                None => false,
                Some(found) => compare(found, *op, value),
            },
        }
    }
}

/// Apply `op` between the value `found` at a path and the query `value`.
///
/// Type mismatches yield `false` rather than an error, per the reference semantics.
fn compare(found: &Value, op: CompareOp, value: &Value) -> bool {
    match op {
        CompareOp::Contains => contains(found, value),
        // Equality is structural across every JSON type (null, bool, string, number, array,
        // object), not just the ordered scalars — `ordering` returns `None` for null/array/
        // object, so it must not gate `Eq`/`Ne`.
        CompareOp::Eq => equal(found, value),
        // `Ne` matches only when the field is present (a missing path already returned `false`
        // upstream) and the values differ.
        CompareOp::Ne => !equal(found, value),
        CompareOp::Lt => ordering(found, value) == Some(std::cmp::Ordering::Less),
        CompareOp::Le => matches!(
            ordering(found, value),
            Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
        ),
        CompareOp::Gt => ordering(found, value) == Some(std::cmp::Ordering::Greater),
        CompareOp::Ge => matches!(
            ordering(found, value),
            Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
        ),
    }
}

/// Structural equality between two JSON values, with numbers normalized to [`f64`].
///
/// Unlike [`ordering`], this is total over every JSON type: `null == null`, and arrays and
/// objects compare element-wise via [`serde_json`]'s own equality. Numbers are compared as
/// `f64` so integer and float encodings of the same value (`42` and `42.0`) are equal,
/// matching the ordering-based comparisons.
fn equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) => x == y,
            _ => false,
        },
        _ => a == b,
    }
}

/// Order two JSON values of comparable type, or `None` when they cannot be compared.
///
/// Numbers are compared as [`f64`], strings lexicographically, and booleans as `false < true`.
/// Any other pairing (mismatched types, arrays, objects, null) is incomparable and returns
/// `None` — so the surrounding comparison does not match.
fn ordering(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64()?.partial_cmp(&y.as_f64()?),
        (Value::String(x), Value::String(y)) => Some(x.as_str().cmp(y.as_str())),
        (Value::Bool(x), Value::Bool(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

/// `Contains`: substring for strings, element membership for arrays; `false` otherwise.
fn contains(found: &Value, value: &Value) -> bool {
    match found {
        Value::String(haystack) => value
            .as_str()
            .is_some_and(|needle| haystack.contains(needle)),
        Value::Array(items) => items.iter().any(|item| equal(item, value)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_path_never_matches() {
        let p = json!({ "a": 1 });
        assert!(!Filter::eq("b", 1).matches(&p));
        assert!(!Filter::eq("a.b", 1).matches(&p)); // `a` is not an object
        assert!(
            !Filter::ne("b", 1).matches(&p),
            "Ne on a missing path is false"
        );
        assert!(!Filter::gt("b", 0).matches(&p));
    }

    #[test]
    fn type_mismatch_is_false_not_error() {
        let p = json!({ "n": 5, "s": "x" });
        assert!(!Filter::eq("n", "5").matches(&p), "number vs string");
        assert!(!Filter::gt("s", 1).matches(&p), "string vs number");
        assert!(!Filter::lt("n", "z").matches(&p));
        // A present field of a different type is unequal, so `Ne` matches it.
        assert!(Filter::ne("n", "5").matches(&p), "different type is Ne");
    }

    #[test]
    fn numbers_compare_as_f64() {
        let p = json!({ "n": 42 });
        assert!(Filter::eq("n", 42).matches(&p));
        assert!(Filter::eq("n", 42.0).matches(&p));
        assert!(Filter::gt("n", 41.9).matches(&p));
        assert!(Filter::le("n", 42).matches(&p));
        assert!(!Filter::lt("n", 42).matches(&p));
    }

    #[test]
    fn eq_ne_over_all_json_types() {
        // Equality is structural, not restricted to the ordered scalars.
        let p = json!({ "nil": null, "arr": ["a", "b"], "obj": { "k": 1 }, "flag": true });
        assert!(Filter::eq("nil", Value::Null).matches(&p), "null == null");
        assert!(
            Filter::eq("arr", json!(["a", "b"])).matches(&p),
            "array equality"
        );
        assert!(!Filter::eq("arr", json!(["a"])).matches(&p));
        assert!(
            Filter::eq("obj", json!({ "k": 1 })).matches(&p),
            "object equality"
        );
        assert!(Filter::eq("flag", true).matches(&p));

        // Ne is the negation of Eq on a present field.
        assert!(
            Filter::ne("arr", json!(["x"])).matches(&p),
            "differing array is Ne"
        );
        assert!(!Filter::ne("arr", json!(["a", "b"])).matches(&p));
        assert!(!Filter::ne("nil", Value::Null).matches(&p));
        // Numeric equality still normalizes int/float encodings.
        assert!(Filter::eq("obj.k", 1.0).matches(&p));
    }

    #[test]
    fn contains_number_membership_normalizes() {
        // Integer-encoded array element matches a float query and vice versa.
        let p = json!({ "nums": [1, 2, 3] });
        assert!(Filter::contains("nums", 3.0).matches(&p));
        assert!(Filter::contains("nums", 2).matches(&p));
        assert!(!Filter::contains("nums", 4).matches(&p));
    }

    #[test]
    fn strings_compare_lexicographically() {
        let p = json!({ "s": "banana" });
        assert!(Filter::gt("s", "apple").matches(&p));
        assert!(Filter::lt("s", "cherry").matches(&p));
        assert!(Filter::ge("s", "banana").matches(&p));
    }

    #[test]
    fn contains_substring_and_array_membership() {
        let p = json!({ "s": "hello world", "arr": [1, "two", 3] });
        assert!(Filter::contains("s", "o wo").matches(&p));
        assert!(!Filter::contains("s", "xyz").matches(&p));
        assert!(Filter::contains("arr", "two").matches(&p));
        assert!(Filter::contains("arr", 3).matches(&p));
        assert!(!Filter::contains("arr", 9).matches(&p));
        // Contains against a non-string/non-array value is false.
        assert!(!Filter::contains("s", 1).matches(&json!({ "s": 5 })));
    }

    #[test]
    fn exists_treats_present_null_as_existing() {
        let p = json!({ "present": null, "nested": { "x": 1 } });
        assert!(
            Filter::exists("present").matches(&p),
            "present-but-null exists"
        );
        assert!(Filter::exists("nested.x").matches(&p));
        assert!(!Filter::exists("absent").matches(&p));
        assert!(!Filter::exists("nested.y").matches(&p));
    }

    #[test]
    fn empty_and_or_identities() {
        let p = json!({});
        assert!(Filter::all([]).matches(&p), "empty And is vacuously true");
        assert!(!Filter::any([]).matches(&p), "empty Or is false");
    }

    #[test]
    fn boolean_composition() {
        let p = json!({ "a": 1, "b": "x" });
        let f = Filter::all([Filter::eq("a", 1), Filter::eq("b", "x")]);
        assert!(f.matches(&p));
        assert!(Filter::any([Filter::eq("a", 2), Filter::eq("b", "x")]).matches(&p));
        assert!(Filter::eq("a", 2).negate().matches(&p));
        assert!(!Filter::eq("a", 1).negate().matches(&p));
    }

    #[test]
    fn serde_roundtrips() {
        let f = Filter::all([
            Filter::eq("owner", "alice"),
            Filter::gt("size", 10),
            Filter::any([Filter::contains("tags", "b"), Filter::exists("archived")]),
            Filter::ne("state", "deleted").negate(),
        ]);
        let s = serde_json::to_string(&f).unwrap();
        let back: Filter = serde_json::from_str(&s).unwrap();
        assert_eq!(f, back);
    }
}
