//! Presigned URLs from a vended credential.
//!
//! A sharing server hands recipients URLs they can use without a credential of their
//! own. [`presign`] derives them from a credential covering the files:
//!
//! - **S3 and R2**: SigV4 query-signed URLs, signed with the vended session keys.
//! - **Azure with a SAS**: the vended SAS appended to each blob URL. A SAS cannot be
//!   shortened, so the URLs live as long as the credential.
//! - **Azure with an AD token**: a user-delegation SAS signed for the requested lifetime
//!   (the token must be allowed to request a user delegation key).
//! - **`file://`**: the file URLs, unchanged.
//!
//! GCS OAuth tokens cannot sign URLs (that needs a service-account key), and neither can
//! an anonymous credential; both are rejected.
//!
//! Every location must be covered by the credential, at the access the method needs, so
//! a URL is never minted for storage the credential was not granted for. A URL never
//! outlives its credential.

use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use http::Method;
use object_store::path::Path;
use object_store::signer::Signer;
use olai_storage_credentials::{
    AccessLevel, StorageCredentials, StorageLocation, StorageProvider, VendedCredential,
};
use url::Url;

use crate::error::{Error, Result};
use crate::store::{StoreOptions, unsupported};

/// The longest a signed URL lives: SigV4 and user delegation SAS both cap it at 7 days.
const MAX_SIGNED_LIFETIME: TimeDelta = TimeDelta::days(7);

