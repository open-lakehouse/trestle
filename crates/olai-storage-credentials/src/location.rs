//! Storage locations and prefix matching.
//!
//! A storage location can be spelled many ways: `abfss://container@account.dfs.core.windows.net/p`
//! and `https://account.blob.core.windows.net/container/p` name the same directory, as do
//! `s3://bucket/p` and `https://bucket.s3.us-east-1.amazonaws.com/p`. Every spelling parses
//! to one canonical [`StorageLocation`], and prefix matching only ever compares canonical
//! forms, one path segment at a time. So `s3://b/tab` covers `s3://b/tab/part-0` but never
//! `s3://b/table2`.
//!
//! [`StorageScope`] is a location used as a prefix: the subtree a credential covers.
//! [`longest_match`] picks the most specific scope covering a location.

use std::fmt;
use std::str::FromStr;

use object_store::path::Path;
use url::Url;

use crate::error::{Error, Result};
use crate::model::StoreHints;

/// Account name the Azurite storage emulator serves by default.
pub const AZURITE_ACCOUNT: &str = "devstoreaccount1";

/// Blob endpoint the Azurite storage emulator listens on by default.
pub const AZURITE_BLOB_ENDPOINT: &str = "http://127.0.0.1:10000";

/// Port of the Azurite blob service, used to recognize emulator URLs.
const AZURITE_BLOB_PORT: u16 = 10000;

/// The kind of object store a location lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum StorageProvider {
    /// Amazon S3, and S3-compatible stores addressed by `s3://` plus an endpoint hint.
    Aws,
    /// Cloudflare R2. Buckets are scoped to an R2 account, so the account is part of the
    /// location's identity.
    R2,
    /// Azure Blob Storage and ADLS Gen2, including the Azurite emulator.
    Azure,
    /// Google Cloud Storage.
    Gcp,
    /// The local file system.
    Local,
    /// An in-memory store.
    Memory,
}

/// A canonical storage location: a provider, an optional account, a bucket and a path.
///
/// Build one with [`StorageLocation::parse`] (or [`FromStr`]); two spellings of the same
/// location compare equal. [`Display`](fmt::Display) renders the canonical URL:
///
/// | Provider | Canonical form |
/// |---|---|
/// | [`Aws`](StorageProvider::Aws) | `s3://bucket/path` |
/// | [`R2`](StorageProvider::R2) | `r2://bucket@account.r2.cloudflarestorage.com/path` |
/// | [`Azure`](StorageProvider::Azure) | `abfss://container@account.dfs.core.windows.net/path`, or `az://container/path` when the account is unknown |
/// | [`Gcp`](StorageProvider::Gcp) | `gs://bucket/path` |
/// | [`Local`](StorageProvider::Local) | `file:///path` |
/// | [`Memory`](StorageProvider::Memory) | `memory:///path` |
///
/// The bucket is the S3/R2/GCS bucket or the Azure container; it is empty for local and
/// in-memory locations. The path never has a leading or trailing `/`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StorageLocation {
    provider: StorageProvider,
    account: Option<String>,
    bucket: String,
    path: Path,
}

impl StorageLocation {
    /// Parses a storage URL into its canonical location.
    ///
    /// Accepted spellings:
    ///
    /// - `s3://bucket/path`, `s3a://bucket/path`
    /// - `https://bucket.s3[.region].amazonaws.com/path` (virtual-hosted) and
    ///   `https://s3[.region].amazonaws.com/bucket/path` (path-style)
    /// - `r2://bucket@account.r2.cloudflarestorage.com/path`,
    ///   `https://account.r2.cloudflarestorage.com/bucket/path`
    /// - `abfss://container@account.dfs.core.windows.net/path` (also `abfs://`, and `az://`,
    ///   `adl://`, `azure://` with a `container@account` authority)
    /// - `az://container/path` (no account; also `adl://`, `azure://`)
    /// - `https://account.{blob,dfs}.core.windows.net/container/path`
    /// - Azurite: `http://127.0.0.1:10000/account/container/path` (also `localhost`) and
    ///   `azurite://container/path`
    /// - `gs://bucket/path`, `https://storage.googleapis.com/bucket/path`,
    ///   `https://bucket.storage.googleapis.com/path`
    /// - `file:///path`, `memory:///path`
    ///
    /// Query strings and fragments are ignored. A generic `https://host/...` URL is
    /// rejected: without configuration there is no telling which bucket it names.
    pub fn parse(url: &Url) -> Result<Self> {
        Self::parse_with_hints(url).map(|(location, _)| location)
    }

