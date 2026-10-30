//! What every protocol client shares: JSON requests below an API base URL, and the
//! errors vending servers answer with.

use std::error::Error as StdError;
use std::fmt;

use olai_http::{CloudClient, CloudRequestBuilder, RetryError};
use olai_storage_credentials::{
    AccessLevel, StorageCredentials, StorageLocation, StorageProvider, StorageScope, StoreHints,
    VendedCredential,
};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde::de::DeserializeOwned;
use url::Url;

use crate::error::{Error, Result};
use crate::source::CredentialSource;

/// Characters escaped in one URL path segment, so a name can never introduce a new
/// segment.
const PATH_SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'/')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}');

/// An error body's `(code, message)`, either of which a protocol may omit.
pub(crate) type Envelope = (Option<String>, Option<String>);

/// How a protocol's servers report errors.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Dialect {
    /// The server kind, for messages: `Unity Catalog`, `Iceberg REST`, ...
    pub(crate) service: &'static str,
    /// Reads `(code, message)` from an error body, if it has the protocol's envelope.
    pub(crate) envelope: fn(&str) -> Option<Envelope>,
}

/// An authenticated client for the JSON API below `base_url`.
///
/// Authentication, retries and the I/O runtime belong to the wrapped [`CloudClient`].
/// Cheap to clone.
#[derive(Clone)]
pub(crate) struct ApiClient {
    client: CloudClient,
    base_url: Url,
    dialect: Dialect,
}

impl ApiClient {
    /// A client for the API rooted at `base_url`; a trailing `/` is added if missing,
    /// so relative paths stay below it.
    pub(crate) fn new(client: CloudClient, base_url: Url, dialect: Dialect) -> Self {
        Self {
            client,
            base_url: with_trailing_slash(base_url),
            dialect,
        }
    }

    pub(crate) fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// The same client for the API rooted at `base_url`.
    #[cfg_attr(not(feature = "iceberg"), allow(dead_code))]
    pub(crate) fn with_base_url(mut self, base_url: Url) -> Self {
        self.base_url = with_trailing_slash(base_url);
        self
    }

    /// The URL of `segments` below the base, each escaped as one path segment.
    pub(crate) fn url(&self, segments: &[&str]) -> Result<Url> {
        let path = segments
            .iter()
            .map(|s| utf8_percent_encode(s, PATH_SEGMENT).to_string())
            .collect::<Vec<_>>()
            .join("/");
        self.base_url.join(&path).map_err(Error::vend)
    }

    /// A `GET` request for `segments` below the base.
    pub(crate) fn get(&self, segments: &[&str]) -> Result<CloudRequestBuilder> {
        Ok(self.client.get(self.url(segments)?))
    }

    /// A `POST` request for `segments` below the base.
    #[cfg_attr(not(feature = "unity"), allow(dead_code))]
    pub(crate) fn post(&self, segments: &[&str]) -> Result<CloudRequestBuilder> {
        Ok(self.client.post(self.url(segments)?))
    }

    /// Sends `request` and decodes a success body, or turns the failure into an
    /// [`ApiError`].
    ///
    /// olai-http reports non-success statuses as errors carrying a [`RetryError`]
    /// (and, with its `recording` feature, as responses), so both are handled.
    pub(crate) async fn send<T: DeserializeOwned>(
        &self,
        request: CloudRequestBuilder,
    ) -> Result<T> {
        let response = match request.send().await {
            Ok(response) => response,
            Err(err) => return Err(ApiError::from_transport(self.dialect, &err).into()),
        };
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(ApiError::from_response(self.dialect, Some(status.as_u16()), &body).into());
        }
        response.json().await.map_err(|e| {
            Error::vend(format!(
                "{} returned an invalid response body: {e}",
                self.dialect.service
            ))
        })
    }
}

impl fmt::Debug for ApiClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiClient")
            .field("service", &self.dialect.service)
            .field("base_url", &self.base_url.as_str())
            .finish_non_exhaustive()
    }
}

/// `url` with a trailing `/` on its path.
pub(crate) fn with_trailing_slash(mut url: Url) -> Url {
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    url
}

