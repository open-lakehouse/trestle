// @generated — do not edit by hand.
#![allow(unused_mut)]
use futures::{future::BoxFuture, stream::BoxStream, TryStreamExt, StreamExt};
use std::future::IntoFuture;
use crate::Result;
use super::super::stream_paginated;
use example_common::models::catalog::v1::*;
use super::client::*;
/// Builder for by tags
pub struct ListByTagsBuilder {
    client: SchemaClient,
    request: ListByTagsRequest,
}
impl ListByTagsBuilder {
    /// Create a new builder instance.
    /// Obtain via the corresponding method on `SchemaClient`.
    pub(crate) fn new(
        client: SchemaClient,
        tags: Vec<impl Into<String>>,
        max_results: i32,
    ) -> Self {
        let request = ListByTagsRequest {
            tags: tags.into(),
            max_results,
            ..Default::default()
        };
        Self { client, request }
    }
}
impl IntoFuture for ListByTagsBuilder {
    type Output = Result<ListByTagsResponse>;
    type IntoFuture = BoxFuture<'static, Self::Output>;
    fn into_future(self) -> Self::IntoFuture {
        let client = self.client;
        let request = self.request;
        Box::pin(async move { client.list_by_tags(&request).await })
    }
}
/// Builder for by catalog type
pub struct ListByCatalogTypeBuilder {
    client: SchemaClient,
    request: ListByCatalogTypeRequest,
}
impl ListByCatalogTypeBuilder {
    /// Create a new builder instance.
    /// Obtain via the corresponding method on `SchemaClient`.
    pub(crate) fn new(client: SchemaClient, catalog_type: CatalogType) -> Self {
        let request = ListByCatalogTypeRequest {
            catalog_type: catalog_type as i32,
            ..Default::default()
        };
        Self { client, request }
    }
}
impl IntoFuture for ListByCatalogTypeBuilder {
    type Output = Result<ListByTagsResponse>;
    type IntoFuture = BoxFuture<'static, Self::Output>;
    fn into_future(self) -> Self::IntoFuture {
        let client = self.client;
        let request = self.request;
        Box::pin(async move { client.list_by_catalog_type(&request).await })
    }
}
