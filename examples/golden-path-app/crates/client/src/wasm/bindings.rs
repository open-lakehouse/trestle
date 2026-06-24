// @generated — do not edit by hand.
#![cfg(target_arch = "wasm32")]
#![allow(unused_mut, unused_imports, dead_code, clippy::all)]
use crate::codegen::greeting::*;
use crate::models::golden_path_app::v1::*;
use olai_http_wasm::reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use olai_http_wasm::{CredentialsMode, WasmClient};
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
/// Optional client options, passed as the second constructor argument from JS.
///
/// All fields are optional; the no-options / base-URL-only call keeps the default
/// browser-session (cookie) behavior.
#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClientOptions {
    /// Bearer token. When set, every request carries `Authorization: Bearer <token>`.
    #[serde(default)]
    auth_token: Option<String>,
    /// `fetch` credentials mode: `"include"` (default), `"same-origin"`, or `"omit"`.
    #[serde(default)]
    credentials: Option<String>,
}
/// Browser entry point: construct from a base URL, then access per-service clients.
///
/// By default the browser manages the session (cookies / auth headers ride along with
/// `fetch`). Pass an optional options object to use bearer-token auth instead and/or to
/// choose the `fetch` credentials mode.
#[wasm_bindgen(js_name = "GoldenPathAppClient")]
pub struct WasmGoldenPathAppClient {
    client: WasmClient,
    base_url: Url,
}
#[wasm_bindgen(js_class = "GoldenPathAppClient")]
impl WasmGoldenPathAppClient {
    /// Construct the client.
    ///
    /// `options` is optional: omit it (or pass `undefined`/`null`) to keep the
    /// browser-session cookie behavior. Supported fields:
    /// `{ authToken?: string, credentials?: "include" | "same-origin" | "omit" }`.
    #[wasm_bindgen(constructor)]
    pub fn new(
        base_url: String,
        options: Option<JsValue>,
    ) -> Result<WasmGoldenPathAppClient, JsValue> {
        let mut base_url = Url::parse(&base_url)
            .map_err(|e| JsValue::from_str(&format!("invalid base_url: {e}")))?;
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        let options: ClientOptions = match options {
            Some(v) if !v.is_undefined() && !v.is_null() => serde_wasm_bindgen::from_value(v)
                .map_err(|e| JsValue::from_str(&format!("invalid options: {e}")))?,
            _ => ClientOptions::default(),
        };
        let mut client = WasmClient::new();
        if let Some(credentials) = options.credentials.as_deref() {
            let mode = match credentials {
                "include" => CredentialsMode::Include,
                "same-origin" => CredentialsMode::SameOrigin,
                "omit" => CredentialsMode::Omit,
                other => {
                    return Err(JsValue::from_str(&format!(
                        "invalid credentials mode: {other:?} (expected \"include\", \"same-origin\", or \"omit\")"
                    )));
                }
            };
            client = client.with_credentials(mode);
        } else if options.auth_token.is_some() {
            client = client.with_credentials(CredentialsMode::Omit);
        }
        if let Some(token) = options.auth_token {
            let value = HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|e| JsValue::from_str(&format!("invalid authToken: {e}")))?;
            client = client.with_auth(move || {
                let mut headers = HeaderMap::new();
                headers.insert(AUTHORIZATION, value.clone());
                headers
            });
        }
        Ok(Self { client, base_url })
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
