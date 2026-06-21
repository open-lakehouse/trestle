// @generated — do not edit by hand.
#![allow(unused_mut, unused_imports, dead_code, clippy::all)]
pub mod catalog;
pub mod schema;
use std::collections::HashMap;
use futures::stream::TryStreamExt;
use futures::StreamExt;
use napi::bindgen_prelude::{Buffer, ReadableStream};
use napi::Env;
use napi_derive::napi;
use buffa::Message;
use example_client::ExampleClient;
use crate::error::NapiErrorExt;
use example_common::models::catalog::v1::*;
use example_common::models::catalog::v1::*;
use example_common::models::schemas::v1::*;
use example_common::models::tags::v1::*;
use crate::codegen::catalog::NapiCatalogClient;
use crate::codegen::schema::NapiSchemaClient;
#[napi]
pub struct NapiExampleClient {
    client: ExampleClient,
}
#[napi]
impl NapiExampleClient {
    #[napi(factory)]
    pub fn from_url(base_url: String, token: Option<String>) -> napi::Result<Self> {
        let client = if let Some(token) = token {
            olai_http::CloudClient::new_with_token(token)
        } else {
            olai_http::CloudClient::new_unauthenticated()
        };
        let base_url = base_url
            .parse()
            .map_err(|e: url::ParseError| {
                napi::Error::new(napi::Status::GenericFailure, e.to_string())
            })?;
        Ok(Self {
            client: ExampleClient::new(client, base_url),
        })
    }
    #[napi(catch_unwind)]
    pub async fn create_catalog(&self) -> napi::Result<Buffer> {
        let mut request = self.client.create_catalog();
        request.await.map(|item| Buffer::from(item.encode_to_vec())).default_error()
    }
    #[napi(catch_unwind)]
    pub async fn list_catalogs(&self, max_results: i32) -> napi::Result<Vec<Buffer>> {
        let mut request = self.client.list_catalogs(max_results, page_token);
        request
            .into_stream()
            .map_ok(|item| Buffer::from(item.encode_to_vec()))
            .try_collect::<Vec<_>>()
            .await
            .default_error()
    }
    #[napi(catch_unwind)]
    pub fn list_catalogs_stream(
        &self,
        env: Env,
        max_results: i32,
    ) -> napi::Result<ReadableStream<'_, Buffer>> {
        let mut request = self.client.list_catalogs(max_results, page_token);
        ReadableStream::new(
            &env,
            request
                .into_stream()
                .map(|item| {
                    item.map(|v| Buffer::from(v.encode_to_vec()))
                        .map_err(|e| crate::error::convert_error(&e))
                }),
        )
    }
    #[napi(catch_unwind)]
    pub async fn generate_catalog_token(
        &self,
        catalog_id: String,
    ) -> napi::Result<Buffer> {
        let mut request = self.client.generate_catalog_token(catalog_id);
        request.await.map(|item| Buffer::from(item.encode_to_vec())).default_error()
    }
    #[napi(catch_unwind)]
    pub async fn list_by_tags(
        &self,
        tags: Option<Vec<String>>,
        max_results: i32,
    ) -> napi::Result<Buffer> {
        let mut request = self.client.list_by_tags(tags, max_results);
        request.await.map(|item| Buffer::from(item.encode_to_vec())).default_error()
    }
    #[napi(catch_unwind)]
    pub async fn list_by_catalog_type(&self, catalog_type: i32) -> napi::Result<Buffer> {
        let mut request = self
            .client
            .list_by_catalog_type(
                <CatalogType as buffa::Enumeration>::from_i32(catalog_type)
                    .ok_or_else(|| napi::Error::new(
                        napi::Status::GenericFailure,
                        "invalid enum value",
                    ))?,
            );
        request.await.map(|item| Buffer::from(item.encode_to_vec())).default_error()
    }
    #[napi(catch_unwind)]
    pub async fn create_schema(
        &self,
        name: String,
        catalog_name: String,
        schema_type: i32,
    ) -> napi::Result<Buffer> {
        let mut request = self
            .client
            .create_schema(
                name,
                catalog_name,
                <SchemaType as buffa::Enumeration>::from_i32(schema_type)
                    .ok_or_else(|| napi::Error::new(
                        napi::Status::GenericFailure,
                        "invalid enum value",
                    ))?,
            );
        request.await.map(|item| Buffer::from(item.encode_to_vec())).default_error()
    }
    #[napi(catch_unwind)]
    pub async fn list_schemas(
        &self,
        catalog_name: String,
        max_results: i32,
    ) -> napi::Result<Vec<Buffer>> {
        let mut request = self
            .client
            .list_schemas(catalog_name, max_results, page_token);
        request
            .into_stream()
            .map_ok(|item| Buffer::from(item.encode_to_vec()))
            .try_collect::<Vec<_>>()
            .await
            .default_error()
    }
    #[napi(catch_unwind)]
    pub fn list_schemas_stream(
        &self,
        env: Env,
        catalog_name: String,
        max_results: i32,
    ) -> napi::Result<ReadableStream<'_, Buffer>> {
        let mut request = self
            .client
            .list_schemas(catalog_name, max_results, page_token);
        ReadableStream::new(
            &env,
            request
                .into_stream()
                .map(|item| {
                    item.map(|v| Buffer::from(v.encode_to_vec()))
                        .map_err(|e| crate::error::convert_error(&e))
                }),
        )
    }
    #[napi(catch_unwind)]
    pub async fn list_tag_assignments(
        &self,
        entity_type: String,
        entity_name: String,
        max_results: i32,
        page_token: String,
    ) -> napi::Result<Buffer> {
        let mut request = self
            .client
            .list_tag_assignments(entity_type, entity_name, max_results, page_token);
        request.await.map(|item| Buffer::from(item.encode_to_vec())).default_error()
    }
    #[napi(catch_unwind)]
    pub async fn create_tag_assignment(
        &self,
        entity_type: String,
        entity_name: String,
    ) -> napi::Result<Buffer> {
        let mut request = self.client.create_tag_assignment(entity_type, entity_name);
        request.await.map(|item| Buffer::from(item.encode_to_vec())).default_error()
    }
    #[napi(catch_unwind)]
    pub async fn fetch_tag_assignment(
        &self,
        entity_type: String,
        entity_name: String,
        tag_key: String,
    ) -> napi::Result<Buffer> {
        let mut request = self
            .client
            .get_tag_assignment(entity_type, entity_name, tag_key);
        request.await.map(|item| Buffer::from(item.encode_to_vec())).default_error()
    }
    #[napi(catch_unwind)]
    pub async fn delete_tag_assignment(
        &self,
        entity_type: String,
        entity_name: String,
        tag_key: String,
    ) -> napi::Result<Buffer> {
        let mut request = self
            .client
            .delete_tag_assignment(entity_type, entity_name, tag_key);
        request.await.map(|item| Buffer::from(item.encode_to_vec())).default_error()
    }
    #[napi(catch_unwind)]
    pub async fn touch_tag_assignment(
        &self,
        entity_type: String,
        entity_name: String,
        tag_key: String,
    ) -> napi::Result<()> {
        let mut request = self
            .client
            .touch_tag_assignment(entity_type, entity_name, tag_key);
        request.await.default_error()
    }
    #[napi]
    pub fn catalog(&self, catalog_name: String) -> NapiCatalogClient {
        NapiCatalogClient {
            client: self.client.catalog(catalog_name),
        }
    }
    #[napi]
    pub fn schema(&self, catalog_name: String, schema_name: String) -> NapiSchemaClient {
        let full_name = format!("{}.{}", catalog_name, schema_name);
        NapiSchemaClient {
            client: self.client.schema_from_full_name(full_name),
        }
    }
}
