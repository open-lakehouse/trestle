//! A minimal Unity Catalog REST client: securable lookups and credential vending.

use std::error::Error as StdError;
use std::fmt;

use olai_http::{CloudClient, RetryError};
use olai_storage_credentials::wire::unity::TemporaryCredential;
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::{Error, Result};

/// Characters escaped in one URL path segment, so a securable name can never introduce
/// a new segment.
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

/// A client for the Unity Catalog endpoints vending needs: `GET tables/{name}`,
/// `GET volumes/{name}` and `POST temporary-{table,volume,path}-credentials`.
///
/// Authentication, retries and the I/O runtime belong to the wrapped [`CloudClient`].
/// Cheap to clone.
#[derive(Clone)]
pub struct UnityClient {
    client: CloudClient,
    base_url: Url,
}

impl UnityClient {
    /// A client for the API rooted at `base_url`, for example
    /// `https://<workspace>/api/2.1/unity-catalog/`.
    pub fn new(client: CloudClient, mut base_url: Url) -> Self {
        if !base_url.path().ends_with('/') {
            let path = format!("{}/", base_url.path());
            base_url.set_path(&path);
        }
        Self { client, base_url }
    }

    /// The API base URL.
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// Looks up a table by its `catalog.schema.table` name.
    pub async fn get_table(&self, full_name: &str) -> Result<TableInfo> {
        self.get("tables", full_name).await
    }

    /// Looks up a volume by its `catalog.schema.volume` name.
    pub async fn get_volume(&self, full_name: &str) -> Result<VolumeInfo> {
        self.get("volumes", full_name).await
    }

    /// Vends credentials for a table.
    pub async fn temporary_table_credentials(
        &self,
        table_id: &str,
        operation: TableOperation,
    ) -> Result<TemporaryCredential> {
        #[derive(Serialize)]
        struct Request<'a> {
            table_id: &'a str,
            operation: TableOperation,
        }
        self.post(
            "temporary-table-credentials",
            &Request {
                table_id,
                operation,
            },
        )
        .await
    }

    /// Vends credentials for a volume.
    pub async fn temporary_volume_credentials(
        &self,
        volume_id: &str,
        operation: VolumeOperation,
    ) -> Result<TemporaryCredential> {
        #[derive(Serialize)]
        struct Request<'a> {
            volume_id: &'a str,
            operation: VolumeOperation,
        }
        self.post(
            "temporary-volume-credentials",
            &Request {
                volume_id,
                operation,
            },
        )
        .await
    }

    /// Vends credentials for a storage URL governed by an external location.
    pub async fn temporary_path_credentials(
        &self,
        url: &Url,
        operation: PathOperation,
    ) -> Result<TemporaryCredential> {
        #[derive(Serialize)]
        struct Request<'a> {
            url: &'a str,
            operation: PathOperation,
        }
        self.post(
            "temporary-path-credentials",
            &Request {
                url: url.as_str(),
                operation,
            },
        )
        .await
    }

    async fn get<T: DeserializeOwned>(&self, collection: &str, name: &str) -> Result<T> {
        let name = utf8_percent_encode(name, PATH_SEGMENT);
        let url = self.url(&format!("{collection}/{name}"))?;
        decode(self.client.get(url).send().await).await
    }

    async fn post<B: Serialize, T: DeserializeOwned>(&self, endpoint: &str, body: &B) -> Result<T> {
        let url = self.url(endpoint)?;
        decode(self.client.post(url).json(body).send().await).await
    }

    fn url(&self, path: &str) -> Result<Url> {
        self.base_url.join(path).map_err(Error::vend)
    }
}

impl fmt::Debug for UnityClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnityClient")
            .field("base_url", &self.base_url.as_str())
            .finish_non_exhaustive()
    }
}

