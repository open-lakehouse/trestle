use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};

use futures::future::BoxFuture;

use crate::service::make_service;
use crate::token::TemporaryToken;
use crate::{ClientOptions, CloudClient, RequestSigner, Result, RetryConfig, TokenProvider};

use self::credential::{AssumeRoleParams, AssumeRoleProvider, AwsAuthorizer, CredentialExt};
use crate::CredentialProvider;

mod builder;
pub(crate) mod credential;

pub use builder::*;
pub use credential::AwsCredential;

pub type AwsCredentialProvider = Arc<dyn CredentialProvider<Credential = AwsCredential>>;

#[derive(Debug, Clone)]
pub struct AmazonConfig {
    pub region: String,
    pub credentials: AwsCredentialProvider,
    pub retry_config: RetryConfig,
    pub client_options: ClientOptions,
    pub skip_signature: bool,
}

impl AmazonConfig {
    pub(crate) async fn get_credential(&self) -> Result<Option<Arc<AwsCredential>>> {
        Ok(match self.skip_signature {
            false => Some(self.credentials.get_credential().await?),
            true => None,
        })
    }
}

impl RequestSigner for AmazonConfig {
    fn sign<'a>(
        &'a self,
        req: reqwest::RequestBuilder,
    ) -> BoxFuture<'a, Result<reqwest::RequestBuilder>> {
        Box::pin(async move {
            if let Some(cred) = self.get_credential().await? {
                let authorizer = AwsAuthorizer::new(&cred, "execute-api", &self.region);
                Ok(req.with_aws_sigv4(Some(authorizer), None))
            } else {
                Ok(req)
            }
        })
    }
}

/// S3's global endpoint, which answers `HeadBucket` for a bucket in any region.
pub const S3_GLOBAL_ENDPOINT: &str = "https://s3.amazonaws.com";

/// Discovers the region of an S3 bucket.
///
/// Sends a path-style `HeadBucket` for `bucket` to `endpoint` (usually
/// [`S3_GLOBAL_ENDPOINT`]) and reads the `x-amz-bucket-region` header. S3 sets it whether
/// or not the caller may access the bucket (`200`, `301` and `403` all carry it), so the
/// probe needs no credentials. Path-style addressing keeps bucket names with dots
/// matching S3's certificate.
///
/// The probe is always anonymous, whatever `client` signs its own requests with, and is
/// sent once without retries: it is a best-effort lookup, so the caller decides how to
/// fall back. `client`'s [`ClientOptions`] (timeouts, proxies) and runtime still apply;
/// see [`CloudClient::new_unauthenticated_with_options`]. Returns `Ok(None)` when the
/// response names no region, for example from an S3-compatible store that is not AWS.
pub async fn bucket_region(
    client: &CloudClient,
    endpoint: &str,
    bucket: &str,
) -> Result<Option<String>> {
    let mut url = reqwest::Url::parse(endpoint).map_err(|source| crate::Error::Generic {
        source: Box::new(source),
    })?;
    url.path_segments_mut()
        .map_err(|()| crate::Error::Generic {
            source: format!("endpoint `{endpoint}` cannot carry a path").into(),
        })?
        .pop_if_empty()
        .push(bucket);
    // Straight to the transport: no signer (anonymous) and no retry layer, which
    // turns the 301 and 403 that carry the header into errors without headers.
    let request = client.reqwest_client.head(url).build()?;
    let response = client.service.call(request).await?;
    Ok(response
        .headers()
        .get("x-amz-bucket-region")
        .and_then(|region| region.to_str().ok())
        .filter(|region| !region.is_empty())
        .map(str::to_owned))
}

/// Assume an AWS IAM role using explicit base credentials.
///
/// Like [`assume_role`] but accepts an explicit `base_credentials` provider
/// instead of falling back to environment-based credential discovery.
/// Use this when the base identity (the caller's access key + secret) is
/// stored in a credential registry rather than in the server environment.
pub async fn assume_role_with_base(
    role_arn: &str,
    region: &str,
    sts_endpoint: Option<&str>,
    policy: Option<String>,
    base_credentials: AwsCredentialProvider,
) -> Result<TemporaryToken<Arc<AwsCredential>>> {
    assume_role_with_base_and_options(
        role_arn,
        region,
        sts_endpoint,
        policy,
        base_credentials,
        &ClientOptions::default(),
    )
    .await
}

/// Like [`assume_role_with_base`], with explicit HTTP [`ClientOptions`] for the STS call.
///
/// Use this to reach an STS endpoint the defaults refuse, for example a plain-`http`
/// emulator (`ClientOptions::new().with_allow_http(true)`), or to set timeouts and
/// proxies.
pub async fn assume_role_with_base_and_options(
    role_arn: &str,
    region: &str,
    sts_endpoint: Option<&str>,
    policy: Option<String>,
    base_credentials: AwsCredentialProvider,
    client_options: &ClientOptions,
) -> Result<TemporaryToken<Arc<AwsCredential>>> {
    let endpoint = sts_endpoint
        .map(|s| s.to_owned())
        .unwrap_or_else(|| format!("https://sts.{region}.amazonaws.com"));

    let provider = AssumeRoleProvider {
        role_arn: role_arn.to_owned(),
        session_name: DEFAULT_SESSION_NAME.to_owned(),
        endpoint,
        base_credentials,
        region: region.to_owned(),
        policy,
    };

    let client = client_options.client()?;
    let service = make_service(client.clone(), None);
    provider
        .fetch_token(&client, &service, &RetryConfig::default())
        .await
}

