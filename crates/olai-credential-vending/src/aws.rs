//! Minting S3 credentials by assuming an IAM role with a session policy.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use olai_http::aws::{
    AmazonBuilder, AssumeRoleOptions, AwsCredential, AwsCredentialProvider,
    assume_role_with_options,
};
use olai_http::{ClientOptions, StaticCredentialProvider};
use olai_storage_credentials::policy::s3::session_policy_in_partition;
use olai_storage_credentials::{StorageCredentials, StorageProvider, StoreHints, VendedCredential};

use crate::error::{Error, Result};
use crate::mint::{CredentialMinter, MintRequest};
use crate::registered::AwsIamRole;

/// The region STS is called in when a role names none.
const DEFAULT_REGION: &str = "us-east-1";

/// The lifetime of a credential when the request names none.
pub const DEFAULT_TTL: Duration = Duration::from_secs(3600);

/// The shortest session STS issues.
const MIN_TTL: Duration = Duration::from_secs(900);

/// The longest session STS issues (roles may allow less).
const MAX_TTL: Duration = Duration::from_secs(12 * 60 * 60);

/// Mints S3 credentials by assuming an IAM role.
///
/// Each mint calls `STS:AssumeRole` with an inline session policy (P1's
/// [`session_policy`](olai_storage_credentials::policy::s3::session_policy)) that
/// restricts the session to the requested scope and access, whatever the role itself
/// allows. The request's lifetime is clamped to what STS issues (15 minutes to 12
/// hours), and its session name becomes the audited `RoleSessionName`.
pub struct AwsStsMinter {
    role_arn: String,
    region: String,
    external_id: Option<String>,
    base: AwsCredentialProvider,
    sts_endpoint: Option<String>,
    s3_endpoint: Option<String>,
    client_options: ClientOptions,
    source_identity: Option<String>,
    tags: Vec<(String, String)>,
}

impl AwsStsMinter {
    /// A minter for `role`, assuming it with the role's static base keys, or else the
    /// host's ambient AWS credentials (environment, web identity, ECS, IMDS).
    pub fn new(role: &AwsIamRole) -> Result<Self> {
        let region = role
            .region
            .clone()
            .unwrap_or_else(|| DEFAULT_REGION.to_owned());
        let base: AwsCredentialProvider = match (&role.access_key_id, &role.secret_access_key) {
            (Some(key_id), Some(secret_key)) => {
                Arc::new(StaticCredentialProvider::new(AwsCredential {
                    key_id: key_id.clone(),
                    secret_key: secret_key.clone(),
                    token: role.session_token.clone(),
                }))
            }
            (None, None) => {
                AmazonBuilder::from_env()
                    .with_region(&region)
                    .build(None)?
                    .credentials
            }
            _ => {
                return Err(Error::unsupported(
                    &role.role_arn,
                    "base credentials need both `access_key_id` and `secret_access_key`",
                ));
            }
        };
        Ok(Self {
            role_arn: role.role_arn.clone(),
            region,
            external_id: role.external_id.clone(),
            base,
            sts_endpoint: None,
            s3_endpoint: None,
            client_options: ClientOptions::default(),
            source_identity: None,
            tags: Vec::new(),
        })
    }

    /// Signs STS requests with `base` instead.
    pub fn with_base_credentials(mut self, base: AwsCredentialProvider) -> Self {
        self.base = base;
        self
    }

    /// Calls STS at `endpoint` instead of `https://sts.<region>.amazonaws.com`.
    pub fn with_sts_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.sts_endpoint = Some(endpoint.into());
        self
    }

    /// The S3-compatible endpoint the minted credentials are for, passed to clients as
    /// a store hint (with path-style addressing).
    pub fn with_s3_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.s3_endpoint = Some(endpoint.into());
        self
    }

    /// HTTP client options for STS calls.
    pub fn with_client_options(mut self, options: ClientOptions) -> Self {
        self.client_options = options;
        self
    }

    /// Sets the `SourceIdentity` of every session.
    pub fn with_source_identity(mut self, identity: impl Into<String>) -> Self {
        self.source_identity = Some(identity.into());
        self
    }

    /// Adds a session tag to every session.
    pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.push((key.into(), value.into()));
        self
    }

    /// The ARN partition of the region: `aws-cn` for China, `aws-us-gov` for GovCloud.
    fn partition(&self) -> &'static str {
        if self.region.starts_with("cn-") {
            "aws-cn"
        } else if self.region.starts_with("us-gov-") {
            "aws-us-gov"
        } else {
            "aws"
        }
    }

    /// The STS region is not the bucket's: on AWS it is left unset, so clients sign for
    /// the bucket's own region (from the URL, configuration or discovery). An
    /// S3-compatible endpoint has no region of its own to discover, so it gets the STS
    /// region.
    fn hints(&self) -> StoreHints {
        match &self.s3_endpoint {
            Some(endpoint) => StoreHints {
                endpoint: Some(endpoint.clone()),
                region: Some(self.region.clone()),
                path_style: Some(true),
                allow_http: Some(endpoint.starts_with("http://")),
                ..StoreHints::default()
            },
            None => StoreHints::default(),
        }
    }
}

