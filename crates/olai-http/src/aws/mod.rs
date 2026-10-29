use std::sync::Arc;

use futures::future::BoxFuture;

use crate::service::make_service;
use crate::token::TemporaryToken;
use crate::{ClientOptions, CloudClient, RequestSigner, Result, RetryConfig, TokenProvider};

use self::credential::{AssumeRoleProvider, AwsAuthorizer, CredentialExt};
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
        session_name: "TrestleVending".to_owned(),
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
                .match_header("authorization", mockito::Matcher::Missing)
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
}