    /// Like [`parse`](Self::parse), and also returns the [`StoreHints`] the URL implies,
    /// such as the S3 region of a regional endpoint or the Azurite endpoint.
    pub fn parse_with_hints(url: &Url) -> Result<(Self, StoreHints)> {
        let invalid = |reason: &str| Error::invalid_location(url.as_str(), reason);
        let mut hints = StoreHints::default();

        let location = match url.scheme() {
            "s3" | "s3a" => Self::bucketed(StorageProvider::Aws, None, host(url)?, url.path())?,
            "gs" => Self::bucketed(StorageProvider::Gcp, None, host(url)?, url.path())?,
            "r2" => {
                let bucket = url.username();
                if bucket.is_empty() {
                    return Err(invalid(
                        "expected r2://bucket@account.r2.cloudflarestorage.com",
                    ));
                }
                let host = host(url)?;
                let account = r2_account(host).ok_or_else(|| invalid("not an R2 host"))?;
                hints = r2_hints(host);
                Self::bucketed(StorageProvider::R2, Some(account), bucket, url.path())?
            }
            "abfs" | "abfss" | "az" | "adl" | "azure" => {
                let container = url.username();
                if container.is_empty() {
                    if matches!(url.scheme(), "abfs" | "abfss") {
                        return Err(invalid("expected container@account.dfs.core.windows.net"));
                    }
                    // `az://container/path`: the account comes from configuration.
                    Self::bucketed(StorageProvider::Azure, None, host(url)?, url.path())?
                } else {
                    let account = azure_account(host(url)?)
                        .ok_or_else(|| invalid("not an Azure storage host"))?;
                    Self::bucketed(StorageProvider::Azure, Some(account), container, url.path())?
                }
            }
            "azurite" => {
                hints.endpoint = Some(AZURITE_BLOB_ENDPOINT.to_owned());
                hints.allow_http = Some(true);
                Self::bucketed(
                    StorageProvider::Azure,
                    Some(AZURITE_ACCOUNT),
                    host(url)?,
                    url.path(),
                )?
            }
            "http" | "https" => Self::parse_http(url, &mut hints)?,
            "file" => {
                if !matches!(url.host_str(), None | Some("") | Some("localhost")) {
                    return Err(invalid("file URLs must not name a remote host"));
                }
                Self::unbucketed(StorageProvider::Local, url)?
            }
            "memory" => Self::unbucketed(StorageProvider::Memory, url)?,
            _ => return Err(invalid("unsupported scheme")),
        };
        Ok((location, hints))
    }

