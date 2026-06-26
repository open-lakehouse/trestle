// @generated — do not edit by hand.
#![allow(unexpected_cfgs)]
#![allow(clippy::empty_docs)]
#![allow(non_camel_case_types)]
#![allow(clippy::derivable_impls)]
use std::collections::HashMap;
pub mod labels;
pub use labels::{ObjectLabel, Resource};
#[cfg(feature = "python")]
pub mod pyo3_impls;
#[cfg(feature = "python")]
pub use pyo3_impls::*;
pub use catalog::v1::Catalog;
pub use schemas::v1::Schema;
pub type PropertyMap = HashMap<String, serde_json::Value>;
pub mod catalog {
    pub mod v1 {
        include!("./example.catalog.v1.rs");
    }
}
pub mod schemas {
    pub mod v1 {
        include!("./example.schemas.v1.rs");
    }
}
pub mod tags {
    pub mod v1 {
        include!("./example.tags.v1.rs");
    }
}
#[cfg(feature = "axum")]
pub mod catalog;
#[cfg(feature = "axum")]
pub mod query;
#[cfg(feature = "axum")]
pub mod schema;
#[cfg(feature = "axum")]
pub mod tag_assignments;
