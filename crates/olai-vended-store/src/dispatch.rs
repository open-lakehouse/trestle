//! Building stores from securable references, across the enabled protocols.

use std::sync::Arc;

use object_store::ObjectStore;
use object_store::path::Path;
use url::Url;

use crate::error::{Error, Result};

/// The vending protocol a reference belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Protocol {
    /// Unity Catalog REST: `uc://...` and `vol+dbfs:/Volumes/...` (feature `unity`).
    #[cfg(feature = "unity")]
    Unity,
}

impl Protocol {
    /// The protocol whose reference grammar `url` uses, among the enabled ones.
    ///
    /// Storage URLs (`s3://`, `abfss://`, ...) belong to none: they are not securable
    /// references. Build their stores with object_store, or through a protocol that
    /// vends for paths (Unity Catalog's path credentials, for example). Protocols
    /// without a URL grammar are configured through their builders only.
    pub fn of(url: &Url) -> Option<Self> {
        match url.scheme() {
            #[cfg(feature = "unity")]
            "uc" | "vol+dbfs" => Some(Self::Unity),
            _ => None,
        }
    }
}

/// Builds the store `url` refers to, configured by `options`, in the style of
/// `object_store::parse_url_opts`.
///
/// Returns the bucket-rooted store and the path within it that `url` names, which is
/// the prefix the vended credential covers. `options` is a storage-options map: each
/// protocol takes the keys it knows (see each protocol's builder) and ignores the rest.
///
/// Unlike object_store's, this is `async`: it vends a credential.
pub async fn parse_url_opts<I, K, V>(url: &Url, options: I) -> Result<(Arc<dyn ObjectStore>, Path)>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: Into<String>,
{
    match Protocol::of(url) {
        #[cfg(feature = "unity")]
        Some(Protocol::Unity) => {
            let store = crate::unity::UnityBuilder::new()
                .with_options(options)
                .build()?
                .open(url.as_str())
                .await?;
            Ok((store.root(), store.prefix().clone()))
        }
        None => Err(Error::InvalidReference(format!(
            "`{url}` is not a reference of an enabled vending protocol"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_urls_belong_to_no_protocol() {
        for url in [
            "s3://bucket/t",
            "abfss://c@a.dfs.core.windows.net/t",
            "file:///t",
        ] {
            assert_eq!(Protocol::of(&url.parse().unwrap()), None, "{url}");
        }
    }

    #[cfg(feature = "unity")]
    #[test]
    fn unity_references() {
        for url in [
            "uc:///Tables/c/s/t",
            "uc://c.s.t",
            "vol+dbfs:/Volumes/c/s/v/p",
        ] {
            assert_eq!(
                Protocol::of(&url.parse().unwrap()),
                Some(Protocol::Unity),
                "{url}"
            );
        }
    }

    #[tokio::test]
    async fn unknown_references_are_rejected() {
        let url = "s3://bucket/t".parse().unwrap();
        let err = parse_url_opts(&url, [("aws_region", "us-east-1")])
            .await
            .expect_err("not a vending reference");
        assert!(matches!(err, Error::InvalidReference(_)), "{err}");
    }
}