/// Decodes a success body, or turns the failure into an [`ApiError`].
///
/// olai-http reports non-success statuses as errors carrying a [`RetryError`] (and, with
/// its `recording` feature, as responses), so both are handled.
async fn decode<T: DeserializeOwned>(sent: olai_http::Result<reqwest::Response>) -> Result<T> {
    let response = match sent {
        Ok(response) => response,
        Err(err) => return Err(Error::vend(ApiError::from_transport(&err))),
    };
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(Error::vend(ApiError::from_response(
            Some(status.as_u16()),
            &body,
        )));
    }
    response.json().await.map_err(|e| {
        Error::vend(ApiError {
            status: Some(status.as_u16()),
            error_code: None,
            message: format!("invalid response body: {e}"),
        })
    })
}

/// A failed Unity Catalog request.
///
/// Carries the HTTP status and UC's `error_code` when the server answered, so callers
/// can tell a permission denial from an outage. Find it in an [`Error::Vend`] with
/// [`ApiError::find`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    /// The HTTP status, if the server answered.
    pub status: Option<u16>,
    /// UC's error code, such as `PERMISSION_DENIED`.
    pub error_code: Option<String>,
    /// What went wrong.
    pub message: String,
}

impl ApiError {
    fn from_response(status: Option<u16>, body: &str) -> Self {
        #[derive(Deserialize)]
        struct Envelope {
            error_code: Option<String>,
            message: Option<String>,
        }
        match serde_json::from_str::<Envelope>(body) {
            Ok(envelope) => Self {
                status,
                error_code: envelope.error_code,
                message: envelope.message.unwrap_or_default(),
            },
            Err(_) => Self {
                status,
                error_code: None,
                message: body.chars().take(512).collect(),
            },
        }
    }

    fn from_transport(err: &olai_http::Error) -> Self {
        let mut source: Option<&(dyn StdError + 'static)> = Some(err);
        while let Some(current) = source {
            if let Some(retry) = current.downcast_ref::<RetryError>() {
                return Self::from_response(
                    retry.status().map(|s| s.as_u16()),
                    retry.body().unwrap_or_default(),
                );
            }
            source = current.source();
        }
        Self {
            status: None,
            error_code: None,
            message: err.to_string(),
        }
    }

    /// The UC error inside `err`, if it came from a Unity Catalog request.
    pub fn find(err: &Error) -> Option<&ApiError> {
        match err {
            Error::Vend { source } => source.downcast_ref(),
            _ => None,
        }
    }

    /// Whether the server denied the request: HTTP 403 or `PERMISSION_DENIED`.
    pub fn is_permission_denied(&self) -> bool {
        self.status == Some(403) || self.error_code.as_deref() == Some("PERMISSION_DENIED")
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Unity Catalog request failed")?;
        match (self.status, &self.error_code) {
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

/// The fields of a `TableInfo` vending needs.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TableInfo {
    /// The table id credentials are vended for.
    pub table_id: String,
    /// The table's storage root.
    pub storage_location: Option<String>,
}

/// The fields of a `VolumeInfo` vending needs.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct VolumeInfo {
    /// The volume id credentials are vended for.
    pub volume_id: String,
    /// The volume's storage root.
    pub storage_location: Option<String>,
}

/// The operation of a table credential request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum TableOperation {
    /// `READ`
    #[serde(rename = "READ")]
    Read,
    /// `READ_WRITE`
    #[serde(rename = "READ_WRITE")]
    ReadWrite,
}

/// The operation of a volume credential request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum VolumeOperation {
    /// `READ_VOLUME`
    #[serde(rename = "READ_VOLUME")]
    Read,
    /// `WRITE_VOLUME`
    #[serde(rename = "WRITE_VOLUME")]
    ReadWrite,
}

/// The operation of a path credential request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum PathOperation {
    /// `PATH_READ`
    #[serde(rename = "PATH_READ")]
    Read,
    /// `PATH_READ_WRITE`
    #[serde(rename = "PATH_READ_WRITE")]
    ReadWrite,
}

macro_rules! from_access {
    ($($ty:ident),*) => {
        $(impl From<crate::AccessLevel> for $ty {
            fn from(access: crate::AccessLevel) -> Self {
                match access {
                    crate::AccessLevel::Read => Self::Read,
                    crate::AccessLevel::ReadWrite => Self::ReadWrite,
                }
            }
        })*
    };
}

from_access!(TableOperation, VolumeOperation, PathOperation);
