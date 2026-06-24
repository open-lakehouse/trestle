// @generated — do not edit by hand.
#![allow(unused_mut)]
use futures::{future::BoxFuture, stream::BoxStream, TryStreamExt, StreamExt};
use super::super::stream_paginated;
use std::future::IntoFuture;
use crate::Result;
use example_common::models::catalog::v1::*;
use super::client::*;
/// Builder for creating a catalog
pub struct CreateCatalogBuilder {
    client: CatalogServiceClient,
    request: CreateCatalogRequest,
}
impl CreateCatalogBuilder {
    /// Create a new builder instance.
    /// Obtain via the corresponding method on `CatalogServiceClient`.
    pub(crate) fn new(client: CatalogServiceClient) -> Self {
        let request = CreateCatalogRequest {
            ..Default::default()
        };
        Self { client, request }
    }
    /// Set catalog
    pub fn with_catalog(mut self, catalog: impl Into<Option<Catalog>>) -> Self {
        self.request.catalog = catalog.into();
        self
    }
}
impl IntoFuture for CreateCatalogBuilder {
    type Output = Result<Catalog>;
    type IntoFuture = BoxFuture<'static, Self::Output>;
    fn into_future(self) -> Self::IntoFuture {
        let client = self.client;
        let request = self.request;
        Box::pin(async move { client.create_catalog(&request).await })
    }
}
/// Builder for getting a catalog
pub struct GetCatalogBuilder {
    client: CatalogServiceClient,
    request: GetCatalogRequest,
}
impl GetCatalogBuilder {
    /// Create a new builder instance.
    /// Obtain via the corresponding method on `CatalogServiceClient`.
    pub(crate) fn new(client: CatalogServiceClient, name: impl Into<String>) -> Self {
        let request = GetCatalogRequest {
            name: name.into(),
        };
        Self { client, request }
    }
}
impl IntoFuture for GetCatalogBuilder {
    type Output = Result<Catalog>;
    type IntoFuture = BoxFuture<'static, Self::Output>;
    fn into_future(self) -> Self::IntoFuture {
        let client = self.client;
        let request = self.request;
        Box::pin(async move { client.get_catalog(&request).await })
    }
}
/// Builder for listing catalogs
pub struct ListCatalogsBuilder {
    client: CatalogServiceClient,
    request: ListCatalogsRequest,
}
impl ListCatalogsBuilder {
    /// Create a new builder instance.
    /// Obtain via the corresponding method on `CatalogServiceClient`.
    pub(crate) fn new(
        client: CatalogServiceClient,
        max_results: i32,
        page_token: impl Into<String>,
    ) -> Self {
        let request = ListCatalogsRequest {
            max_results,
            page_token: page_token.into(),
        };
        Self { client, request }
    }
    /// Convert paginated request into stream of results
    pub fn into_stream(self) -> BoxStream<'static, Result<Catalog>> {
        let remaining = self.request.max_results;
        stream_paginated(
                (self, remaining),
                move |(mut builder, mut remaining), page_token| async move {
                    builder.request.page_token = page_token;
                    let res = builder.client.list_catalogs(&builder.request).await?;
                    if let Some(ref mut rem) = remaining {
                        *rem -= res.catalogs.len() as i32;
                    }
                    let next_page_token = if remaining.is_some_and(|r| r <= 0) {
                        None
                    } else {
                        res.next_page_token.clone()
                    };
                    Ok((res, (builder, remaining), next_page_token))
                },
            )
            .map_ok(|resp| futures::stream::iter(resp.catalogs.into_iter().map(Ok)))
            .try_flatten()
            .boxed()
    }
}
impl IntoFuture for ListCatalogsBuilder {
    type Output = Result<ListCatalogsResponse>;
    type IntoFuture = BoxFuture<'static, Self::Output>;
    fn into_future(self) -> Self::IntoFuture {
        let client = self.client;
        let request = self.request;
        Box::pin(async move { client.list_catalogs(&request).await })
    }
}
/// Builder for updating a catalog
pub struct UpdateCatalogBuilder {
    client: CatalogServiceClient,
    request: UpdateCatalogRequest,
}
impl UpdateCatalogBuilder {
    /// Create a new builder instance.
    /// Obtain via the corresponding method on `CatalogServiceClient`.
    pub(crate) fn new(client: CatalogServiceClient, name: impl Into<String>) -> Self {
        let request = UpdateCatalogRequest {
            name: name.into(),
            ..Default::default()
        };
        Self { client, request }
    }
    /// Set catalog
    pub fn with_catalog(mut self, catalog: impl Into<Option<Catalog>>) -> Self {
        self.request.catalog = catalog.into();
        self
    }
}
impl IntoFuture for UpdateCatalogBuilder {
    type Output = Result<Catalog>;
    type IntoFuture = BoxFuture<'static, Self::Output>;
    fn into_future(self) -> Self::IntoFuture {
        let client = self.client;
        let request = self.request;
        Box::pin(async move { client.update_catalog(&request).await })
    }
}
/// Builder for deleting a catalog
pub struct DeleteCatalogBuilder {
    client: CatalogServiceClient,
    request: DeleteCatalogRequest,
}
impl DeleteCatalogBuilder {
    /// Create a new builder instance.
    /// Obtain via the corresponding method on `CatalogServiceClient`.
    pub(crate) fn new(client: CatalogServiceClient, name: impl Into<String>) -> Self {
        let request = DeleteCatalogRequest {
            name: name.into(),
        };
        Self { client, request }
    }
}
impl IntoFuture for DeleteCatalogBuilder {
    type Output = Result<DeleteCatalogResponse>;
    type IntoFuture = BoxFuture<'static, Self::Output>;
    fn into_future(self) -> Self::IntoFuture {
        let client = self.client;
        let request = self.request;
        Box::pin(async move { client.delete_catalog(&request).await })
    }
}
/// Builder for catalog token
pub struct GenerateCatalogTokenBuilder {
    client: CatalogServiceClient,
    request: GenerateCatalogTokenRequest,
}
impl GenerateCatalogTokenBuilder {
    /// Create a new builder instance.
    /// Obtain via the corresponding method on `CatalogServiceClient`.
    pub(crate) fn new(
        client: CatalogServiceClient,
        catalog_id: impl Into<String>,
    ) -> Self {
        let request = GenerateCatalogTokenRequest {
            catalog_id: catalog_id.into(),
        };
        Self { client, request }
    }
}
impl IntoFuture for GenerateCatalogTokenBuilder {
    type Output = Result<CatalogToken>;
    type IntoFuture = BoxFuture<'static, Self::Output>;
    fn into_future(self) -> Self::IntoFuture {
        let client = self.client;
        let request = self.request;
        Box::pin(async move { client.generate_catalog_token(&request).await })
    }
}
/// Builder for catalog status
pub struct GetCatalogStatusBuilder {
    client: CatalogServiceClient,
    request: GetCatalogStatusRequest,
}
impl GetCatalogStatusBuilder {
    /// Create a new builder instance.
    /// Obtain via the corresponding method on `CatalogServiceClient`.
    pub(crate) fn new(client: CatalogServiceClient, name: impl Into<String>) -> Self {
        let request = GetCatalogStatusRequest {
            name: name.into(),
        };
        Self { client, request }
    }
}
impl IntoFuture for GetCatalogStatusBuilder {
    type Output = Result<CatalogStatus>;
    type IntoFuture = BoxFuture<'static, Self::Output>;
    fn into_future(self) -> Self::IntoFuture {
        let client = self.client;
        let request = self.request;
        Box::pin(async move { client.get_catalog_status(&request).await })
    }
}
