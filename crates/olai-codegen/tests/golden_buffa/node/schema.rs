// @generated — do not edit by hand.
#![allow(unused_mut, unused_imports, dead_code, clippy::all)]
use std::collections::HashMap;
use napi::bindgen_prelude::Buffer;
use napi_derive::napi;
use buffa::Message;
use example_client::SchemaClient;
use example_common::models::schemas::v1::*;
use crate::error::NapiErrorExt;
#[napi]
pub struct NapiSchemaClient {
    pub(crate) client: SchemaClient,
}
#[napi]
impl NapiSchemaClient {
    #[napi(catch_unwind)]
    pub async fn get(&self, view: i32) -> napi::Result<Buffer> {
        let mut request = self
            .client
            .get(
                <get_schema_request::View as buffa::Enumeration>::from_i32(view)
                    .ok_or_else(|| napi::Error::new(
                        napi::Status::GenericFailure,
                        "invalid enum value",
                    ))?,
            );
        request.await.map(|item| Buffer::from(item.encode_to_vec())).default_error()
    }
    #[napi(catch_unwind)]
    pub async fn update(&self) -> napi::Result<Buffer> {
        let mut request = self.client.update();
        request.await.map(|item| Buffer::from(item.encode_to_vec())).default_error()
    }
    #[napi(catch_unwind)]
    pub async fn delete(&self) -> napi::Result<()> {
        let mut request = self.client.delete();
        request.await.default_error()
    }
}
impl NapiSchemaClient {
    pub fn new(client: SchemaClient) -> Self {
        Self { client }
    }
}
