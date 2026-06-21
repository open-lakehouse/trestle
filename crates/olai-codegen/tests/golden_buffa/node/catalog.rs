// @generated — do not edit by hand.
#![allow(unused_mut, unused_imports, dead_code, clippy::all)]
use std::collections::HashMap;
use napi::bindgen_prelude::Buffer;
use napi_derive::napi;
use buffa::Message;
use example_client::CatalogClient;
use example_common::models::catalog::v1::*;
use crate::error::NapiErrorExt;
#[napi]
pub struct NapiCatalogClient {
    pub(crate) client: CatalogClient,
}
#[napi]
impl NapiCatalogClient {
    #[napi(catch_unwind)]
    pub async fn get(&self) -> napi::Result<Buffer> {
        let mut request = self.client.get();
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
    #[napi(catch_unwind)]
    pub async fn get_catalog_status(&self) -> napi::Result<Buffer> {
        let mut request = self.client.get_catalog_status();
        request.await.map(|item| Buffer::from(item.encode_to_vec())).default_error()
    }
}
impl NapiCatalogClient {
    pub fn new(client: CatalogClient) -> Self {
        Self { client }
    }
}
