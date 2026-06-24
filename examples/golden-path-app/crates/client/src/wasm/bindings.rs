// @generated — do not edit by hand.
#![cfg(target_arch = "wasm32")]
#![allow(unused_mut, unused_imports, dead_code, clippy::all)]
use crate::codegen::greeting::*;
use crate::models::golden_path_app::v1::*;
use olai_http_wasm::WasmClient;
use url::Url;
use wasm_bindgen::prelude::*;
///WASM/browser binding for the `greeting` service.
#[wasm_bindgen]
pub struct WasmGreetingServiceClient {
    inner: GreetingServiceClient,
}
#[wasm_bindgen]
impl WasmGreetingServiceClient {
    #[wasm_bindgen(js_name = "createGreeting")]
    pub async fn create_greeting(&self, request: JsValue) -> Result<JsValue, JsValue> {
        let request: CreateGreetingRequest = serde_wasm_bindgen::from_value(request)
            .map_err(|e| JsValue::from_str(&format!("invalid request: {e}")))?;
        let response = self
            .inner
            .create_greeting(&request)
            .await
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        serde_wasm_bindgen::to_value(&response)
            .map_err(|e| JsValue::from_str(&format!("invalid response: {e}")))
    }
    #[wasm_bindgen(js_name = "getGreeting")]
    pub async fn get_greeting(&self, request: JsValue) -> Result<JsValue, JsValue> {
        let request: GetGreetingRequest = serde_wasm_bindgen::from_value(request)
            .map_err(|e| JsValue::from_str(&format!("invalid request: {e}")))?;
        let response = self
            .inner
            .get_greeting(&request)
            .await
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        serde_wasm_bindgen::to_value(&response)
            .map_err(|e| JsValue::from_str(&format!("invalid response: {e}")))
    }
}
/// Browser entry point: construct from a base URL, then access per-service clients.
/// The browser manages the session (cookies / auth headers ride along with `fetch`).
#[wasm_bindgen(js_name = "GoldenPathAppClient")]
pub struct WasmGoldenPathAppClient {
    client: WasmClient,
    base_url: Url,
}
#[wasm_bindgen(js_class = "GoldenPathAppClient")]
impl WasmGoldenPathAppClient {
    #[wasm_bindgen(constructor)]
    pub fn new(base_url: String) -> Result<WasmGoldenPathAppClient, JsValue> {
        let mut base_url = Url::parse(&base_url)
            .map_err(|e| JsValue::from_str(&format!("invalid base_url: {e}")))?;
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        Ok(Self {
            client: WasmClient::new(),
            base_url,
        })
    }
    ///Access the `greeting` service.
    #[wasm_bindgen(js_name = "greeting")]
    pub fn greeting(&self) -> WasmGreetingServiceClient {
        WasmGreetingServiceClient {
            inner: crate::codegen::greeting::GreetingServiceClient::new(
                self.client.clone(),
                self.base_url.clone(),
            ),
        }
    }
}