/// The session name [`assume_role_with_options`] uses when none is given.
pub const DEFAULT_SESSION_NAME: &str = "TrestleVending";

/// The parameters of an `STS:AssumeRole` call beyond the role.
///
/// <https://docs.aws.amazon.com/STS/latest/APIReference/API_AssumeRole.html>
#[derive(Debug, Clone, Default)]
pub struct AssumeRoleOptions {
    /// `DurationSeconds`: the session lifetime, between 15 minutes and the role's maximum.
    /// STS's own default (one hour) when `None`.
    pub duration: Option<Duration>,
    /// `RoleSessionName`, recorded in CloudTrail; [`DEFAULT_SESSION_NAME`] when `None`.
    pub session_name: Option<String>,
    /// `Policy`: an inline session policy (JSON) intersected with the role's policies.
    pub policy: Option<String>,
    /// `ExternalId`, when the role's trust policy requires one.
    pub external_id: Option<String>,
    /// `SourceIdentity`, a persistent identity for the caller, recorded in CloudTrail.
    pub source_identity: Option<String>,
    /// Session tags, as `Tags.member.N` key/value pairs.
    pub tags: Vec<(String, String)>,
    /// The STS endpoint; `https://sts.<region>.amazonaws.com` when `None`.
    pub sts_endpoint: Option<String>,
    /// HTTP client options for the STS call.
    pub client_options: ClientOptions,
    /// Retries for the STS call.
    pub retry: RetryConfig,
}

/// Session credentials from `STS:AssumeRole`, with their wall-clock expiry.
#[derive(Debug)]
pub struct AssumedRole {
    /// The session credentials (`Debug` redacts them).
    pub credential: AwsCredential,
    /// When STS says the credentials expire.
    pub expiration: DateTime<Utc>,
}

/// Assume an AWS IAM role with every `AssumeRole` parameter available.
///
/// `base_credentials` signs the STS request. Unlike [`assume_role_with_base`], this
/// returns the expiry STS reports as a wall-clock time, which is what a credential
/// vending server hands its clients.
pub async fn assume_role_with_options(
    role_arn: &str,
    region: &str,
    base_credentials: AwsCredentialProvider,
    options: &AssumeRoleOptions,
) -> Result<AssumedRole> {
    let endpoint = options
        .sts_endpoint
        .clone()
        .unwrap_or_else(|| format!("https://sts.{region}.amazonaws.com"));
    let base = base_credentials
        .get_credential()
        .await
        .map_err(|source| crate::Error::Generic {
            source: Box::new(source),
        })?;
    let params = AssumeRoleParams {
        role_arn,
        session_name: options
            .session_name
            .as_deref()
            .unwrap_or(DEFAULT_SESSION_NAME),
        duration_secs: options.duration.map(|d| d.as_secs()),
        policy: options.policy.as_deref(),
        external_id: options.external_id.as_deref(),
        source_identity: options.source_identity.as_deref(),
        tags: &options.tags,
    };
    let client = options.client_options.client()?;
    let service = make_service(client.clone(), None);
    let (credential, expiration) = credential::assume_role_request(
        &client,
        &service,
        &options.retry,
        &base,
        region,
        &endpoint,
        &params,
    )
    .await
    .map_err(|source| crate::Error::Generic { source })?;
    Ok(AssumedRole {
        credential,
        expiration,
    })
}

/// Assume an AWS IAM role using ambient server credentials.
///
/// Uses the environment credential chain (`AWS_*` env vars, instance profile,
/// ECS task role, WebIdentity, etc.) as the base identity for the STS
/// `AssumeRole` call. Prefer [`assume_role_with_base`] when the base
/// credentials are stored in a credential registry.
pub async fn assume_role(
    role_arn: &str,
    region: &str,
    sts_endpoint: Option<&str>,
    policy: Option<String>,
) -> Result<TemporaryToken<Arc<AwsCredential>>> {
    let base_credentials = AmazonBuilder::from_env()
        .with_region(region)
        .build(None)?
        .credentials;
    assume_role_with_base(role_arn, region, sts_endpoint, policy, base_credentials).await
}

#[cfg(test)]
mod tests {
    use mockito::Matcher;

    use super::*;