    fn parse_http(url: &Url, hints: &mut StoreHints) -> Result<Self> {
        let invalid = |reason: &str| Error::invalid_location(url.as_str(), reason);
        let host = host(url)?;
        let labels: Vec<&str> = host.split('.').collect();

        // Azurite serves `http://127.0.0.1:10000/<account>/<container>/<path>`.
        if matches!(host, "127.0.0.1" | "localhost" | "[::1]")
            && url.port() == Some(AZURITE_BLOB_PORT)
        {
            let (account, rest) = split_first_segment(url.path());
            let (container, path) = split_first_segment(rest);
            if account.is_empty() || container.is_empty() {
                return Err(invalid("expected an Azurite /account/container path"));
            }
            hints.endpoint = Some(url[..url::Position::BeforePath].to_owned());
            hints.allow_http = Some(url.scheme() == "http");
            return Self::bucketed(StorageProvider::Azure, Some(account), container, path);
        }

        if host.ends_with(".core.windows.net") {
            let account =
                azure_account(host).ok_or_else(|| invalid("not an Azure storage host"))?;
            let (container, path) = split_first_segment(url.path());
            return Self::bucketed(StorageProvider::Azure, Some(account), container, path);
        }

        if let Some(account) = r2_account(host) {
            *hints = r2_hints(host);
            let (bucket, path) = split_first_segment(url.path());
            return Self::bucketed(StorageProvider::R2, Some(account), bucket, path);
        }

        if host.ends_with(".amazonaws.com") {
            // The `s3` label separates the bucket (before it, virtual-hosted) from the
            // regional suffix (after it). `s3-<region>` is the legacy dash form.
            let Some(s3) = labels
                .iter()
                .position(|l| *l == "s3" || is_legacy_regional_s3_label(l))
            else {
                return Err(invalid(
                    "not an S3 endpoint (access points and other S3 hosts are configured through hints, not URLs)",
                ));
            };
            hints.region = s3_region(&labels[s3..]);
            return if s3 == 0 {
                let (bucket, path) = split_first_segment(url.path());
                Self::bucketed(StorageProvider::Aws, None, bucket, path)
            } else {
                Self::bucketed(
                    StorageProvider::Aws,
                    None,
                    labels[..s3].join("."),
                    url.path(),
                )
            };
        }

        if host == "storage.googleapis.com" {
            let (bucket, path) = split_first_segment(url.path());
            return Self::bucketed(StorageProvider::Gcp, None, bucket, path);
        }
        if let Some(bucket) = host.strip_suffix(".storage.googleapis.com") {
            return Self::bucketed(StorageProvider::Gcp, None, bucket, url.path());
        }

        Err(invalid("unrecognized storage host"))
    }

    fn bucketed(
        provider: StorageProvider,
        account: Option<&str>,
        bucket: impl Into<String>,
        url_path: &str,
    ) -> Result<Self> {
        let bucket = bucket.into();
        if bucket.is_empty() {
            return Err(Error::invalid_location(
                url_path,
                "missing bucket or container",
            ));
        }
        Ok(Self {
            provider,
            account: account.map(str::to_owned),
            bucket,
            path: url_to_path(url_path)?,
        })
    }

    fn unbucketed(provider: StorageProvider, url: &Url) -> Result<Self> {
        Ok(Self {
            provider,
            account: None,
            bucket: String::new(),
            path: url_to_path(url.path())?,
        })
    }

    /// The store this location lives in.
    pub fn provider(&self) -> StorageProvider {
        self.provider
    }

    /// The Azure storage account or R2 account, when the location names one.
    pub fn account(&self) -> Option<&str> {
        self.account.as_deref()
    }

    /// The bucket (S3, R2, GCS) or container (Azure); empty for local and in-memory stores.
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// The object path within the bucket, without leading or trailing `/`.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether `self` is `prefix` or lies below it.
    ///
    /// Both must be in the same bucket of the same account, and `prefix`'s path must be a
    /// whole-segment prefix of `self`'s: `b/tab` is a prefix of `b/tab/x`, not of
    /// `b/table2`.
    pub fn starts_with(&self, prefix: &StorageLocation) -> bool {
        self.provider == prefix.provider
            && self.account == prefix.account
            && self.bucket == prefix.bucket
            && self.path.prefix_matches(&prefix.path)
    }

