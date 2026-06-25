// @generated — do not edit by hand.
#![allow(unused_mut)]
#![allow(unused_imports)]
type BoxFut<'a, T> = ::futures::future::BoxFuture<'a, T>;
use std::future::IntoFuture;
use crate::Result;
use example_common::models::catalog::v1::*;
use super::client::*;
/// Builder for by tags
pub struct ListByTagsBuilder {
    client: QueryClient,
    request: ListByTagsRequest,
}
impl ListByTagsBuilder {
    /// Create a new builder instance.
    /// Obtain via the corresponding method on `QueryClient`.
    pub(crate) fn new(
        client: QueryClient,
        tags: Vec<impl Into<String>>,
        max_results: i32,
    ) -> Self {
        let request = ListByTagsRequest {
            tags: tags.into(),
            max_results,
        };
        Self { client, request }
    }
}
impl IntoFuture for ListByTagsBuilder {
    type Output = Result<ListByTagsResponse>;
    type IntoFuture = BoxFut<'static, Self::Output>;
    fn into_future(self) -> Self::IntoFuture {
        let client = self.client;
        let request = self.request;
        Box::pin(async move { client.list_by_tags(&request).await })
    }
}
/// Builder for by catalog type
pub struct ListByCatalogTypeBuilder {
    client: QueryClient,
    request: ListByCatalogTypeRequest,
}
impl ListByCatalogTypeBuilder {
    /// Create a new builder instance.
    /// Obtain via the corresponding method on `QueryClient`.
    pub(crate) fn new(client: QueryClient, catalog_type: CatalogType) -> Self {
        let request = ListByCatalogTypeRequest {
            catalog_type: catalog_type as i32,
        };
        Self { client, request }
    }
}
impl IntoFuture for ListByCatalogTypeBuilder {
    type Output = Result<ListByTagsResponse>;
    type IntoFuture = BoxFut<'static, Self::Output>;
    fn into_future(self) -> Self::IntoFuture {
        let client = self.client;
        let request = self.request;
        Box::pin(async move { client.list_by_catalog_type(&request).await })
    }
}
