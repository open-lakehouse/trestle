//! S3 bucket region discovery.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use olai_http::aws::{S3_GLOBAL_ENDPOINT, bucket_region};
use olai_http::{ClientOptions, CloudClient, RetryConfig};

use crate::error::{Error, Result};

/// How long a discovered region is trusted. A bucket's region only changes if it is
/// deleted and re-created elsewhere.
pub const DEFAULT_REGION_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// How long one probe may take before discovery gives up.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Discovers and caches the regions of S3 buckets.
///
/// Vended S3 credentials rarely say which region their bucket is in, and SigV4
/// signatures must name the right one. Discovery sends an anonymous `HeadBucket` (see
/// [`olai_http::aws::bucket_region`]): the vended credential is scoped to a prefix and
/// may not authorize `HeadBucket`, and it must not be sent anywhere but the store.
///
/// Successful lookups are cached for [`DEFAULT_REGION_TTL`]; failures are not, so a
/// transient error does not stick. Clones share one cache.
#[derive(Clone)]
pub struct BucketRegions {
    inner: Arc<Inner>,
}

struct Inner {
    client: CloudClient,
    endpoint: String,
    ttl: Duration,
    cache: Mutex<HashMap<String, (String, Instant)>>,
}

impl BucketRegions {
    /// Discovery against S3's global endpoint.
    pub fn new() -> Result<Self> {
        Self::with_endpoint(S3_GLOBAL_ENDPOINT)
    }

    /// Discovery against `endpoint` instead, for example a test server. Plain `http` is
    /// allowed only when `endpoint` uses it.
    pub fn with_endpoint(endpoint: impl Into<String>) -> Result<Self> {
        let endpoint = endpoint.into();
        let options = ClientOptions::new()
            .with_timeout(PROBE_TIMEOUT)
            .with_allow_http(endpoint.starts_with("http://"));
        let client =
            CloudClient::new_unauthenticated_with_options(&options, RetryConfig::default(), None)
                .map_err(Error::vend)?;
        Ok(Self {
            inner: Arc::new(Inner {
                client,
                endpoint,
                ttl: DEFAULT_REGION_TTL,
                cache: Mutex::default(),
            }),
        })
    }

    /// The process-wide instance [`StoreOptions`](crate::StoreOptions) uses unless told
    /// otherwise. `None` if its HTTP client could not be created.
    pub(crate) fn shared() -> Option<Self> {
        static SHARED: OnceLock<Option<BucketRegions>> = OnceLock::new();
        SHARED.get_or_init(|| Self::new().ok()).clone()
    }

    /// The region of `bucket`, or `None` if it could not be discovered.
    pub async fn region(&self, bucket: &str) -> Option<String> {
        if let Some((region, at)) = self.inner.cache.lock().unwrap().get(bucket)
            && at.elapsed() < self.inner.ttl
        {
            return Some(region.clone());
        }
        let region = bucket_region(&self.inner.client, &self.inner.endpoint, bucket)
            .await
            .ok()
            .flatten()?;
        self.inner
            .cache
            .lock()
            .unwrap()
            .insert(bucket.to_owned(), (region.clone(), Instant::now()));
        Some(region)
    }
}

impl fmt::Debug for BucketRegions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BucketRegions")
            .field("endpoint", &self.inner.endpoint)
            .field("ttl", &self.inner.ttl)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regions(server: &mockito::Server) -> BucketRegions {
        BucketRegions::with_endpoint(server.url()).unwrap()
    }

    #[tokio::test]
    async fn region_is_discovered_once_per_bucket() {
        let mut server = mockito::Server::new_async().await;
        let probe = server
            .mock("HEAD", "/bucket")
            .with_status(403)
            .with_header("x-amz-bucket-region", "eu-central-1")
            .expect(1)
            .create_async()
            .await;
        let regions = regions(&server);
        for _ in 0..3 {
            assert_eq!(
                regions.region("bucket").await.as_deref(),
                Some("eu-central-1")
            );
        }
        probe.assert_async().await;
    }

    #[tokio::test]
    async fn failed_discovery_is_retried() {
        let mut server = mockito::Server::new_async().await;
        let missing = server
            .mock("HEAD", "/bucket")
            .with_status(500)
            .expect(1)
            .create_async()
            .await;
        let regions = regions(&server);
        assert_eq!(regions.region("bucket").await, None);
        missing.assert_async().await;
        missing.remove_async().await;

        let found = server
            .mock("HEAD", "/bucket")
            .with_status(200)
            .with_header("x-amz-bucket-region", "us-west-2")
            .create_async()
            .await;
        assert_eq!(regions.region("bucket").await.as_deref(), Some("us-west-2"));
        found.assert_async().await;
    }
}
