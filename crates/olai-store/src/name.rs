//! Hierarchical resource names and their dotted, backtick-escaped string form.
//!
//! A [`ResourceName`] is a path of field-name segments (`["catalog", "schema", "table"]`)
//! identifying a resource within a taxonomy's hierarchy. It round-trips to and from a dotted
//! string via [`Display`] and [`FromStr`](std::str::FromStr): segments join with `.`, and a segment containing a
//! dot, whitespace, other non-alphanumeric character, or a leading digit is wrapped in backticks
//! (`` ` ``) with any literal backtick doubled — the same quoting rule SQL uses for identifiers.
//! [`parse_column_name_list`](ResourceName::parse_column_name_list) parses a comma-separated
//! list of such names.

use crate::{Error, Result};

use std::borrow::Borrow;
use std::fmt::{Display, Formatter};
use std::hash::{Hash, Hasher};
use std::iter::Peekable;
use std::ops::Deref;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

/// A shared, empty [`ResourceName`] (a zero-segment path).
///
/// Useful as a default or as the root of a hierarchy without allocating a fresh empty name.
pub static EMPTY_RESOURCE_NAME: LazyLock<ResourceName> =
    LazyLock::new(|| ResourceName::new(&[] as &[String]));

/// A (possibly nested) resource name: a path of field-name segments.
///
/// Segments are ordered outermost-first (`["catalog", "schema", "table"]`). The name derefs to
/// `&[String]`, and [`Display`]/[`FromStr`](std::str::FromStr) convert to and from the dotted, backtick-escaped
/// string form described in the [module docs](self).
///
/// # Examples
///
/// ```
/// use olai_store::ResourceName;
///
/// let name: ResourceName = "catalog.schema.table".parse().unwrap();
/// assert_eq!(name.path(), ["catalog", "schema", "table"]);
/// assert_eq!(name.to_string(), "catalog.schema.table");
///
/// // A segment needing quoting round-trips through backticks.
/// let quoted = ResourceName::new(["my catalog", "t.1"]);
/// assert_eq!(quoted.to_string(), "`my catalog`.`t.1`");
/// assert_eq!(quoted.to_string().parse::<ResourceName>().unwrap(), quoted);
/// ```
#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "sqlx", derive(sqlx::Type))]
#[cfg_attr(feature = "sqlx", sqlx(transparent, no_pg_array))]
pub struct ResourceName(Vec<String>);

impl ResourceName {
    /// Creates a new resource name from an iterator of field names.
    pub fn new<A>(iter: impl IntoIterator<Item = A>) -> Self
    where
        Self: FromIterator<A>,
    {
        iter.into_iter().collect()
    }

    /// Naively splits a string at dots to create a resource name.
    ///
    /// This method is _NOT_ recommended for production use, as it does not attempt to interpret
    /// special characters in field names.
    pub fn from_naive_str_split(name: impl AsRef<str>) -> Self {
        Self::new(name.as_ref().split(FIELD_SEPARATOR))
    }

    /// Parses a comma-separated list of resource names, properly accounting for escapes and special
    /// characters.
    ///
    /// Surrounding whitespace around each name is ignored; an empty input yields an empty list.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Generic`] if any name is malformed — for example an
    /// unterminated backtick quote or an invalid character following a segment.
    ///
    /// [`Error::Generic`]: crate::Error::Generic
    pub fn parse_column_name_list(names: impl AsRef<str>) -> Result<Vec<ResourceName>> {
        let names = names.as_ref();
        let chars = &mut names.chars().peekable();

        drop_leading_whitespace(chars);
        let mut ending = match chars.peek() {
            Some(_) => FieldEnding::NextColumn,
            None => FieldEnding::InputExhausted,
        };

        let mut cols = vec![];
        while ending == FieldEnding::NextColumn {
            let (col, new_ending) = parse_resource_name(chars)?;
            cols.push(col);
            ending = new_ending;
        }
        Ok(cols)
    }

    /// Joins this name with another, concatenating their fields into a single nested path.
    pub fn join(&self, right: &ResourceName) -> ResourceName {
        [self.clone(), right.clone()].into_iter().collect()
    }

    /// The path of field names for this resource name.
    pub fn path(&self) -> &[String] {
        &self.0
    }

    /// Consumes this resource name and returns the path of field names.
    pub fn into_inner(self) -> Vec<String> {
        self.0
    }

