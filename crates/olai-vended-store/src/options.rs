//! Configuration keys every protocol builder shares: the stores' S3 region and endpoint,
//! their HTTP client keys, and the requested access level.

use olai_storage_credentials::AccessLevel;
use url::Url;

use crate::error::{Error, Result};
use crate::store::StoreOptions;

/// The store keys of a storage-options map.
///
/// | Key | Meaning |
/// |---|---|
/// | `aws_region` | The region of vended S3 locations, overriding hints and discovery |
/// | `aws_endpoint`, `aws_endpoint_url` | The S3 endpoint, for servers backed by an S3-compatible store |
/// | `allow_local_storage` | Accept `file://` and `memory://` locations the catalog names; see [`StoreOptions::with_local_storage`] |
/// | client keys (`allow_http`, `timeout`, ...) | Apply to the stores, and to the vending server |
#[derive(Debug, Clone, Default)]
pub(crate) struct StoreKeys {
    aws_region: Option<String>,
    aws_endpoint: Option<String>,
    allow_local_storage: Option<bool>,
    #[cfg(any(feature = "aws", feature = "azure", feature = "gcp"))]
    client: Vec<(object_store::ClientConfigKey, String)>,
}

impl StoreKeys {
    /// Takes `key` if it configures only the stores, and returns whether it did. Client
    /// keys are recorded but not taken: they configure the vending server's client too.
    pub(crate) fn set(&mut self, key: &str, value: &str) -> bool {
        match key {
            "aws_region" => self.aws_region = Some(value.to_owned()),
            "aws_endpoint" | "aws_endpoint_url" => self.aws_endpoint = Some(value.to_owned()),
            "allow_local_storage" => {
                self.allow_local_storage = Some(crate::auth::is_truthy(value));
            }
            _ => {
                #[cfg(any(feature = "aws", feature = "azure", feature = "gcp"))]
                if let Ok(client_key) = key.parse::<object_store::ClientConfigKey>() {
                    self.client.retain(|(existing, _)| *existing != client_key);
                    self.client.push((client_key, value.to_owned()));
                }
                return false;
            }
        }
        true
    }

    /// `options` with these keys applied on top.
    pub(crate) fn apply(&self, mut options: StoreOptions) -> StoreOptions {
        if let Some(allow) = self.allow_local_storage {
            options = options.with_local_storage(allow);
        }
        #[cfg(any(feature = "aws", feature = "azure", feature = "gcp"))]
        if !self.client.is_empty() {
            let mut client_options = object_store::ClientOptions::new();
            for (key, value) in &self.client {
                client_options = client_options.with_config(*key, value);
            }
            options = options.with_client_options(client_options);
        }
        #[cfg(feature = "aws")]
        {
            if let Some(region) = &self.aws_region {
                options = options.with_aws_region(region);
            }
            if let Some(endpoint) = &self.aws_endpoint {
                options = options.with_s3_endpoint(endpoint);
            }
        }
        options
    }
}

/// Parses the access-level key `key`: `read` or `read_write`, `None` when unset.
#[cfg_attr(
    not(any(feature = "unity", feature = "iceberg", feature = "neon")),
    allow(dead_code)
)]
pub(crate) fn parse_access(key: &str, value: Option<&str>) -> Result<Option<AccessLevel>> {
    match value {
        None => Ok(None),
        Some("read") => Ok(Some(AccessLevel::Read)),
        Some("read_write") => Ok(Some(AccessLevel::ReadWrite)),
        Some(other) => Err(Error::Config(format!(
            "`{key}` must be `read` or `read_write`, got `{other}`"
        ))),
    }
}

/// The key value of `access`, as [`parse_access`] reads it.
#[cfg_attr(
    not(any(feature = "unity", feature = "iceberg", feature = "neon")),
    allow(dead_code)
)]
pub(crate) fn access_key_value(access: AccessLevel) -> &'static str {
    match access {
        AccessLevel::Read => "read",
        AccessLevel::ReadWrite => "read_write",
    }
}

/// Parses the URL key `key`, with a trailing `/` so relative paths stay below it.
pub(crate) fn parse_base_url(key: &str, value: &str) -> Result<Url> {
    let url =
        Url::parse(value).map_err(|e| Error::Config(format!("invalid `{key}` `{value}`: {e}")))?;
    Ok(crate::api::with_trailing_slash(url))
}
