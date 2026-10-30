//! A minimal Unity Catalog REST client: securable lookups and credential vending.

use std::fmt;

use olai_http::CloudClient;
use olai_storage_credentials::wire::unity::TemporaryCredential;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::api::{ApiClient, Dialect};
use crate::error::Result;

/// Unity Catalog errors: `{"error_code": "...", "message": "..."}`.
const DIALECT: Dialect = Dialect {
    service: "Unity Catalog",
    envelope: |body| {
        #[derive(Deserialize)]
        struct Envelope {
            error_code: Option<String>,
            message: Option<String>,
        }
        serde_json::from_str::<Envelope>(body)
            .ok()
            .map(|e| (e.error_code, e.message))
    },
};

/// A client for the Unity Catalog endpoints vending needs: `GET tables/{name}`,
/// `GET volumes/{name}` and `POST temporary-{table,volume,path}-credentials`.
///
/// Authentication, retries and the I/O runtime belong to the wrapped [`CloudClient`].
/// Cheap to clone.
#[derive(Clone)]
pub struct UnityClient {
    api: ApiClient,
}

impl UnityClient {
    /// A client for the API rooted at `base_url`, for example
    /// `https://<workspace>/api/2.1/unity-catalog/`.
    pub fn new(client: CloudClient, base_url: Url) -> Self {
        Self {
            api: ApiClient::new(client, base_url, DIALECT),
        }
    }

    /// The API base URL.
    pub fn base_url(&self) -> &Url {
        self.api.base_url()
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
        self.api.send(self.api.get(&[collection, name])?).await
    }

    /// `GET` below the base URL, each of `segments` escaped as one path segment.
    #[cfg_attr(not(feature = "delta-api"), allow(dead_code))]
    pub(crate) async fn get_segments<T: DeserializeOwned>(
        &self,
        segments: &[&str],
        query: &[(&str, &str)],
    ) -> Result<T> {
        self.api.send(self.api.get(segments)?.query(query)).await
    }

    async fn post<B: Serialize, T: DeserializeOwned>(&self, endpoint: &str, body: &B) -> Result<T> {
        self.api.send(self.api.post(&[endpoint])?.json(body)).await
    }
}

impl fmt::Debug for UnityClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnityClient")
            .field("base_url", &self.base_url().as_str())
            .finish_non_exhaustive()
    }
}

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