    /// Returns `true` if this name starts with the given prefix.
    ///
    /// The comparison is segment-wise, not by string prefix: a name matches `prefix` when its
    /// first `prefix.len()` segments equal `prefix`'s segments. An empty prefix matches every name.
    /// Used to scope listings to a namespace subtree.
    ///
    /// # Examples
    ///
    /// ```
    /// use olai_store::ResourceName;
    ///
    /// let name = ResourceName::new(["catalog", "schema", "table"]);
    /// assert!(name.prefix_matches(&ResourceName::new(["catalog", "schema"])));
    /// assert!(!name.prefix_matches(&ResourceName::new(["catalog", "other"])));
    /// ```
    ///
    /// Segments are compared case-insensitively for ASCII, matching the
    /// `COLLATE NOCASE` folding the SQLite backend applies to the `name` column
    /// (so namespace prefix listing agrees with `get_by_name` equality). Case
    /// folding is ASCII-only; non-ASCII segments compare byte-for-byte.
    ///
    /// ```
    /// use olai_store::ResourceName;
    ///
    /// let name = ResourceName::new(["Catalog", "Schema", "table"]);
    /// assert!(name.prefix_matches(&ResourceName::new(["catalog", "schema"])));
    /// ```
    pub fn prefix_matches(&self, prefix: &ResourceName) -> bool {
        if self.len() < prefix.len() {
            return false;
        }
        prefix
            .iter()
            .zip(self.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    }

    /// Compare two resource names for equality, folding ASCII case per segment.
    ///
    /// This mirrors the `COLLATE NOCASE` folding the SQLite backend applies to
    /// the `name` column, so in-memory name lookups agree with the persistent
    /// backend: `Catalog` and `catalog` are the same name. Case folding is
    /// ASCII-only; non-ASCII segments compare byte-for-byte.
    ///
    /// # Examples
    ///
    /// ```
    /// use olai_store::ResourceName;
    ///
    /// assert!(ResourceName::new(["Catalog"]).eq_ignore_ascii_case(&ResourceName::new(["catalog"])));
    /// assert!(!ResourceName::new(["a", "b"]).eq_ignore_ascii_case(&ResourceName::new(["a"])));
    /// ```
    pub fn eq_ignore_ascii_case(&self, other: &ResourceName) -> bool {
        self.len() == other.len()
            && self
                .iter()
                .zip(other.iter())
                .all(|(a, b)| a.eq_ignore_ascii_case(b))
    }
}

impl<A: Into<String>> FromIterator<A> for ResourceName {
    fn from_iter<T: IntoIterator<Item = A>>(iter: T) -> Self {
        let path = iter.into_iter().map(|s| s.into()).collect();
        Self(path)
    }
}

impl From<Vec<String>> for ResourceName {
    fn from(path: Vec<String>) -> Self {
        Self(path)
    }
}

impl FromIterator<ResourceName> for ResourceName {
    fn from_iter<T: IntoIterator<Item = ResourceName>>(iter: T) -> Self {
        let path = iter.into_iter().flat_map(|c| c.into_iter()).collect();
        Self(path)
    }
}

impl IntoIterator for ResourceName {
    type Item = String;
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl Deref for ResourceName {
    type Target = [String];

    fn deref(&self) -> &[String] {
        &self.0
    }
}

impl Borrow<[String]> for ResourceName {
    fn borrow(&self) -> &[String] {
        self
    }
}

impl Borrow<[String]> for &ResourceName {
    fn borrow(&self) -> &[String] {
        self
    }
}

impl Hash for ResourceName {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        (**self).hash(hasher)
    }
}

impl Display for ResourceName {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for (i, s) in self.iter().enumerate() {
            use std::fmt::Write as _;

            if i > 0 {
                f.write_char(FIELD_SEPARATOR)?;
            }

            let digit_char = |c: char| c.is_ascii_digit();
            if s.is_empty() || s.starts_with(digit_char) || s.contains(|c| !is_simple_char(c)) {
                f.write_char(FIELD_ESCAPE_CHAR)?;
                for c in s.chars() {
                    f.write_char(c)?;
                    if c == FIELD_ESCAPE_CHAR {
                        f.write_char(c)?;
                    }
                }
                f.write_char(FIELD_ESCAPE_CHAR)?;
            } else {
                f.write_str(s)?;
            }
        }
        Ok(())
    }
}

fn is_simple_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn drop_leading_whitespace(iter: &mut Peekable<impl Iterator<Item = char>>) {
    while iter.next_if(|c| c.is_whitespace()).is_some() {}
}

impl std::str::FromStr for ResourceName {
    type Err = Error;

    /// Parses the dotted, backtick-escaped string form (see the [module docs](self)).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Generic`] if the string is malformed — an
    /// unterminated backtick quote, an invalid character after a segment, or a trailing comma
    /// (parse a comma-separated list with
    /// [`parse_column_name_list`](ResourceName::parse_column_name_list) instead).
    ///
    /// [`Error::Generic`]: crate::Error::Generic
    fn from_str(s: &str) -> Result<Self> {
        match parse_resource_name(&mut s.chars().peekable())? {
            (_, FieldEnding::NextColumn) => Err(Error::generic("Trailing comma in column name")),
            (col, _) => Ok(col),
        }
    }
}

