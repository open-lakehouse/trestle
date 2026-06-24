//! Parsing stage: compiled descriptor → [`CodeGenMetadata`].
//!
//! This is the first stage of the code-generation pipeline. It walks every
//! message, enum, and service in a [`FileDescriptorSet`] and extracts the
//! annotations that drive generation into a structured, runtime-friendly form.
//!
//! The generator reads **standard Google API extensions only** — there are no
//! custom Trestle proto extensions. The recognized extensions and their field
//! numbers are the `*_EXTENSION` module constants below (`google.api.http`,
//! `google.api.resource`, `google.api.field_behavior`,
//! `google.api.resource_reference`, and `gnostic.openapi.v3.operation`), plus the
//! core `debug_redact` option (`google.protobuf.FieldOptions` field 16, handled
//! in the `message` submodule). [`parse_file_descriptor_set`] is the entry point.

use std::collections::HashMap;

pub use self::http::{HttpPattern, UrlSegment, extract_http_rule_pattern, extract_path_parameters};
pub use self::models::*;
use protobuf::descriptor::{FileDescriptorProto, FileDescriptorSet, SourceCodeInfo};
pub mod types;
use crate::Result;

mod enum_parser;
pub mod http;
mod message;
mod models;
mod service;

/// Extract documentation text for a protobuf element at the given source path.
///
/// Scans `SourceCodeInfo.location` for an entry whose path matches `path`,
/// prefers leading comments over trailing comments, and returns the trimmed text.
pub(super) fn extract_documentation(sci: Option<&SourceCodeInfo>, path: &[i32]) -> Option<String> {
    let sci = sci?;
    for location in &sci.location {
        if location.path.as_slice() == path {
            let text = if location.has_leading_comments() {
                location.leading_comments().trim().to_string()
            } else if location.has_trailing_comments() {
                location.trailing_comments().trim().to_string()
            } else {
                String::new()
            };
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

// Known extension field numbers
const GOOGLE_API_HTTP_EXTENSION: u32 = 72295728; // google.api.http
const GNOSTIC_OPERATION_EXTENSION: u32 = 1143; // gnostic.openapi.v3.operation
const GOOGLE_API_RESOURCE_EXTENSION: u32 = 1053; // google.api.resource
const GOOGLE_API_FIELD_BEHAVIOR_EXTENSION: u32 = 1052; // google.api.field_behavior
const GOOGLE_API_RESOURCE_REFERENCE_EXTENSION: u32 = 1055; // google.api.resource_reference

/// Parse a compiled protobuf [`FileDescriptorSet`] into [`CodeGenMetadata`].
///
/// This is the first stage of the code-generation pipeline. It walks every
/// message, enum, and service in the descriptor set and extracts the annotations
/// that drive generation (`google.api.http`, `google.api.resource`,
/// `field_behavior`, `debug_redact`, …) into a structured form.
///
/// The full pipeline is three stages:
///
/// ```text
/// FileDescriptorSet
///   │  parse_file_descriptor_set
///   ▼
/// CodeGenMetadata
///   │  analyze_metadata          (analysis/mod.rs)
///   ▼
/// GenerationPlan
///   │  generate_code             (codegen/mod.rs)
///   ▼
/// Rust / Python / TypeScript source written to disk
/// ```
///
/// Typical usage from a build script:
///
/// ```ignore
/// let fds = prost_types::FileDescriptorSet::decode(&bytes[..])?;
/// let metadata = parse_file_descriptor_set(&fds)?;
/// let config = CodeGenConfig::new(/* … */);
/// generate_code(&metadata, &config)?;
/// ```
///
/// Methods without HTTP annotations are not an error here; they are recorded later,
/// during [`analyze_metadata`](crate::analyze_metadata).
///
/// # Errors
///
/// Returns:
///
/// - [`Error::InvalidAnnotation`](crate::Error::InvalidAnnotation) if a `google.api.http`,
///   `google.api.resource`, or `field_behavior` annotation is present but malformed (e.g. a
///   resource pattern or HTTP rule that cannot be interpreted).
/// - [`Error::Build`](crate::Error::Build) if an annotation extension cannot be decoded from its
///   raw descriptor bytes.
pub fn parse_file_descriptor_set(
    file_descriptor_set: &FileDescriptorSet,
) -> Result<CodeGenMetadata> {
    let mut codegen_metadata = CodeGenMetadata {
        messages: HashMap::new(),
        enums: HashMap::new(),
        services: HashMap::new(),
    };

    // Process each file descriptor
    for file_descriptor in &file_descriptor_set.file {
        process_file_descriptor(file_descriptor, &mut codegen_metadata)?;
    }

    Ok(codegen_metadata)
}

/// Process a single protobuf file descriptor
///
/// Extracts all messages, services, and annotations from the file.
/// Collects metadata for code generation.
pub fn process_file_descriptor(
    file_desc: &FileDescriptorProto,
    codegen_metadata: &mut CodeGenMetadata,
) -> Result<()> {
    let file_name = file_desc.name();

    // Extract source code info for documentation
    let source_code_info = file_desc.source_code_info.as_ref();

    // Process enums in the file
    for (enum_index, enum_desc) in file_desc.enum_type.iter().enumerate() {
        let package_name = file_desc.package();
        let type_prefix = if package_name.is_empty() {
            String::new()
        } else {
            format!(".{}", package_name)
        };
        enum_parser::process_enum(
            enum_desc,
            codegen_metadata,
            &type_prefix,
            source_code_info,
            &[5, enum_index as i32], // enum_type is field 5 in FileDescriptorProto
        )?;
    }

    // Process messages in the file
    for (message_index, message) in file_desc.message_type.iter().enumerate() {
        let package_name = file_desc.package();
        let type_prefix = if package_name.is_empty() {
            String::new()
        } else {
            format!(".{}", package_name)
        };

        message::process_message(
            message,
            file_name,
            codegen_metadata,
            &type_prefix,
            source_code_info,
            &[4, message_index as i32], // message_type is field 4 in FileDescriptorProto
        )?;
    }

    // Process services in the file
    let package_name = file_desc.package().to_string();
    for (service_index, service) in file_desc.service.iter().enumerate() {
        service::process_service(
            service,
            &package_name,
            codegen_metadata,
            source_code_info,
            service_index,
        )?;
    }

    Ok(())
}