/// A request a vending server refused or could not answer.
///
/// Carries the HTTP status and the protocol's error code when the server answered, so
/// callers can tell a permission denial from an outage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    /// The server kind: `Unity Catalog`, `Iceberg REST`, ...
    pub service: &'static str,
    /// The HTTP status, if the server answered.
    pub status: Option<u16>,
    /// The protocol's error code, such as UC's `PERMISSION_DENIED`.
    pub code: Option<String>,
    /// What went wrong.
    pub message: String,
}

impl ApiError {
    fn from_response(dialect: Dialect, status: Option<u16>, body: &str) -> Self {
        let (code, message) = (dialect.envelope)(body)
            .unwrap_or_else(|| (None, Some(body.chars().take(512).collect())));
        Self {
            service: dialect.service,
            status,
            code,
            message: message.unwrap_or_default(),
        }
    }

    fn from_transport(dialect: Dialect, err: &olai_http::Error) -> Self {
        let mut source: Option<&(dyn StdError + 'static)> = Some(err);
        while let Some(current) = source {
            if let Some(retry) = current.downcast_ref::<RetryError>() {
                return Self::from_response(
                    dialect,
                    retry.status().map(|s| s.as_u16()),
                    retry.body().unwrap_or_default(),
                );
            }
            source = current.source();
        }
        Self {
            service: dialect.service,
            status: None,
            code: None,
            message: err.to_string(),
        }
    }

    /// The server's answer inside `err`, if it came from a vending server.
    pub fn find(err: &Error) -> Option<&ApiError> {
        match err {
            Error::Api(api) => Some(api),
            _ => None,
        }
    }

    /// Whether the server denied the request: HTTP 403 or `PERMISSION_DENIED`.
    pub fn is_permission_denied(&self) -> bool {
        self.status == Some(403) || self.code.as_deref() == Some("PERMISSION_DENIED")
    }

    /// Whether the server does not know the endpoint or resource: HTTP 404, 405 or 501.
    pub fn is_not_found(&self) -> bool {
        matches!(self.status, Some(404 | 405 | 501))
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} request failed", self.service)?;
        match (self.status, &self.code) {
            (Some(status), Some(code)) => write!(f, " ({status} {code})")?,
            (Some(status), None) => write!(f, " ({status})")?,
            (None, Some(code)) => write!(f, " ({code})")?,
            (None, None) => {}
        }
        if !self.message.is_empty() {
            write!(f, ": {}", self.message)?;
        }
        Ok(())
    }
}

impl StdError for ApiError {}

/// Vends the seed for a source bound to read-write, rebinding it to read when the
/// server denies that; any other failure is returned.
///
/// This is how protocol modules open a reference without a configured access level:
/// they look the securable up once, and try the wider grant first.
#[cfg_attr(not(feature = "unity"), allow(dead_code))]
pub(crate) async fn fetch_or_read_only<S: CredentialSource>(
    source: S,
    rebind: impl FnOnce(S, AccessLevel) -> S,
) -> Result<(S, VendedCredential)> {
    match source.fetch().await {
        Ok(seed) => Ok((source, seed)),
        Err(err) if ApiError::find(&err).is_some_and(ApiError::is_permission_denied) => {
            let source = rebind(source, AccessLevel::Read);
            let seed = source.fetch().await?;
            Ok((source, seed))
        }
        Err(err) => Err(err),
    }
}

/// The storage root a server named for a securable, with the hints its URL implies (an
/// emulator endpoint, a regional S3 host).
pub(crate) fn storage_root(
    service: &str,
    storage_location: &str,
) -> Result<(StorageScope, StoreHints)> {
    let url = Url::parse(storage_location).map_err(|e| {
        Error::InvalidReference(format!(
            "{service} returned an invalid storage location `{storage_location}`: {e}"
        ))
    })?;
    let (location, hints) = StorageLocation::parse_with_hints(&url)?;
    Ok((StorageScope::new(location), hints))
}

/// The credential for local (`file://`) storage, which needs no vend; `None` for other
/// storage.
pub(crate) fn local_credential(
    scope: &StorageScope,
    access: AccessLevel,
) -> Option<VendedCredential> {
    (scope.location().provider() == StorageProvider::Local)
        .then(|| VendedCredential::new(StorageCredentials::None, scope.clone(), access))
}