type Chars<'a> = Peekable<std::str::Chars<'a>>;

#[derive(PartialEq)]
enum FieldEnding {
    InputExhausted,
    NextField,
    NextColumn,
}

const FIELD_ESCAPE_CHAR: char = '`';
const FIELD_SEPARATOR: char = '.';
const COLUMN_SEPARATOR: char = ',';

fn parse_resource_name(chars: &mut Chars<'_>) -> Result<(ResourceName, FieldEnding)> {
    drop_leading_whitespace(chars);
    let mut ending = if chars.peek().is_none() {
        FieldEnding::InputExhausted
    } else if chars.next_if_eq(&COLUMN_SEPARATOR).is_some() {
        FieldEnding::NextColumn
    } else {
        FieldEnding::NextField
    };

    let mut path = vec![];
    while ending == FieldEnding::NextField {
        drop_leading_whitespace(chars);
        let field_name = match chars.next_if_eq(&FIELD_ESCAPE_CHAR) {
            Some(_) => parse_escaped_field_name(chars)?,
            None => parse_simple_field_name(chars)?,
        };

        ending = match chars.find(|c| !c.is_whitespace()) {
            None => FieldEnding::InputExhausted,
            Some(FIELD_SEPARATOR) => FieldEnding::NextField,
            Some(COLUMN_SEPARATOR) => FieldEnding::NextColumn,
            Some(other) => {
                return Err(Error::generic(format!(
                    "Invalid character {other:?} after field {field_name:?}",
                )));
            }
        };
        path.push(field_name);
    }
    Ok((ResourceName::new(path), ending))
}

fn parse_simple_field_name(chars: &mut Chars<'_>) -> Result<String> {
    let mut name = String::new();
    let mut first = true;
    while let Some(c) = chars.next_if(|c| is_simple_char(*c)) {
        if first && c.is_ascii_digit() {
            return Err(Error::generic(format!(
                "Unescaped field name cannot start with a digit {c:?}"
            )));
        }
        name.push(c);
        first = false;
    }
    Ok(name)
}

fn parse_escaped_field_name(chars: &mut Chars<'_>) -> Result<String> {
    let mut name = String::new();
    loop {
        match chars.next() {
            Some(FIELD_ESCAPE_CHAR) if chars.next_if_eq(&FIELD_ESCAPE_CHAR).is_none() => break,
            Some(c) => name.push(c),
            None => {
                return Err(Error::generic(format!(
                    "No closing {FIELD_ESCAPE_CHAR:?} after field {name:?}"
                )));
            }
        }
    }
    Ok(name)
}

#[cfg(test)]
mod test {
    use super::*;

    impl ResourceName {
        fn empty() -> Self {
            Self::new(&[] as &[String])
        }
    }

    #[test]
    fn test_column_name_methods() {
        let simple: ResourceName = "x".parse().unwrap();
        let nested: ResourceName = "x.y".parse().unwrap();

        assert_eq!(simple.path(), ["x"]);
        assert_eq!(nested.path(), ["x", "y"]);

        assert_eq!(simple.clone().into_inner(), ["x"]);
        assert_eq!(nested.clone().into_inner(), ["x", "y"]);

        let name: &[String] = &nested;
        assert_eq!(name, &["x", "y"]);

        let name: ResourceName = ["x", "y"].into_iter().collect();
        assert_eq!(name, nested);

        let name: ResourceName = [&nested, &simple].into_iter().cloned().collect();
        assert_eq!(name, ResourceName::new(["x", "y", "x"]));
    }

