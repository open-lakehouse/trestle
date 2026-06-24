// @generated — do not edit by hand.
#![allow(unexpected_cfgs)]
use std::collections::HashMap;
pub use golden_path_app::v1::Greeting;
pub type PropertyMap = HashMap<String, serde_json::Value>;
pub mod golden_path_app {
    pub mod v1 {
        include!("././golden_path_app.v1.rs");
        #[cfg(feature = "grpc")]
        include!("././golden_path_app.v1.tonic.rs");
    }
}
#[cfg(feature = "axum")]
pub mod greeting;
