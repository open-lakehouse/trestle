//! Proto-driven code generation from compiled protobuf descriptors.
//!
//! Turns descriptor bytes (from `buf build`) into Rust REST-API glue code (Axum
//! handlers, HTTP clients), resource registries, and optional PyO3 / NAPI /
//! TypeScript bindings. Code generation is normally driven by the `trestle` CLI
//! ([`olai-trestle`]); this is the library you embed for custom build tooling.
//!
//! ## Pipeline
//!
//! ```text
//! descriptor bytes
//!   └─ parse_file_descriptor_set ─▶ CodeGenMetadata
//!        └─ generate_code(&metadata, &config) ─▶ files written to CodeGenOutput dirs
//! ```
//!
//! ## Example
//!
//! ```rust,ignore
//! use std::fs;
//! use olai_codegen::{CodeGenConfig, CodeGenOutput, generate_code, parse_file_descriptor_set};
//! use protobuf::{Message, descriptor::FileDescriptorSet};
//!
//! let bytes = fs::read("descriptors.bin")?;           // produced by `buf build`
//! let fds = FileDescriptorSet::parse_from_bytes(&bytes)?;
//! let metadata = parse_file_descriptor_set(&fds)?;
//!
//! let config = CodeGenConfig {
//!     context_type_path: "crate::api::RequestContext".into(),
//!     result_type_path: "crate::Result".into(),
//!     models_path_template: "my_crate::models::{service}::v1".into(),
//!     models_path_crate_template: "crate::models::{service}::v1".into(),
//!     generate_resource_enum: true,
//!     output: CodeGenOutput {
//!         common: "out/common".into(),
//!         server: Some("out/server".into()),
//!         client: Some("out/client".into()),
//!         ..Default::default()
//!     },
//!     ..Default::default()
//! };
//! config.validate()?;
//! generate_code(&metadata, &config)?;
//! ```
//!
//! [`olai-trestle`]: https://crates.io/crates/olai-trestle

pub use error::*;

pub mod analysis;
pub mod codegen;
pub mod error;
pub mod openapi_enrich;
pub mod output;
pub mod parsing;
pub mod utils;

pub use codegen::{
    BindingsConfig, CodeGenConfig, CodeGenOutput, DEFAULT_TRANSPORT_TYPE_PATH, GeneratedCode,
    Runtime, generate_code, generate_models_mod,
};

pub use analysis::{
    BodyField, GenerationPlan, ManagedResource, MethodPlan, PathParam, QueryParam, RequestParam,
    RequestType, ResourceHierarchy, ServicePlan, SkippedMethod, analyze_metadata,
    extract_managed_resources, split_body_fields,
};
// Note: MethodPlanner is pub(crate) — it is an internal helper, not part of the public API.
pub use openapi_enrich::run as enrich_openapi;
pub use parsing::http::HttpPattern;
pub use parsing::types::{BaseType, RenderContext, UnifiedType};
pub use parsing::{CodeGenMetadata, parse_file_descriptor_set, process_file_descriptor};

/// The `FieldBehavior` enum from `google.api.field_behavior`, re-exported for
/// consumers that need to inspect field behavior annotations (e.g. in tests).
pub use google::api::FieldBehavior;

// Prost-generated Google API proto types — internal only.
pub(crate) mod google {
    pub mod api {
        #![allow(unused)]
        #![allow(clippy::doc_overindented_list_items)]
        #![allow(clippy::doc_lazy_continuation)]
        include!("./gen/google.api.rs");
    }
}

pub(crate) mod gnostic {
    pub mod openapi {
        pub mod v3 {
            #![allow(unused)]
            #![allow(clippy::large_enum_variant)]
            include!("./gen/gnostic.openapi.v3.rs");
        }
    }
}
