// @generated — do not edit by hand.
#![allow(unexpected_cfgs)]
#![allow(clippy::empty_docs)]
#![allow(clippy::useless_borrows_in_formatting)]
#![allow(non_camel_case_types)]
#![allow(clippy::derivable_impls)]
#![allow(clippy::match_single_binding)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::doc_lazy_continuation)]

use std::collections::HashMap;

pub mod labels;
pub use labels::{ObjectLabel, Resource};

#[cfg(feature = "python")]
#[allow(clippy::clone_on_copy)]
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
pub mod extractors;
