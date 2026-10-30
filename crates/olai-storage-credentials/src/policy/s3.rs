//! S3 session policies for STS `AssumeRole`.
//!
//! A session policy is intersected with the assumed role's own policies, so it can only
//! narrow what the role allows. [`session_policy`] narrows to one [`StorageScope`]:
//!
//! - object actions on the scope's exact key and everything below it
//!   (`arn:aws:s3:::bucket/prefix` and `.../prefix/*`);
//! - `s3:ListBucket` on the bucket, conditioned on `s3:prefix` being below the scope's
//!   prefix (`prefix/...`), so a holder cannot enumerate the rest of the bucket, sibling
//!   keys that merely start with the same string included;
//! - `s3:GetBucketLocation`, which clients use for region discovery.
//!
//! [`AccessLevel::ReadWrite`] adds writes and deletes, plus the multipart actions
//! `object_store` needs for large uploads.

use serde_json::json;

use crate::error::{Error, Result};
use crate::location::{StorageProvider, StorageScope};
use crate::model::AccessLevel;

/// Maximum length of a session policy's plain text, per the STS `AssumeRole` API.
pub const MAX_SESSION_POLICY_LEN: usize = 2048;

const READ_ACTIONS: &[&str] = &["s3:GetObject", "s3:GetObjectVersion"];
const WRITE_ACTIONS: &[&str] = &[
    "s3:PutObject",
    "s3:DeleteObject",
    "s3:AbortMultipartUpload",
    "s3:ListMultipartUploadParts",
];

/// The session policy restricting a role session to `scope`, in the `aws` partition.
///
/// See [`session_policy_in_partition`].
pub fn session_policy(scope: &StorageScope, access: AccessLevel) -> Result<String> {
    session_policy_in_partition(scope, access, "aws")
}