/// Presigned URLs for a batch of locations, all valid until the same time.
///
/// The URLs are bearer credentials: `Debug` shows them without their query strings.
#[derive(Clone, PartialEq, Eq)]
pub struct Presigned {
    /// One URL per requested location, in order.
    pub urls: Vec<Url>,
    /// When the URLs stop working; `None` if they do not expire (local files).
    pub expires_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for Presigned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let urls: Vec<String> = self
            .urls
            .iter()
            .map(|url| match url.query() {
                Some(_) => format!("{}?<redacted>", &url[..url::Position::AfterPath]),
                None => url.to_string(),
            })
            .collect();
        f.debug_struct("Presigned")
            .field("urls", &urls)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Presigns `locations` for `method`, valid for at most `ttl` and never past the
/// credential's expiry or 7 days, the longest a signed S3 URL or user delegation SAS
/// lives. See the module docs.
///
/// The exception is a vended Azure SAS: it cannot be shortened, so its URLs are valid
/// until the SAS expires, whatever `ttl` asks for. [`Presigned::expires_at`] always
/// states the real expiry.
///
/// `GET` and `HEAD` need [`AccessLevel::Read`]; every other method needs
/// [`AccessLevel::ReadWrite`]. `options` supplies the clock, S3 region and HTTP client
/// options, as for [`build_store`](crate::build_store).
pub async fn presign(
    credential: &VendedCredential,
    method: Method,
    locations: &[StorageLocation],
    ttl: Duration,
    options: &StoreOptions,
) -> Result<Presigned> {
    let access = if matches!(method, Method::GET | Method::HEAD) {
        AccessLevel::Read
    } else {
        AccessLevel::ReadWrite
    };
    if let Some(location) = locations.iter().find(|l| !credential.covers(l, access)) {
        return Err(Error::NotCovered {
            location: location.to_string(),
            scope: credential.scope.to_string(),
            access,
        });
    }

    let scope = credential.scope.location();
    if scope.provider() == StorageProvider::Local {
        return Ok(Presigned {
            urls: locations.iter().map(StorageLocation::to_url).collect(),
            expires_at: None,
        });
    }

    let now = options.now();
    if credential.is_expired(now) {
        return Err(Error::Expired {
            scope: credential.scope.to_string(),
        });
    }
    // SigV4 query signatures and user delegation SAS both live at most 7 days; a longer
    // URL would be refused when used, while `expires_at` promised more.
    let requested = TimeDelta::from_std(ttl)
        .unwrap_or(TimeDelta::MAX)
        .min(MAX_SIGNED_LIFETIME);
    let lifetime = match credential.expires_at {
        Some(expires_at) => requested.min(expires_at - now),
        None => requested,
    };
    // An expiry past the representable range would read as "does not expire".
    let expires_at = Some(
        now.checked_add_signed(lifetime)
            .ok_or_else(|| Error::Config(format!("presign lifetime {ttl:?} is out of range")))?,
    );
    let lifetime = lifetime.to_std().unwrap_or_default();
    let paths: Vec<Path> = locations.iter().map(|l| l.path().clone()).collect();

    let urls = match (scope.provider(), &credential.credentials) {
        #[cfg(feature = "aws")]
        (StorageProvider::Aws | StorageProvider::R2, StorageCredentials::Aws { .. }) => {
            s3_urls(credential, options, method, &paths, lifetime).await?
        }
        #[cfg(feature = "azure")]
        (StorageProvider::Azure, StorageCredentials::AzureSas(pairs)) => {
            // The SAS carries its own expiry, which cannot be shortened.
            return Ok(Presigned {
                urls: locations
                    .iter()
                    .map(|l| azure_sas_url(credential, l, pairs))
                    .collect::<Result<_>>()?,
                expires_at: credential.expires_at,
            });
        }
        #[cfg(feature = "azure")]
        (StorageProvider::Azure, StorageCredentials::AzureBearer(token)) => {
            let store = crate::store::azure_builder(credential, options)?
                .with_bearer_token_authorization(token)
                .build()?;
            store.signed_urls(method, &paths, lifetime).await?
        }
        (provider, credentials) => {
            return Err(unsupported(
                scope,
                format!(
                    "cannot presign {provider:?} URLs with {} credentials",
                    crate::convert::kind(credentials)
                ),
            ));
        }
    };
    Ok(Presigned { urls, expires_at })
}

#[cfg(feature = "aws")]
async fn s3_urls(
    credential: &VendedCredential,
    options: &StoreOptions,
    method: Method,
    paths: &[Path],
    lifetime: Duration,
) -> Result<Vec<Url>> {
    let StorageCredentials::Aws {
        key_id,
        secret_key,
        token,
    } = &credential.credentials
    else {
        unreachable!("matched by the caller");
    };
    let mut builder = crate::store::s3_builder(credential, options)
        .await?
        .with_access_key_id(key_id)
        .with_secret_access_key(secret_key);
    if let Some(token) = token {
        builder = builder.with_token(token);
    }
    Ok(builder
        .build()?
        .signed_urls(method, paths, lifetime)
        .await?)
}

/// The blob URL of `location` with the vended SAS as its query.
#[cfg(feature = "azure")]
fn azure_sas_url(
    credential: &VendedCredential,
    location: &StorageLocation,
    sas: &[(String, String)],
) -> Result<Url> {
    let account = location
        .account()
        .ok_or_else(|| unsupported(location, "Azure location names no storage account"))?;
    let base = match &credential.hints.endpoint {
        Some(endpoint) => crate::store::azure_account_url(endpoint, account),
        None => format!("https://{account}.blob.core.windows.net"),
    };
    let mut url = Url::parse(&base).map_err(|e| unsupported(location, e.to_string()))?;
    url.path_segments_mut()
        .map_err(|()| unsupported(location, "endpoint cannot carry a path"))?
        .pop_if_empty()
        .push(location.bucket())
        .extend(location.path().parts().map(|p| p.as_ref().to_owned()));
    // The pairs are decoded; this encodes them exactly once.
    url.query_pairs_mut().extend_pairs(sas);
    Ok(url)
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "azure")]
    use olai_storage_credentials::StoreHints;

    use super::*;

    fn location(url: &str) -> StorageLocation {
        url.parse().unwrap()
    }

    fn fixed_clock(now: DateTime<Utc>) -> StoreOptions {
        let options = StoreOptions::new().with_clock(std::sync::Arc::new(move || now));
        #[cfg(feature = "aws")]
        let options = options
            .without_region_discovery()
            .with_aws_region("us-east-1");
        options
    }

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap()
    }

    #[cfg(feature = "aws")]
    fn s3(expires_in: i64) -> VendedCredential {
        VendedCredential::new(
            StorageCredentials::aws("AKIDEXAMPLE", "secret", Some("session".into())),
            "s3://bucket/tab".parse().unwrap(),
            AccessLevel::Read,
        )
        .with_expiry(Some(now() + TimeDelta::seconds(expires_in)))
    }

    #[cfg(feature = "aws")]
    #[tokio::test]
    async fn s3_urls_are_signed_and_clamped_to_the_credential() {
        let signed = presign(
            &s3(30),
            Method::GET,
            &[location("s3://bucket/tab/part-0")],
            Duration::from_secs(3600),
            &fixed_clock(now()),
        )
        .await
        .unwrap();
        assert_eq!(signed.expires_at, Some(now() + TimeDelta::seconds(30)));
        let url = &signed.urls[0];
        assert!(url.path().ends_with("/tab/part-0"), "{url}");
        let query: Vec<_> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert!(
            query.contains(&("X-Amz-Expires".into(), "30".into())),
            "{url}"
        );
        assert!(
            query.contains(&("X-Amz-Security-Token".into(), "session".into())),
            "{url}"
        );

        // A shorter ttl wins over the credential.
        let signed = presign(
            &s3(3600),
            Method::GET,
            &[location("s3://bucket/tab/part-0")],
            Duration::from_secs(60),
            &fixed_clock(now()),
        )
        .await
        .unwrap();
        assert_eq!(signed.expires_at, Some(now() + TimeDelta::seconds(60)));
    }

    #[cfg(feature = "aws")]
    #[tokio::test]
    async fn s3_urls_live_at_most_seven_days() {
        // Static keys do not expire, so only SigV4's limit applies.
        let credential = VendedCredential::new(
            StorageCredentials::aws("AKIDEXAMPLE", "secret", None),
            "s3://bucket/tab".parse().unwrap(),
            AccessLevel::Read,
        );
        let signed = presign(
            &credential,
            Method::GET,
            &[location("s3://bucket/tab/part-0")],
            Duration::from_secs(30 * 24 * 3600),
            &fixed_clock(now()),
        )
        .await
        .unwrap();
        assert_eq!(signed.expires_at, Some(now() + TimeDelta::days(7)));
        let url = &signed.urls[0];
        assert!(
            url.query_pairs()
                .any(|(k, v)| k == "X-Amz-Expires" && v == "604800"),
            "{url}"
        );
    }

    #[cfg(feature = "aws")]
    #[tokio::test]
    async fn uncovered_locations_are_refused() {
        for (url, method) in [
            // A sibling sharing the string prefix.
            ("s3://bucket/table2/part-0", Method::GET),
            ("s3://other/tab/part-0", Method::GET),
            // Writes need read-write access.
            ("s3://bucket/tab/part-0", Method::PUT),
        ] {
            let err = presign(
                &s3(3600),
                method,
                &[location("s3://bucket/tab/ok"), location(url)],
                Duration::from_secs(60),
                &fixed_clock(now()),
            )
            .await
            .unwrap_err();
            assert!(matches!(err, Error::NotCovered { .. }), "{url}: {err}");
        }
    }

    #[cfg(feature = "aws")]
    #[tokio::test]
    async fn expired_credential_is_refused() {
        let err = presign(
            &s3(-1),
            Method::GET,
            &[location("s3://bucket/tab/part-0")],
            Duration::from_secs(60),
            &fixed_clock(now()),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, Error::Expired { .. }), "{err}");
    }

    #[cfg(feature = "azure")]
    #[tokio::test]
    async fn azure_sas_is_appended_once_encoded() {
        let credential = VendedCredential::new(
            StorageCredentials::azure_sas(
                "sv=2022-11-02&sr=c&sp=rl&se=2027-01-01T00%3A00%3A00Z&sig=a%2Bb%2F%3D",
            )
            .unwrap(),
            "abfss://ctr@acct.dfs.core.windows.net/tab".parse().unwrap(),
            AccessLevel::Read,
        )
        .with_expiry(Some(now() + TimeDelta::hours(1)));
        let signed = presign(
            &credential,
            Method::GET,
            &[location("abfss://ctr@acct.dfs.core.windows.net/tab/part 0")],
            Duration::from_secs(60),
            &fixed_clock(now()),
        )
        .await
        .unwrap();
        let url = &signed.urls[0];
        assert_eq!(url.host_str(), Some("acct.blob.core.windows.net"));
        assert_eq!(url.path(), "/ctr/tab/part%200");
        let sig: Vec<_> = url.query_pairs().filter(|(k, _)| k == "sig").collect();
        assert_eq!(sig[0].1, "a+b/=", "decoded once, not twice: {url}");
        // A SAS cannot be shortened: the URL lives as long as the credential.
        assert_eq!(signed.expires_at, credential.expires_at);
    }

    #[cfg(feature = "azure")]
    #[tokio::test]
    async fn azurite_sas_urls_are_path_style() {
        let credential = VendedCredential::new(
            StorageCredentials::azure_sas("sv=2022-11-02&sig=abc").unwrap(),
            "abfss://ctr@devstoreaccount1.dfs.core.windows.net/tab"
                .parse()
                .unwrap(),
            AccessLevel::Read,
        )
        .with_hints(StoreHints {
            endpoint: Some("http://127.0.0.1:10000".into()),
            ..StoreHints::default()
        });
        let signed = presign(
            &credential,
            Method::GET,
            &[location(
                "abfss://ctr@devstoreaccount1.dfs.core.windows.net/tab/a",
            )],
            Duration::from_secs(60),
            &fixed_clock(now()),
        )
        .await
        .unwrap();
        assert_eq!(
            signed.urls[0].as_str(),
            "http://127.0.0.1:10000/devstoreaccount1/ctr/tab/a?sv=2022-11-02&sig=abc"
        );
    }

    #[test]
    fn debug_hides_signatures() {
        let presigned = Presigned {
            urls: vec![Url::parse("https://h/p?sig=SECRET").unwrap()],
            expires_at: None,
        };
        let debug = format!("{presigned:?}");
        assert!(!debug.contains("SECRET"), "{debug}");
        assert!(debug.contains("https://h/p?<redacted>"), "{debug}");
    }

    #[tokio::test]
    async fn local_files_are_returned_unchanged() {
        let credential = VendedCredential::new(
            StorageCredentials::None,
            "file:///data/tab".parse().unwrap(),
            AccessLevel::Read,
        );
        let signed = presign(
            &credential,
            Method::GET,
            &[location("file:///data/tab/part-0")],
            Duration::from_secs(60),
            &StoreOptions::new(),
        )
        .await
        .unwrap();
        assert_eq!(signed.urls[0].as_str(), "file:///data/tab/part-0");
        assert_eq!(signed.expires_at, None);
    }

    #[cfg(feature = "gcp")]
    #[tokio::test]
    async fn gcs_tokens_cannot_presign() {
        let credential = VendedCredential::new(
            StorageCredentials::GcpBearer("ya29".into()),
            "gs://bucket/tab".parse().unwrap(),
            AccessLevel::Read,
        );
        let err = presign(
            &credential,
            Method::GET,
            &[location("gs://bucket/tab/part-0")],
            Duration::from_secs(60),
            &StoreOptions::new(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }), "{err}");
        assert!(!err.to_string().contains("ya29"));
    }
}