/// A `RoleSessionName`: STS allows 2 to 64 characters of `[\w+=,.@-]`.
fn session_name(requested: Option<&str>) -> String {
    let name: String = requested
        .unwrap_or(olai_http::aws::DEFAULT_SESSION_NAME)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "_+=,.@-".contains(c) {
                c
            } else {
                '-'
            }
        })
        .take(64)
        .collect();
    if name.len() < 2 {
        olai_http::aws::DEFAULT_SESSION_NAME.to_owned()
    } else {
        name
    }
}

#[async_trait]
impl CredentialMinter for AwsStsMinter {
    async fn mint(&self, request: &MintRequest) -> Result<VendedCredential> {
        if request.scope.location().provider() != StorageProvider::Aws {
            return Err(Error::unsupported(
                &request.scope,
                "an IAM role mints for s3:// storage only",
            ));
        }
        let policy = session_policy_in_partition(&request.scope, request.access, self.partition())?;
        let ttl = request.ttl.unwrap_or(DEFAULT_TTL).clamp(MIN_TTL, MAX_TTL);
        let options = AssumeRoleOptions {
            duration: Some(ttl),
            session_name: Some(session_name(request.session_name.as_deref())),
            policy: Some(policy),
            external_id: self.external_id.clone(),
            source_identity: self.source_identity.clone(),
            tags: self.tags.clone(),
            sts_endpoint: self.sts_endpoint.clone(),
            client_options: self.client_options.clone(),
            ..AssumeRoleOptions::default()
        };
        let role = assume_role_with_options(
            &self.role_arn,
            &self.region,
            Arc::clone(&self.base),
            &options,
        )
        .await?;
        let credentials = StorageCredentials::aws(
            role.credential.key_id.clone(),
            role.credential.secret_key.clone(),
            role.credential.token.clone(),
        );
        Ok(
            VendedCredential::new(credentials, request.scope.clone(), request.access)
                .with_expiry(Some(role.expiration))
                .with_hints(self.hints()),
        )
    }
}

impl fmt::Debug for AwsStsMinter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AwsStsMinter")
            .field("role_arn", &self.role_arn)
            .field("region", &self.region)
            .field(
                "external_id",
                &self.external_id.as_ref().map(|_| "<redacted>"),
            )
            .field("sts_endpoint", &self.sts_endpoint)
            .field("s3_endpoint", &self.s3_endpoint)
            .field("source_identity", &self.source_identity)
            .field("tags", &self.tags)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use olai_storage_credentials::AccessLevel;

    use super::*;

    fn role() -> AwsIamRole {
        AwsIamRole {
            role_arn: "arn:aws:iam::1:role/vend".into(),
            access_key_id: Some("AKIDBASE".into()),
            secret_access_key: Some("SECRET-BASE".into()),
            ..AwsIamRole::default()
        }
    }

    #[test]
    fn session_names_are_sanitized() {
        assert_eq!(session_name(Some("alice@corp.com")), "alice@corp.com");
        assert_eq!(
            session_name(Some("main.default.t / alice")),
            "main.default.t---alice"
        );
        assert_eq!(
            session_name(Some("x")),
            olai_http::aws::DEFAULT_SESSION_NAME
        );
        assert_eq!(session_name(Some(&"a".repeat(100))).len(), 64);
    }

    #[test]
    fn partitions_follow_the_region() {
        let minter = |region: &str| {
            AwsStsMinter::new(&AwsIamRole {
                region: Some(region.into()),
                ..role()
            })
            .unwrap()
        };
        assert_eq!(minter("eu-west-1").partition(), "aws");
        assert_eq!(minter("cn-north-1").partition(), "aws-cn");
        assert_eq!(minter("us-gov-west-1").partition(), "aws-us-gov");
    }

    #[test]
    fn the_sts_region_is_not_hinted_as_the_bucket_region() {
        let role = AwsIamRole {
            region: Some("us-east-1".into()),
            ..role()
        };
        // A bucket in another region must not be signed for the STS region.
        assert_eq!(
            AwsStsMinter::new(&role).unwrap().hints(),
            StoreHints::default()
        );
        let hints = AwsStsMinter::new(&role)
            .unwrap()
            .with_s3_endpoint("http://127.0.0.1:9000")
            .hints();
        assert_eq!(hints.region.as_deref(), Some("us-east-1"));
        assert_eq!(hints.path_style, Some(true));
        assert_eq!(hints.allow_http, Some(true));
    }

    #[test]
    fn half_configured_base_credentials_are_rejected() {
        let err = AwsStsMinter::new(&AwsIamRole {
            secret_access_key: None,
            ..role()
        })
        .unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }), "{err}");
    }

    #[tokio::test]
    async fn only_s3_scopes_are_minted() {
        let minter = AwsStsMinter::new(&role()).unwrap();
        let request = MintRequest::new(
            "abfss://c@a.dfs.core.windows.net/t".parse().unwrap(),
            AccessLevel::Read,
        );
        assert!(matches!(
            minter.mint(&request).await,
            Err(Error::Unsupported { .. })
        ));
    }

    #[test]
    fn debug_hides_secrets() {
        let minter = AwsStsMinter::new(&AwsIamRole {
            external_id: Some("SECRET-EXT".into()),
            ..role()
        })
        .unwrap();
        let debug = format!("{minter:?}");
        assert!(!debug.contains("SECRET"), "{debug}");
    }
}
