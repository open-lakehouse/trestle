// @generated — do not edit by hand.
use std::collections::HashMap;
pub mod labels;
pub use labels::{ObjectLabel, Resource};
pub use catalog::v1::Catalog;
pub type PropertyMap = HashMap<String, serde_json::Value>;
pub mod catalog {
    pub mod v1 {
        include!("./../gen/example.catalog.v1.rs");
        #[cfg(feature = "grpc")]
        include!("./../gen/example.catalog.v1.tonic.rs");
    }
}
pub mod tags {
    pub mod v1 {
        include!("./../gen/example.tags.v1.rs");
        #[cfg(feature = "grpc")]
        include!("./../gen/example.tags.v1.tonic.rs");
    }
}