/// The session policy restricting a role session to `scope`, for an ARN `partition`
/// (`aws`, `aws-cn`, `aws-us-gov`).
///
/// Fails if `scope` is not an S3 location, if its path contains `*`, `?` or `${`
/// (IAM would read them as wildcards or policy variables, widening the grant), or if
/// the document exceeds [`MAX_SESSION_POLICY_LEN`].
pub fn session_policy_in_partition(
    scope: &StorageScope,
    access: AccessLevel,
    partition: &str,
) -> Result<String> {
    let location = scope.location();
    let invalid = |reason: &str| Error::invalid_location(location.to_string(), reason);
    if location.provider() != StorageProvider::Aws {
        return Err(invalid("S3 session policies apply to s3:// scopes only"));
    }
    let bucket = location.bucket();
    // Parsing already rejects such buckets; checked again because a wildcard bucket
    // here would grant the path in every bucket the role can reach.
    if bucket.is_empty()
        || !bucket
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    {
        return Err(invalid("not a valid S3 bucket name"));
    }
    let prefix = location.path().as_ref();
    if prefix.contains(['*', '?']) || prefix.contains("${") {
        return Err(invalid(
            "paths containing `*`, `?` or `${` cannot be expressed in an IAM policy",
        ));
    }

    let bucket_arn = format!("arn:{partition}:s3:::{bucket}");
    let object_arns = if prefix.is_empty() {
        vec![format!("{bucket_arn}/*")]
    } else {
        vec![
            format!("{bucket_arn}/{prefix}"),
            format!("{bucket_arn}/{prefix}/*"),
        ]
    };
    let object_actions: Vec<&str> = match access {
        AccessLevel::Read => READ_ACTIONS.to_vec(),
        AccessLevel::ReadWrite => [READ_ACTIONS, WRITE_ACTIONS].concat(),
    };

    let mut list = json!({
        "Sid": "List",
        "Effect": "Allow",
        "Action": ["s3:ListBucket"],
        "Resource": [bucket_arn],
    });
    if !prefix.is_empty() {
        // Only `prefix/` and below, never the bare `prefix`: S3 matches list prefixes as
        // plain strings, so `prefix=tab` would also list the keys of `table2/...`.
        // `object_store` always lists with the trailing `/`.
        list["Condition"] = json!({
            "StringLike": { "s3:prefix": [format!("{prefix}/"), format!("{prefix}/*")] }
        });
    }

    // Built with `serde_json` so a crafted bucket or path cannot break out of its string
    // and add statements or resources.
    let policy = json!({
        "Version": "2012-10-17",
        "Statement": [
            {
                "Sid": "Objects",
                "Effect": "Allow",
                "Action": object_actions,
                "Resource": object_arns,
            },
            list,
            {
                "Sid": "Location",
                "Effect": "Allow",
                "Action": ["s3:GetBucketLocation"],
                "Resource": [bucket_arn],
            },
        ],
    })
    .to_string();

    if policy.len() > MAX_SESSION_POLICY_LEN {
        return Err(invalid(
            "session policy exceeds the STS limit of 2048 characters",
        ));
    }
    Ok(policy)
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    fn policy(scope: &str, access: AccessLevel) -> Value {
        let scope: StorageScope = scope.parse().unwrap();
        serde_json::from_str(&session_policy(&scope, access).unwrap()).unwrap()
    }

    #[test]
    fn read_policy_for_a_prefix() {
        assert_eq!(
            policy("s3://bucket/warehouse/tab/", AccessLevel::Read),
            json!({
                "Version": "2012-10-17",
                "Statement": [
                    {
                        "Sid": "Objects",
                        "Effect": "Allow",
                        "Action": ["s3:GetObject", "s3:GetObjectVersion"],
                        "Resource": [
                            "arn:aws:s3:::bucket/warehouse/tab",
                            "arn:aws:s3:::bucket/warehouse/tab/*",
                        ],
                    },
                    {
                        "Sid": "List",
                        "Effect": "Allow",
                        "Action": ["s3:ListBucket"],
                        "Resource": ["arn:aws:s3:::bucket"],
                        "Condition": {
                            "StringLike": { "s3:prefix": ["warehouse/tab/", "warehouse/tab/*"] }
                        },
                    },
                    {
                        "Sid": "Location",
                        "Effect": "Allow",
                        "Action": ["s3:GetBucketLocation"],
                        "Resource": ["arn:aws:s3:::bucket"],
                    },
                ],
            })
        );
    }

    #[test]
    fn read_write_adds_writes_and_multipart() {
        let p = policy("s3://bucket/tab", AccessLevel::ReadWrite);
        assert_eq!(
            p["Statement"][0]["Action"],
            json!([
                "s3:GetObject",
                "s3:GetObjectVersion",
                "s3:PutObject",
                "s3:DeleteObject",
                "s3:AbortMultipartUpload",
                "s3:ListMultipartUploadParts",
            ])
        );
        // Bucket-level statements are the same as for reads.
        assert_eq!(
            p["Statement"][1],
            policy("s3://bucket/tab", AccessLevel::Read)["Statement"][1]
        );
    }

    #[test]
    fn bucket_scope_has_no_list_condition() {
        let p = policy("s3://bucket", AccessLevel::Read);
        assert_eq!(
            p["Statement"][0]["Resource"],
            json!(["arn:aws:s3:::bucket/*"])
        );
        assert!(p["Statement"][1].get("Condition").is_none());
    }

    #[test]
    fn no_double_slashes() {
        for scope in [
            "s3://bucket/tab/",
            "s3://bucket/a/b/",
            "https://bucket.s3.amazonaws.com/tab/",
        ] {
            let scope: StorageScope = scope.parse().unwrap();
            let text = session_policy(&scope, AccessLevel::ReadWrite).unwrap();
            assert!(!text.contains("//"), "{text}");
        }
    }

    #[test]
    fn crafted_paths_cannot_break_out() {
        // A JSON breakout attempt stays inside its resource strings: `serde_json`
        // escapes the decoded `"`.
        let scope: StorageScope = r#"s3://bucket/x%22%5D,%22Resource%22:%5B%22arn:aws:s3:::other"#
            .parse()
            .unwrap();
        let p: Value =
            serde_json::from_str(&session_policy(&scope, AccessLevel::Read).unwrap()).unwrap();
        let resources = p["Statement"][0]["Resource"].as_array().unwrap();
        assert_eq!(resources.len(), 2);
        assert!(
            resources
                .iter()
                .all(|r| r.as_str().unwrap().starts_with("arn:aws:s3:::bucket/x"))
        );
        assert_eq!(p["Statement"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn rejects_iam_wildcards_and_variables() {
        for scope in [
            "s3://bucket/ta*",
            "s3://bucket/t%3Fb",
            "s3://bucket/$%7Baws:username%7D",
        ] {
            let scope: StorageScope = scope.parse().unwrap();
            assert!(
                session_policy(&scope, AccessLevel::Read).is_err(),
                "{scope}"
            );
        }
    }

    #[test]
    fn list_condition_excludes_the_bare_prefix() {
        let p = policy("s3://bucket/tab", AccessLevel::Read);
        let prefixes = &p["Statement"][1]["Condition"]["StringLike"]["s3:prefix"];
        assert_eq!(prefixes, &json!(["tab/", "tab/*"]));
    }

    #[test]
    fn rejects_other_providers_and_oversized_policies() {
        let gs: StorageScope = "gs://bucket/tab".parse().unwrap();
        assert!(session_policy(&gs, AccessLevel::Read).is_err());

        let long: StorageScope = format!("s3://bucket/{}", "a".repeat(600)).parse().unwrap();
        assert!(session_policy(&long, AccessLevel::Read).is_err());
    }

    #[test]
    fn partitions() {
        let scope: StorageScope = "s3://bucket/tab".parse().unwrap();
        let text = session_policy_in_partition(&scope, AccessLevel::Read, "aws-cn").unwrap();
        assert!(text.contains("arn:aws-cn:s3:::bucket/tab/*"));
        assert!(!text.contains("arn:aws:"));
    }
}