    /// The canonical URL of this location (see the type docs for the forms).
    pub fn to_url(&self) -> Url {
        let base = match (self.provider, &self.account) {
            (StorageProvider::Aws, _) => format!("s3://{}", self.bucket),
            (StorageProvider::R2, Some(account)) => {
                format!("r2://{}@{account}.r2.cloudflarestorage.com", self.bucket)
            }
            (StorageProvider::R2, None) => format!("s3://{}", self.bucket),
            (StorageProvider::Azure, Some(account)) => {
                format!("abfss://{}@{account}.dfs.core.windows.net", self.bucket)
            }
            (StorageProvider::Azure, None) => format!("az://{}", self.bucket),
            (StorageProvider::Gcp, _) => format!("gs://{}", self.bucket),
            (StorageProvider::Local, _) => "file://".to_owned(),
            (StorageProvider::Memory, _) => "memory://".to_owned(),
        };
        let mut url = Url::parse(&base).expect("canonical location base is a valid URL");
        // `set_path` percent-encodes what a URL path cannot carry (`?`, `#`, spaces); the
        // `%` escapes `Path` already uses pass through unchanged, so `parse` round-trips.
        url.set_path(&format!("/{}", self.path));
        url
    }
}

impl fmt::Display for StorageLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.to_url().as_str())
    }
}

impl FromStr for StorageLocation {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let url = Url::parse(s).map_err(|e| Error::invalid_location(s, e.to_string()))?;
        Self::parse(&url)
    }
}

impl TryFrom<&Url> for StorageLocation {
    type Error = Error;

    fn try_from(url: &Url) -> Result<Self> {
        Self::parse(url)
    }
}

/// The subtree of storage a credential covers: a location used as a prefix.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StorageScope(StorageLocation);

impl StorageScope {
    /// A scope covering `location` and everything below it.
    pub fn new(location: StorageLocation) -> Self {
        Self(location)
    }

    /// The location at the root of this scope.
    pub fn location(&self) -> &StorageLocation {
        &self.0
    }

    /// Converts into the location at the root of this scope.
    pub fn into_location(self) -> StorageLocation {
        self.0
    }

    /// Whether `location` lies within this scope (see [`StorageLocation::starts_with`]).
    pub fn covers(&self, location: &StorageLocation) -> bool {
        location.starts_with(&self.0)
    }

    /// Whether `other` lies entirely within this scope.
    pub fn contains(&self, other: &StorageScope) -> bool {
        self.covers(&other.0)
    }

    /// How specific this scope is: the number of path segments below the bucket.
    pub fn depth(&self) -> usize {
        self.0.path.parts().count()
    }
}

impl From<StorageLocation> for StorageScope {
    fn from(location: StorageLocation) -> Self {
        Self(location)
    }
}

impl fmt::Display for StorageScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for StorageScope {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        s.parse().map(Self)
    }
}

/// Returns the value whose scope most specifically covers `location`.
///
/// "Most specific" is the deepest covering scope by path segments; among equally deep
/// candidates the first wins. Returns `None` when no scope covers `location`.
///
/// ```
/// # use olai_storage_credentials::location::{longest_match, StorageLocation, StorageScope};
/// let table: StorageScope = "s3://bucket/warehouse/tab".parse().unwrap();
/// let warehouse: StorageScope = "s3://bucket/warehouse".parse().unwrap();
/// let scopes = [(&warehouse, "warehouse"), (&table, "table")];
///
/// let file: StorageLocation = "s3://bucket/warehouse/tab/part-0.parquet".parse().unwrap();
/// assert_eq!(longest_match(scopes, &file), Some("table"));
///
/// let sibling: StorageLocation = "s3://bucket/warehouse/table2/x".parse().unwrap();
/// assert_eq!(longest_match(scopes, &sibling), Some("warehouse"));
/// ```
pub fn longest_match<'a, T>(
    candidates: impl IntoIterator<Item = (&'a StorageScope, T)>,
    location: &StorageLocation,
) -> Option<T> {
    let mut best: Option<(usize, T)> = None;
    for (scope, value) in candidates {
        if !scope.covers(location) {
            continue;
        }
        let depth = scope.depth();
        if best.as_ref().is_none_or(|(d, _)| depth > *d) {
            best = Some((depth, value));
        }
    }
    best.map(|(_, value)| value)
}

fn host(url: &Url) -> Result<&str> {
    match url.host_str() {
        Some(host) if !host.is_empty() => Ok(host),
        _ => Err(Error::invalid_location(url.as_str(), "missing host")),
    }
}

