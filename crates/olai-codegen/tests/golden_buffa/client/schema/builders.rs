// @generated — do not edit by hand.
#![allow(unused_mut)]
#![allow(unused_imports)]
type BoxFut<'a, T> = ::futures::future::BoxFuture<'a, T>;
type BoxStr<'a, T> = ::futures::stream::BoxStream<'a, T>;
use futures::{TryStreamExt, StreamExt};
use super::super::stream_paginated;
use std::future::IntoFuture;
use crate::Result;
use example_common::models::schemas::v1::*;
use super::client::*;
/// Builder for creating a schema
pub struct CreateSchemaBuilder {
    client: SchemaServiceClient,
    request: CreateSchemaRequest,
}
impl CreateSchemaBuilder {
    /// Create a new builder instance.
    /// Obtain via the corresponding method on `SchemaServiceClient`.
    pub(crate) fn new(
        client: SchemaServiceClient,
        name: impl Into<String>,
        catalog_name: impl Into<String>,
        schema_type: SchemaType,
    ) -> Self {
        let request = CreateSchemaRequest {
            name: name.into(),
            catalog_name: catalog_name.into(),
            schema_type: buffa::EnumValue::Known(schema_type),
            ..Default::default()
        };
        Self { client, request }
    }
}
impl IntoFuture for CreateSchemaBuilder {
    type Output = Result<Schema>;
    type IntoFuture = BoxFut<'static, Self::Output>;
    fn into_future(self) -> Self::IntoFuture {
        let client = self.client;
        let request = self.request;
        Box::pin(async move { client.create_schema(&request).await })
    }
}
/// Builder for getting a schema
pub struct GetSchemaBuilder {
    client: SchemaServiceClient,
    request: GetSchemaRequest,
}
impl GetSchemaBuilder {
    /// Create a new builder instance.
    /// Obtain via the corresponding method on `SchemaServiceClient`.
    pub(crate) fn new(
        client: SchemaServiceClient,
        full_name: impl Into<String>,
        view: get_schema_request::View,
    ) -> Self {
        let request = GetSchemaRequest {
            full_name: full_name.into(),
            view: buffa::EnumValue::Known(view),
            ..Default::default()
        };
        Self { client, request }
    }
}
impl IntoFuture for GetSchemaBuilder {
    type Output = Result<Schema>;
    type IntoFuture = BoxFut<'static, Self::Output>;
    fn into_future(self) -> Self::IntoFuture {
        let client = self.client;
        let request = self.request;
        Box::pin(async move { client.get_schema(&request).await })
    }
}
/// Builder for listing schemas
pub struct ListSchemasBuilder {
    client: SchemaServiceClient,
    request: ListSchemasRequest,
}
impl ListSchemasBuilder {
    /// Create a new builder instance.
    /// Obtain via the corresponding method on `SchemaServiceClient`.
    pub(crate) fn new(
        client: SchemaServiceClient,
        catalog_name: impl Into<String>,
        max_results: i32,
        page_token: impl Into<String>,
    ) -> Self {
        let request = ListSchemasRequest {
            catalog_name: catalog_name.into(),
            max_results,
            page_token: page_token.into(),
            ..Default::default()
        };
        Self { client, request }
    }
    /// Convert paginated request into stream of results
    pub fn into_stream(self) -> BoxStr<'static, Result<Schema>> {
        let remaining = self.request.max_results;
        let stream = stream_paginated(
                (self, remaining),
                move |(mut builder, mut remaining), page_token| async move {
                    builder.request.page_token = page_token;
                    let res = builder.client.list_schemas(&builder.request).await?;
                    if let Some(ref mut rem) = remaining {
                        *rem -= res.schemas.len() as i32;
                    }
                    let next_page_token = if remaining.is_some_and(|r| r <= 0) {
                        None
                    } else {
                        res.next_page_token.clone()
                    };
                    Ok((res, (builder, remaining), next_page_token))
                },
            )
            .map_ok(|resp| futures::stream::iter(resp.schemas.into_iter().map(Ok)))
            .try_flatten();
        stream.boxed()
    }
}
impl IntoFuture for ListSchemasBuilder {
    type Output = Result<ListSchemasResponse>;
    type IntoFuture = BoxFut<'static, Self::Output>;
    fn into_future(self) -> Self::IntoFuture {
        let client = self.client;
        let request = self.request;
        Box::pin(async move { client.list_schemas(&request).await })
    }
}
/// Builder for updating a schema
pub struct UpdateSchemaBuilder {
    client: SchemaServiceClient,
    request: UpdateSchemaRequest,
}
impl UpdateSchemaBuilder {
    /// Create a new builder instance.
    /// Obtain via the corresponding method on `SchemaServiceClient`.
    pub(crate) fn new(
        client: SchemaServiceClient,
        full_name: impl Into<String>,
    ) -> Self {
        let request = UpdateSchemaRequest {
            full_name: full_name.into(),
            ..Default::default()
        };
        Self { client, request }
    }
    /// Set schema
    pub fn with_schema(mut self, schema: impl Into<Option<Schema>>) -> Self {
        self.request.schema = {
            let schema: ::core::option::Option<_> = schema.into();
            buffa::MessageField::from(schema)
        };
        self
    }
}
impl IntoFuture for UpdateSchemaBuilder {
    type Output = Result<Schema>;
    type IntoFuture = BoxFut<'static, Self::Output>;
    fn into_future(self) -> Self::IntoFuture {
        let client = self.client;
        let request = self.request;
        Box::pin(async move { client.update_schema(&request).await })
    }
}
/// Builder for deleting a schema
pub struct DeleteSchemaBuilder {
    client: SchemaServiceClient,
    request: DeleteSchemaRequest,
}
impl DeleteSchemaBuilder {
    /// Create a new builder instance.
    /// Obtain via the corresponding method on `SchemaServiceClient`.
    pub(crate) fn new(
        client: SchemaServiceClient,
        full_name: impl Into<String>,
    ) -> Self {
        let request = DeleteSchemaRequest {
            full_name: full_name.into(),
            ..Default::default()
        };
        Self { client, request }
    }
}
impl IntoFuture for DeleteSchemaBuilder {
    type Output = Result<DeleteSchemaResponse>;
    type IntoFuture = BoxFut<'static, Self::Output>;
    fn into_future(self) -> Self::IntoFuture {
        let client = self.client;
        let request = self.request;
        Box::pin(async move { client.delete_schema(&request).await })
    }
}
