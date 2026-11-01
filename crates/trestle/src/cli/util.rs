//! Helpers shared by the `trestle new` and `trestle env new` commands.

use std::path::Path;

use crate::error::{Error, Result};

/// Check that `name` matches `^[a-z][a-z0-9-]*$`, the shape both commands use for
/// project and environment names. The name becomes an output directory, so this
/// rejects path-traversal names like `../foo` before anything touches the disk.
/// `label` is the variable the error reports.
pub(super) fn validate_name(label: &str, name: &str) -> Result<()> {
    let mut chars = name.chars();
    let valid = matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if valid {
        Ok(())
    } else {
        Err(Error::InvalidVariable {
            name: label.to_string(),
            reason: format!(
                "`{name}` must match `^[a-z][a-z0-9-]*$` (lowercase letters, digits, and dashes; \
                 must start with a letter)"
            ),
        })
    }
}

/// Parse a `key=value[,value...]` flag value into its key and non-empty values.
pub(super) fn parse_selection(s: &str) -> std::result::Result<(String, Vec<String>), String> {
    let (key, vals) = s
        .split_once('=')
        .ok_or_else(|| format!("expected key=value[,value], got `{s}`"))?;
    let values: Vec<String> = vals
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(String::from)
        .collect();
    Ok((key.to_string(), values))
}

/// Create `dir` and its parents, refusing a non-empty `dir` unless `force` is set.
pub(super) fn ensure_writable_dir(dir: &Path, force: bool) -> Result<()> {
    if dir.exists() {
        let mut entries = dir.read_dir().map_err(|e| Error::io_at(dir, e))?;
        if entries.next().is_some() && !force {
            return Err(Error::OutputExists(dir.to_path_buf()));
        }
    }
    std::fs::create_dir_all(dir).map_err(|e| Error::io_at(dir, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_selection_splits_and_trims_values() {
        assert_eq!(
            parse_selection("storage=a, b ,c").unwrap(),
            (
                "storage".to_string(),
                vec!["a".to_string(), "b".to_string(), "c".to_string()]
            )
        );
    }

    #[test]
    fn parse_selection_drops_empty_values() {
        // Trailing comma / empty segments are filtered, yielding an empty pick.
        assert_eq!(
            parse_selection("storage=").unwrap(),
            ("storage".to_string(), Vec::<String>::new())
        );
        assert_eq!(
            parse_selection("storage=a,,").unwrap(),
            ("storage".to_string(), vec!["a".to_string()])
        );
    }

    #[test]
    fn parse_selection_rejects_missing_equals() {
        assert!(parse_selection("storage").is_err());
    }

    #[test]
    fn validate_name_accepts_kebab() {
        assert!(validate_name("project_name", "my-app-2").is_ok());
        assert!(validate_name("project_name", "a").is_ok());
    }

    #[test]
    fn validate_name_rejects_traversal_and_bad_chars() {
        for bad in ["../foo", "/abs", "Foo", "1app", "my_app", "app!", ""] {
            assert!(
                validate_name("project_name", bad).is_err(),
                "should reject {bad:?}"
            );
        }
    }

    #[test]
    fn ensure_writable_dir_creates_missing_and_guards_non_empty() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("app");
        ensure_writable_dir(&dir, false).unwrap();
        assert!(dir.is_dir());

        std::fs::write(dir.join("keep.txt"), "x").unwrap();
        assert!(matches!(
            ensure_writable_dir(&dir, false),
            Err(Error::OutputExists(_))
        ));
        ensure_writable_dir(&dir, true).unwrap();
    }
}