/// Splits `/first/rest...` into `("first", "rest...")`, leaving `rest` URL-encoded.
fn split_first_segment(path: &str) -> (&str, &str) {
    let path = path.strip_prefix('/').unwrap_or(path);
    path.split_once('/').unwrap_or((path, ""))
}

fn url_to_path(url_path: &str) -> Result<Path> {
    Path::from_url_path(url_path).map_err(|e| Error::invalid_location(url_path, e.to_string()))
}

/// `account.{blob,dfs}.<suffix>` → `account`.
fn azure_account(host: &str) -> Option<&str> {
    let mut labels = host.split('.');
    let account = labels.next().filter(|a| !a.is_empty())?;
    matches!(labels.next(), Some("blob" | "dfs")).then_some(account)
}

/// `account.r2.cloudflarestorage.com` → `account`.
fn r2_account(host: &str) -> Option<&str> {
    host.strip_suffix(".r2.cloudflarestorage.com")
        .filter(|a| !a.is_empty() && !a.contains('.'))
}

fn r2_hints(host: &str) -> StoreHints {
    StoreHints {
        endpoint: Some(format!("https://{host}")),
        // R2 accepts any SigV4 region but documents `auto`.
        region: Some("auto".to_owned()),
        ..StoreHints::default()
    }
}

/// `s3-us-west-2`, the legacy dash form of a regional endpoint. Excludes
/// `s3-accesspoint`, `s3-object-lambda` and similar non-bucket endpoints, whose suffix is
/// not a region (every AWS region name contains a digit).
fn is_legacy_regional_s3_label(label: &str) -> bool {
    label
        .strip_prefix("s3-")
        .is_some_and(|region| region.bytes().any(|b| b.is_ascii_digit()))
}

