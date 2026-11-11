//! The generated models (`crate::models`) and REST client (`crate::codegen`) of
//! `proto/demo/v1/svc.proto`, mounted as a client crate mounts them.

// The generated code refers to this crate by name.
extern crate self as rest_client_e2e;

pub mod models {
    include!(concat!(env!("OUT_DIR"), "/models_mod.rs"));
    pub use _gen::*;
}

pub mod codegen {
    include!(concat!(env!("OUT_DIR"), "/client/mod.rs"));
}

// The binding crate imports the clients and the error module from the crate root.
pub use codegen::*;

/// The default signer of the Python client (`default_signer` in the generation config), by
/// the host: a bearer token at `localhost`, none at `127.0.0.1`, an error elsewhere.
#[cfg(feature = "cloud")]
pub async fn default_signer(
    base_url: &url::Url,
) -> olai_http::Result<Option<std::sync::Arc<dyn olai_http::RequestSigner>>> {
    use olai_http::bearer::{BearerAuth, BearerProviderSigner};

    match base_url.host_str() {
        Some("localhost") => Ok(Some(std::sync::Arc::new(BearerProviderSigner(
            BearerAuth::static_token("defaulted"),
        )))),
        Some("127.0.0.1") => Ok(None),
        _ => Err(olai_http::Error::Generic {
            source: "no default token here".into(),
        }),
    }
}

/// Context type of the (unused) server config.
pub struct Context;

#[cfg(test)]
mod tests;
