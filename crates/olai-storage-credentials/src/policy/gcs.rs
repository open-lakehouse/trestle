//! GCS Credential Access Boundaries.
//!
//! A downscoped GCS token is minted by exchanging a source token at Google's STS
//! endpoint together with a Credential Access Boundary: rules that can only narrow the
//! source token's permissions. [`access_boundary`] restricts a token to one bucket,
//! and below the bucket root to one prefix, through a CEL condition that admits
//!
//! - objects at or below the prefix (`resource.name`), and
//! - list calls whose prefix starts at the prefix (`objectListPrefix`), so a holder
//!   cannot enumerate the rest of the bucket.

use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::location::{StorageProvider, StorageScope};
use crate::model::AccessLevel;

/// The predefined role a boundary makes available for `access`.
///
/// - [`AccessLevel::Read`]: `roles/storage.objectViewer` (get and list objects).
/// - [`AccessLevel::ReadWrite`]: `roles/storage.objectAdmin` (also create, overwrite
///   and delete). The boundary only narrows: the source credential must hold these
///   permissions itself.
pub fn boundary_role(access: AccessLevel) -> &'static str {
    match access {
        AccessLevel::Read => "roles/storage.objectViewer",
        AccessLevel::ReadWrite => "roles/storage.objectAdmin",
    }
}

/// The Credential Access Boundary restricting a token to `scope`.
///
/// The result is the `options` document of the token exchange
/// (`{"accessBoundary": {"accessBoundaryRules": [...]}}`); the minter URL-encodes it
/// into the request.
///
/// Fails if `scope` is not a GCS location, or its path contains control characters.
pub fn access_boundary(scope: &StorageScope, access: AccessLevel) -> Result<Value> {
    let location = scope.location();
    let invalid = |reason: &str| Error::invalid_location(location.to_string(), reason);
    if location.provider() != StorageProvider::Gcp {
        return Err(invalid("access boundaries apply to gs:// scopes only"));
    }
    let bucket = location.bucket();
    let prefix = location.path().as_ref();

    let mut rule = json!({
        "availableResource": format!("//storage.googleapis.com/projects/_/buckets/{bucket}"),
        "availablePermissions": [format!("inRole:{}", boundary_role(access))],
    });
    if !prefix.is_empty() {
        let object = cel_string(&format!("projects/_/buckets/{bucket}/objects/{prefix}"))
            .ok_or_else(|| invalid("paths containing control characters are not supported"))?;
        let below = cel_string(&format!("projects/_/buckets/{bucket}/objects/{prefix}/"))
            .ok_or_else(|| invalid("paths containing control characters are not supported"))?;
        let list = cel_string(&format!("{prefix}/"))
            .ok_or_else(|| invalid("paths containing control characters are not supported"))?;
        let expression = format!(
            "resource.name == {object} || resource.name.startsWith({below}) || \
             api.getAttribute('storage.googleapis.com/objectListPrefix', '').startsWith({list})"
        );
        rule["availabilityCondition"] = json!({
            "title": "storage-scope",
            "expression": expression,
        });
    }
    Ok(json!({ "accessBoundary": { "accessBoundaryRules": [rule] } }))
}

/// `s` as a single-quoted CEL string literal, or `None` if it contains control
/// characters (which no supported object name needs).
fn cel_string(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            c if c.is_control() => return None,
            c => out.push(c),
        }
    }
    out.push('\'');
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boundary(scope: &str, access: AccessLevel) -> Value {
        access_boundary(&scope.parse().unwrap(), access).unwrap()
    }

    #[test]
    fn prefix_boundary() {
        assert_eq!(
            boundary("gs://bucket/warehouse/tab/", AccessLevel::Read),
            json!({
                "accessBoundary": {
                    "accessBoundaryRules": [{
                        "availableResource": "//storage.googleapis.com/projects/_/buckets/bucket",
                        "availablePermissions": ["inRole:roles/storage.objectViewer"],
                        "availabilityCondition": {
                            "title": "storage-scope",
                            "expression": "resource.name == 'projects/_/buckets/bucket/objects/warehouse/tab' || resource.name.startsWith('projects/_/buckets/bucket/objects/warehouse/tab/') || api.getAttribute('storage.googleapis.com/objectListPrefix', '').startsWith('warehouse/tab/')",
                        },
                    }],
                },
            })
        );
    }

    #[test]
    fn bucket_boundary_has_no_condition() {
        let b = boundary("gs://bucket", AccessLevel::ReadWrite);
        let rule = &b["accessBoundary"]["accessBoundaryRules"][0];
        assert_eq!(
            rule["availablePermissions"],
            json!(["inRole:roles/storage.objectAdmin"])
        );
        assert!(rule.get("availabilityCondition").is_none());
    }

    #[test]
    fn crafted_paths_stay_inside_cel_strings() {
        // `'` is legal in object names; unescaped it would end the literal and let the
        // rest of the name append `|| true`.
        let b = boundary(
            "gs://bucket/x'%20%7C%7C%20true%20%7C%7C%20'",
            AccessLevel::Read,
        );
        let expr =
            b["accessBoundary"]["accessBoundaryRules"][0]["availabilityCondition"]["expression"]
                .as_str()
                .unwrap()
                .to_owned();
        assert!(expr.contains(r"objects/x\' || true || \''"), "{expr}");
        assert_eq!(cel_string(r"a\b'c").as_deref(), Some(r"'a\\b\'c'"));
        assert_eq!(cel_string("a\nb"), None);
    }

    #[test]
    fn rejects_other_providers() {
        let s3: StorageScope = "s3://bucket/tab".parse().unwrap();
        assert!(access_boundary(&s3, AccessLevel::Read).is_err());
    }
}