/// The region in an S3 endpoint, given the labels from the `s3` label onward:
/// `s3.us-west-2.amazonaws.com`, `s3.dualstack.us-west-2.amazonaws.com`,
/// `s3-us-west-2.amazonaws.com`. `s3.amazonaws.com` names no region.
fn s3_region(labels: &[&str]) -> Option<String> {
    let (first, rest) = labels.split_first()?;
    if let Some(region) = first.strip_prefix("s3-") {
        return Some(region.to_owned());
    }
    rest.iter()
        .find(|l| **l != "dualstack")
        .filter(|l| **l != "amazonaws")
        .map(|l| (*l).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(s: &str) -> StorageLocation {
        s.parse()
            .unwrap_or_else(|e| panic!("failed to parse {s}: {e}"))
    }

    fn scope(s: &str) -> StorageScope {
        StorageScope::new(loc(s))
    }

    #[test]
    fn spellings_parse_to_one_canonical_location() {
        let cases: &[(&[&str], &str)] = &[
            (
                &[
                    "s3://bucket/a/b",
                    "s3a://bucket/a/b/",
                    "https://bucket.s3.amazonaws.com/a/b",
                    "https://bucket.s3.us-west-2.amazonaws.com/a/b",
                    "https://bucket.s3.dualstack.us-west-2.amazonaws.com/a/b",
                    "https://bucket.s3-us-west-2.amazonaws.com/a/b",
                    "https://s3.amazonaws.com/bucket/a/b",
                    "https://s3.eu-central-1.amazonaws.com/bucket/a/b",
                ],
                "s3://bucket/a/b",
            ),
            (
                &[
                    "abfss://ctr@acct.dfs.core.windows.net/a/b",
                    "abfs://ctr@acct.dfs.core.windows.net/a/b",
                    "abfss://ctr@acct.blob.core.windows.net/a/b",
                    "az://ctr@acct.dfs.core.windows.net/a/b",
                    "https://acct.blob.core.windows.net/ctr/a/b",
                    "https://acct.dfs.core.windows.net/ctr/a/b/",
                ],
                "abfss://ctr@acct.dfs.core.windows.net/a/b",
            ),
            (
                &[
                    "http://127.0.0.1:10000/devstoreaccount1/ctr/a/b",
                    "http://localhost:10000/devstoreaccount1/ctr/a/b",
                    "azurite://ctr/a/b",
                ],
                "abfss://ctr@devstoreaccount1.dfs.core.windows.net/a/b",
            ),
            (&["az://ctr/a/b", "azure://ctr/a/b"], "az://ctr/a/b"),
            (
                &[
                    "gs://bucket/a/b",
                    "https://storage.googleapis.com/bucket/a/b",
                    "https://bucket.storage.googleapis.com/a/b",
                ],
                "gs://bucket/a/b",
            ),
            (
                &[
                    "r2://bucket@acct.r2.cloudflarestorage.com/a/b",
                    "https://acct.r2.cloudflarestorage.com/bucket/a/b",
                ],
                "r2://bucket@acct.r2.cloudflarestorage.com/a/b",
            ),
            (
                &["file:///tmp/a/b", "file://localhost/tmp/a/b/"],
                "file:///tmp/a/b",
            ),
            (&["memory:///a/b"], "memory:///a/b"),
        ];
        for (spellings, canonical) in cases {
            let expected = loc(canonical);
            assert_eq!(
                expected.to_string(),
                *canonical,
                "canonical form is a fixpoint"
            );
            for spelling in *spellings {
                assert_eq!(loc(spelling), expected, "{spelling}");
            }
        }
    }

    #[test]
    fn abfss_takes_container_from_userinfo() {
        // Mangrove's parser took `p` as the container here, because `host_str()` drops
        // the `container@` userinfo.
        let l = loc("abfss://ctr@acct.dfs.core.windows.net/p/q");
        assert_eq!(l.provider(), StorageProvider::Azure);
        assert_eq!(l.account(), Some("acct"));
        assert_eq!(l.bucket(), "ctr");
        assert_eq!(l.path().as_ref(), "p/q");
    }

    #[test]
    fn dotted_bucket_names_survive_virtual_hosting() {
        let l = loc("https://my.dotted.bucket.s3.us-east-1.amazonaws.com/k");
        assert_eq!(l.bucket(), "my.dotted.bucket");
        assert_eq!(l.path().as_ref(), "k");
    }

    #[test]
    fn hints_from_urls() {
        let parse = |s: &str| {
            StorageLocation::parse_with_hints(&Url::parse(s).unwrap())
                .unwrap()
                .1
        };

        assert_eq!(
            parse("https://bucket.s3.us-west-2.amazonaws.com/k")
                .region
                .as_deref(),
            Some("us-west-2")
        );
        assert_eq!(
            parse("https://bucket.s3-eu-west-1.amazonaws.com/k")
                .region
                .as_deref(),
            Some("eu-west-1")
        );
        assert_eq!(parse("https://bucket.s3.amazonaws.com/k").region, None);

        let azurite = parse("http://127.0.0.1:10000/devstoreaccount1/ctr/k");
        assert_eq!(azurite.endpoint.as_deref(), Some("http://127.0.0.1:10000"));
        assert_eq!(azurite.allow_http, Some(true));

        let r2 = parse("r2://bucket@acct.r2.cloudflarestorage.com/k");
        assert_eq!(
            r2.endpoint.as_deref(),
            Some("https://acct.r2.cloudflarestorage.com")
        );
        assert_eq!(r2.region.as_deref(), Some("auto"));

        assert_eq!(parse("s3://bucket/k"), StoreHints::default());
    }

    #[test]
    fn rejects_unrecognized_locations() {
        for bad in [
            "https://example.com/bucket/key",
            "http://localhost:9000/bucket/key",
            "https://ap-123456789012.s3-accesspoint.us-west-2.amazonaws.com/k",
            "abfss://acct.dfs.core.windows.net/p",
            "abfss://ctr@example.com/p",
            "r2://acct.r2.cloudflarestorage.com/p",
            "ftp://host/p",
            "file://remote-host/p",
            "s3://bucket/a//b",
            "https://acct.blob.core.windows.net/",
        ] {
            assert!(
                bad.parse::<StorageLocation>().is_err(),
                "{bad} should be rejected"
            );
        }
    }

    #[test]
    fn dot_segments_are_resolved_before_matching() {
        // `url` resolves `.` and `..` (also percent-encoded) while parsing, so a location
        // cannot climb out of a scope by spelling.
        assert_eq!(loc("s3://b/tab/../other/x"), loc("s3://b/other/x"));
        assert_eq!(loc("s3://b/tab/%2E%2E/other"), loc("s3://b/other"));
        assert!(!scope("s3://b/tab").covers(&loc("s3://b/tab/../other/x")));
    }

    #[test]
    fn prefix_matching_respects_segment_boundaries() {
        let tab = scope("s3://b/tab");
        assert!(tab.covers(&loc("s3://b/tab")));
        assert!(tab.covers(&loc("s3://b/tab/part-0.parquet")));
        assert!(tab.covers(&loc("s3://b/tab/")));
        assert!(!tab.covers(&loc("s3://b/table2")));
        assert!(!tab.covers(&loc("s3://b/table2/part-0.parquet")));
        assert!(!tab.covers(&loc("s3://b/ta")));
        assert!(!tab.covers(&loc("s3://b")));
        assert!(!tab.covers(&loc("s3://other/tab/x")));
        assert!(!tab.covers(&loc("gs://b/tab/x")));

        let bucket = scope("s3://b");
        assert!(bucket.covers(&loc("s3://b/anything/at/all")));
        assert!(bucket.contains(&tab));
        assert!(!tab.contains(&bucket));
    }

    #[test]
    fn prefix_matching_compares_canonical_forms() {
        let s = scope("https://acct.blob.core.windows.net/ctr/tab");
        assert!(s.covers(&loc("abfss://ctr@acct.dfs.core.windows.net/tab/x")));
        assert!(!s.covers(&loc("abfss://ctr@other.dfs.core.windows.net/tab/x")));
        assert!(!s.covers(&loc("abfss://other@acct.dfs.core.windows.net/tab/x")));
        // An account-less location is not covered by an account-qualified scope.
        assert!(!s.covers(&loc("az://ctr/tab/x")));
        // R2 buckets are per account; an S3 scope never covers them.
        assert!(!scope("s3://bucket").covers(&loc("r2://bucket@acct.r2.cloudflarestorage.com/x")));
    }

    #[test]
    fn percent_encoded_segments_round_trip() {
        for s in [
            "s3://b/with%20space/x",
            "s3://b/q%3Fmark/x",
            "s3://b/hash%23/x",
            "s3://b/pct%25/x",
            "gs://b/caf%C3%A9",
        ] {
            let l = loc(s);
            assert_eq!(loc(&l.to_string()), l, "{s} round-trips");
        }
        let l = loc("s3://b/with%20space/x");
        assert_eq!(l.path().parts().next().unwrap().as_ref(), "with space");
        // An encoded space does not sneak past a segment boundary.
        assert!(!scope("s3://b/with").covers(&l));
    }

    #[test]
    fn longest_match_picks_the_deepest_covering_scope() {
        let bucket = scope("s3://b");
        let warehouse = scope("s3://b/warehouse");
        let table = scope("s3://b/warehouse/tab");
        let other = scope("gs://b/warehouse/tab/deeper");
        let candidates = [(&bucket, 0), (&table, 2), (&warehouse, 1), (&other, 3)];

        assert_eq!(
            longest_match(candidates, &loc("s3://b/warehouse/tab/x")),
            Some(2)
        );
        assert_eq!(
            longest_match(candidates, &loc("s3://b/warehouse/table2/x")),
            Some(1)
        );
        assert_eq!(longest_match(candidates, &loc("s3://b/elsewhere")), Some(0));
        assert_eq!(
            longest_match(candidates, &loc("s3://c/warehouse/tab")),
            None
        );

        let dup = scope("s3://b/warehouse");
        assert_eq!(
            longest_match(
                [(&warehouse, "first"), (&dup, "second")],
                &loc("s3://b/warehouse/x")
            ),
            Some("first")
        );
    }
}
