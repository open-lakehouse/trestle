//! Resource-name validation: the names a plan provisions are interpolated verbatim into the
//! providers' init jobs (Postgres `CREATE DATABASE` SQL, bucket/container creation shell), so
//! the planner — not each front end — is where an unsafe name is rejected.
//!
//! The rules are per resource role and deliberately conservative: each is the intersection of
//! what the role's baseline providers accept, and every allowed character is inert in both SQL
//! identifiers and shell words.

use super::PlanError;
use crate::model::role::Role;

/// The longest resource name any role accepts (Postgres identifiers and S3/Azure bucket names
/// both cap at 63).
const MAX_LEN: usize = 63;

/// A human-readable description of the names `role` accepts, as shown in
/// [`PlanError::InvalidResourceName`].
pub fn resource_name_rule(role: &str) -> &'static str {
    match role {
        Role::RELATIONAL_DB => "1–63 lowercase letters, digits and _, starting with a letter or _",
        Role::OBJECT_STORE => {
            "3–63 lowercase letters, digits and -, starting and ending alphanumeric, \
             with no consecutive -"
        }
        _ => "1–63 lowercase letters, digits, _ and -, starting alphanumeric",
    }
}

/// Whether `name` is a valid resource name for `role` (see [`resource_name_rule`]).
pub fn is_valid_resource_name(role: &str, name: &str) -> bool {
    let bytes = name.as_bytes();
    let (Some(&first), Some(&last)) = (bytes.first(), bytes.last()) else {
        return false;
    };
    if bytes.len() > MAX_LEN {
        return false;
    }
    let lower_alnum = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    match role {
        Role::RELATIONAL_DB => {
            (first.is_ascii_lowercase() || first == b'_')
                && bytes.iter().all(|&b| lower_alnum(b) || b == b'_')
        }
        Role::OBJECT_STORE => {
            bytes.len() >= 3
                && lower_alnum(first)
                && lower_alnum(last)
                && bytes.iter().all(|&b| lower_alnum(b) || b == b'-')
                && !name.contains("--")
        }
        _ => {
            lower_alnum(first)
                && bytes
                    .iter()
                    .all(|&b| lower_alnum(b) || b == b'_' || b == b'-')
        }
    }
}

/// Reject `name` for `role` with [`PlanError::InvalidResourceName`] unless it is valid.
pub(super) fn check_resource_name(role: &str, name: &str) -> Result<(), PlanError> {
    if is_valid_resource_name(role, name) {
        Ok(())
    } else {
        Err(PlanError::InvalidResourceName {
            resource: role.to_string(),
            name: name.to_string(),
            rule: resource_name_rule(role),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relational_db_names() {
        for ok in ["mlflow", "unitycatalog", "_scratch", "a", "db_2"] {
            assert!(is_valid_resource_name(Role::RELATIONAL_DB, ok), "{ok}");
        }
        for bad in [
            "",
            "2db",
            "Analytics",
            "my-db",
            "a'; DROP DATABASE x; --",
            &"a".repeat(64),
        ] {
            assert!(!is_valid_resource_name(Role::RELATIONAL_DB, bad), "{bad}");
        }
    }

    #[test]
    fn object_store_names() {
        for ok in ["unity", "mlflow", "raw-data", "a1b", &"b".repeat(63)] {
            assert!(is_valid_resource_name(Role::OBJECT_STORE, ok), "{ok}");
        }
        for bad in [
            "ab",
            "-raw",
            "raw-",
            "raw--data",
            "Raw",
            "x; rm -rf /",
            "a_b",
            "$(id)",
        ] {
            assert!(!is_valid_resource_name(Role::OBJECT_STORE, bad), "{bad}");
        }
    }

    #[test]
    fn other_roles_use_the_fallback() {
        assert!(is_valid_resource_name("queue", "jobs-v2_a"));
        assert!(!is_valid_resource_name("queue", "_jobs"));
        assert!(!is_valid_resource_name("queue", "jobs queue"));
    }

    #[test]
    fn error_carries_the_rule() {
        let err = check_resource_name(Role::OBJECT_STORE, "Bad").unwrap_err();
        assert_eq!(
            err,
            PlanError::InvalidResourceName {
                resource: Role::OBJECT_STORE.into(),
                name: "Bad".into(),
                rule: resource_name_rule(Role::OBJECT_STORE),
            }
        );
    }
}