    #[test]
    fn test_column_name_from_str() {
        let cases = [
            ("", Some(ResourceName::empty())),
            (".", Some(ResourceName::new(["", ""]))),
            ("  .  ", Some(ResourceName::new(["", ""]))),
            (" ", Some(ResourceName::empty())),
            ("0", None),
            (".a", Some(ResourceName::new(["", "a"]))),
            ("a.", Some(ResourceName::new(["a", ""]))),
            ("  a  .  ", Some(ResourceName::new(["a", ""]))),
            ("a..b", Some(ResourceName::new(["a", "", "b"]))),
            ("`a", None),
            ("a`", None),
            ("a`b`", None),
            ("`a`b", None),
            ("`a``b`", Some(ResourceName::new(["a`b"]))),
            ("  `a``b`  ", Some(ResourceName::new(["a`b"]))),
            ("`a`` b`", Some(ResourceName::new(["a` b"]))),
            ("a", Some(ResourceName::new(["a"]))),
            ("a0", Some(ResourceName::new(["a0"]))),
            ("`a`", Some(ResourceName::new(["a"]))),
            ("  `a`  ", Some(ResourceName::new(["a"]))),
            ("` `", Some(ResourceName::new([" "]))),
            ("  ` `  ", Some(ResourceName::new([" "]))),
            ("`0`", Some(ResourceName::new(["0"]))),
            ("`.`", Some(ResourceName::new(["."]))),
            ("`.`.`.`", Some(ResourceName::new([".", "."]))),
            ("` `.` `", Some(ResourceName::new([" ", " "]))),
            ("a.b", Some(ResourceName::new(["a", "b"]))),
            ("a b", None),
            ("a.`b`", Some(ResourceName::new(["a", "b"]))),
            ("`a`.b", Some(ResourceName::new(["a", "b"]))),
            ("`a`.`b`", Some(ResourceName::new(["a", "b"]))),
            ("`a`.`b`.`c`", Some(ResourceName::new(["a", "b", "c"]))),
            ("`a``.`b```", None),
            ("`a```.`b``", None),
            ("`a```.`b```", Some(ResourceName::new(["a`", "b`"]))),
            ("`a.`b``.c`", None),
            ("`a.``b`.c`", None),
            ("`a.``b``.c`", Some(ResourceName::new(["a.`b`.c"]))),
            ("a`.b``", None),
        ];
        for (input, expected_output) in cases {
            let output: Result<ResourceName> = input.parse();
            match (&output, &expected_output) {
                (Ok(output), Some(expected_output)) => {
                    assert_eq!(output, expected_output, "from {input}")
                }
                (Err(_), None) => {}
                _ => panic!("Expected {input} to parse as {expected_output:?}, got {output:?}"),
            }
        }
    }

    #[test]
    fn test_column_name_to_string() {
        let cases = [
            ("", ResourceName::empty()),
            ("``.``", ResourceName::new(["", ""])),
            ("``.a", ResourceName::new(["", "a"])),
            ("a.``", ResourceName::new(["a", ""])),
            ("a.``.b", ResourceName::new(["a", "", "b"])),
            ("a", ResourceName::new(["a"])),
            ("a0", ResourceName::new(["a0"])),
            ("`a `", ResourceName::new(["a "])),
            ("` `", ResourceName::new([" "])),
            ("`0`", ResourceName::new(["0"])),
            ("`.`", ResourceName::new(["."])),
            ("`.`.`.`", ResourceName::new([".", "."])),
            ("` `.` `", ResourceName::new([" ", " "])),
            ("a.b", ResourceName::new(["a", "b"])),
            ("a.b.c", ResourceName::new(["a", "b", "c"])),
            ("a.`b.c`.d", ResourceName::new(["a", "b.c", "d"])),
            ("`a```.`b```", ResourceName::new(["a`", "b`"])),
        ];
        for (expected_output, input) in cases {
            let output = input.to_string();
            assert_eq!(output, expected_output);

            let parsed: ResourceName = output.parse().expect(&output);
            assert_eq!(parsed, input);
        }

        let cases = [
            ("  `a`  ", "a", ResourceName::new(["a"])),
            ("  `a0`  ", "a0", ResourceName::new(["a0"])),
            ("  `a`  .  `b`  ", "a.b", ResourceName::new(["a", "b"])),
        ];
        for (input, expected_output, expected_parsed) in cases {
            let parsed: ResourceName = input.parse().unwrap();
            assert_eq!(parsed, expected_parsed);
            assert_eq!(parsed.to_string(), expected_output);
        }
    }

    #[test]
    fn test_parse_column_name_list() {
        let cases = [
            ("", Some(vec![])),
            (
                "  ,  ",
                Some(vec![ResourceName::empty(), ResourceName::empty()]),
            ),
            ("  a  ", Some(vec![ResourceName::new(["a"])])),
            (
                "  ,  a  ",
                Some(vec![ResourceName::empty(), ResourceName::new(["a"])]),
            ),
            (
                "  a  ,  ",
                Some(vec![ResourceName::new(["a"]), ResourceName::empty()]),
            ),
            (
                "a  ,  b",
                Some(vec![ResourceName::new(["a"]), ResourceName::new(["b"])]),
            ),
            ("`a, b`", Some(vec![ResourceName::new(["a, b"])])),
            (
                "a.b, c",
                Some(vec![
                    ResourceName::new(["a", "b"]),
                    ResourceName::new(["c"]),
                ]),
            ),
        ];
        for (input, expected_output) in cases {
            let output = ResourceName::parse_column_name_list(input);
            match (&output, &expected_output) {
                (Ok(output), Some(expected_output)) => {
                    assert_eq!(output, expected_output, "from \"{input}\"")
                }
                (Err(_), None) => {}
                _ => panic!("Expected {input} to parse as {expected_output:?}, got {output:?}"),
            }
        }
    }
}