    fn client() -> CloudClient {
        CloudClient::new_unauthenticated_with_options(
            &ClientOptions::new().with_allow_http(true),
            RetryConfig::default(),
            None,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn bucket_region_is_read_from_any_status() {
        let mut server = mockito::Server::new_async().await;
        for (bucket, status, region) in [
            ("open", 200, "us-east-1"),
            ("elsewhere", 301, "eu-west-1"),
            ("private.with.dots", 403, "ap-south-1"),
        ] {
            let mock = server
                .mock("HEAD", format!("/{bucket}").as_str())
                .match_header("authorization", Matcher::Missing)
                .with_status(status)
                .with_header("x-amz-bucket-region", region)
                .create_async()
                .await;
            let found = bucket_region(&client(), &server.url(), bucket)
                .await
                .unwrap();
            assert_eq!(found.as_deref(), Some(region), "{bucket}");
            mock.assert_async().await;
        }
    }

    #[tokio::test]
    async fn bucket_region_is_none_without_the_header() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("HEAD", "/missing")
            .with_status(404)
            .create_async()
            .await;
        let found = bucket_region(&client(), &server.url(), "missing")
            .await
            .unwrap();
        assert_eq!(found, None);
    }

    const ASSUME_ROLE_RESPONSE: &str = r#"<AssumeRoleResponse xmlns="https://sts.amazonaws.com/doc/2011-06-15/">
  <AssumeRoleResult>
    <Credentials>
      <AccessKeyId>ASIASESSION</AccessKeyId>
      <SecretAccessKey>session-secret</SecretAccessKey>
      <SessionToken>session-token</SessionToken>
      <Expiration>2030-01-01T00:15:00Z</Expiration>
    </Credentials>
  </AssumeRoleResult>
</AssumeRoleResponse>"#;

    fn base() -> AwsCredentialProvider {
        Arc::new(crate::StaticCredentialProvider::new(AwsCredential {
            key_id: "AKIDBASE".into(),
            secret_key: "base-secret".into(),
            token: None,
        }))
    }

    fn options(server: &mockito::Server) -> AssumeRoleOptions {
        AssumeRoleOptions {
            sts_endpoint: Some(server.url()),
            client_options: ClientOptions::new().with_allow_http(true),
            ..AssumeRoleOptions::default()
        }
    }

    #[tokio::test]
    async fn assume_role_sends_every_option() {
        let mut server = mockito::Server::new_async().await;
        let sts = server
            .mock("POST", "/")
            .match_header(
                "authorization",
                Matcher::Regex("^AWS4-HMAC-SHA256 Credential=AKIDBASE/".into()),
            )
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("Action".into(), "AssumeRole".into()),
                Matcher::UrlEncoded("RoleArn".into(), "arn:aws:iam::1:role/vend".into()),
                Matcher::UrlEncoded("DurationSeconds".into(), "900".into()),
                Matcher::UrlEncoded("RoleSessionName".into(), "alice@table".into()),
                Matcher::UrlEncoded("ExternalId".into(), "ext-42".into()),
                Matcher::UrlEncoded("SourceIdentity".into(), "alice".into()),
                Matcher::UrlEncoded("Policy".into(), r#"{"Version":"2012-10-17"}"#.into()),
                Matcher::UrlEncoded("Tags.member.1.Key".into(), "table".into()),
                Matcher::UrlEncoded("Tags.member.1.Value".into(), "main.default.t".into()),
                Matcher::UrlEncoded("Tags.member.2.Key".into(), "env".into()),
            ]))
            .with_body(ASSUME_ROLE_RESPONSE)
            .create_async()
            .await;
        let options = AssumeRoleOptions {
            duration: Some(Duration::from_secs(900)),
            session_name: Some("alice@table".into()),
            policy: Some(r#"{"Version":"2012-10-17"}"#.into()),
            external_id: Some("ext-42".into()),
            source_identity: Some("alice".into()),
            tags: vec![
                ("table".into(), "main.default.t".into()),
                ("env".into(), "dev".into()),
            ],
            ..options(&server)
        };
        let role =
            assume_role_with_options("arn:aws:iam::1:role/vend", "us-east-1", base(), &options)
                .await
                .unwrap();
        assert_eq!(role.credential.key_id, "ASIASESSION");
        assert_eq!(role.credential.token.as_deref(), Some("session-token"));
        assert_eq!(role.expiration.to_rfc3339(), "2030-01-01T00:15:00+00:00");
        sts.assert_async().await;
    }

    #[tokio::test]
    async fn assume_role_defaults() {
        let mut server = mockito::Server::new_async().await;
        let body = Arc::new(std::sync::Mutex::new(String::new()));
        let seen = Arc::clone(&body);
        let sts = server
            .mock("POST", "/")
            .with_body_from_request(move |request| {
                *seen.lock().unwrap() =
                    String::from_utf8_lossy(request.body().unwrap()).into_owned();
                ASSUME_ROLE_RESPONSE.into()
            })
            .create_async()
            .await;
        assume_role_with_options(
            "arn:aws:iam::1:role/vend",
            "us-east-1",
            base(),
            &options(&server),
        )
        .await
        .unwrap();
        sts.assert_async().await;
        let body = body.lock().unwrap();
        assert!(body.contains("DurationSeconds=3600"), "{body}");
        assert!(
            body.contains(&format!("RoleSessionName={DEFAULT_SESSION_NAME}")),
            "{body}"
        );
        // Unset options are not sent at all.
        for absent in ["ExternalId", "SourceIdentity", "Tags", "Policy"] {
            assert!(!body.contains(absent), "{absent} in {body}");
        }
    }
}
