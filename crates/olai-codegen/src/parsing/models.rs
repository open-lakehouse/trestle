use std::collections::HashMap;

use crate::gnostic::openapi::v3::Operation;
use crate::google::api::{FieldBehavior, HttpRule, ResourceDescriptor, ResourceReference};
use crate::parsing::http::HttpPattern;
use crate::parsing::types::UnifiedType;
use crate::utils::last_segment;

/// Collected metadata for code generation
#[derive(Debug, Default)]
pub struct CodeGenMetadata {
    pub messages: HashMap<String, MessageInfo>,
    pub enums: HashMap<String, EnumInfo>,
    pub services: HashMap<String, ServiceInfo>,
}

impl CodeGenMetadata {
    /// Get message fields for a given type name
    pub fn get_message_fields(&self, type_name: &str) -> Vec<MessageField> {
        self.messages
            .get(type_name)
            .map(|msg| msg.fields.clone())
            .unwrap_or_default()
    }

    /// Find a resource descriptor whose `singular` field matches `name`.
    pub fn resource_from_singular(&self, name: &str) -> Option<&ResourceDescriptor> {
        self.messages.values().find_map(|info| {
            info.resource_descriptor
                .as_ref()
                .filter(|r| r.singular == name)
        })
    }

    /// Find a resource descriptor whose `plural` field matches `name`.
    pub fn resource_from_plural(&self, name: &str) -> Option<&ResourceDescriptor> {
        self.messages.values().find_map(|info| {
            info.resource_descriptor
                .as_ref()
                .filter(|r| r.plural == name)
        })
    }

    /// Get resource descriptor by message type name (simple or fully-qualified).
    pub fn get_resource_descriptor(&self, type_name: &str) -> Option<&ResourceDescriptor> {
        // Try direct lookup first (fully-qualified name)
        if let Some(descriptor) = self
            .messages
            .get(type_name)
            .and_then(|info| info.resource_descriptor.as_ref())
        {
            return Some(descriptor);
        }

        // Fall back to simple-name suffix match
        self.messages.iter().find_map(|(key, info)| {
            if last_segment(key) == type_name {
                info.resource_descriptor.as_ref()
            } else {
                None
            }
        })
    }
}

/// Information about a protobuf message
#[derive(Debug, Clone)]
pub struct MessageInfo {
    pub name: String,
    pub fields: Vec<MessageField>,
    pub resource_descriptor: Option<ResourceDescriptor>,
    pub documentation: Option<String>,
}

/// Information about a field in a protobuf message
#[derive(Debug, Clone)]
pub struct MessageField {
    pub name: String,
    /// The explicit `json_name` of the field; `None` for protoc's default.
    pub json_name: Option<String>,
    /// The HTTP response header this field is read from (`(olai.trestle.v1.response_header)`).
    pub response_header: Option<String>,
    /// The HTTP request header this field is sent in (`(olai.trestle.v1.request_header)`).
    pub request_header: Option<String>,
    /// Whether the field is sent even when unpopulated (`(olai.trestle.v1.emit_unpopulated)`).
    pub emit_unpopulated: bool,
    /// Whether the field is the raw HTTP body of its message (`(olai.trestle.v1.raw_body)`).
    pub raw_body: bool,
    /// Language-agnostic type; carries `is_optional` and `is_repeated` flags.
    pub unified_type: UnifiedType,
    pub documentation: Option<String>,
    /// For oneof fields, contains the variants with their field names and types.
    pub oneof_variants: Option<Vec<OneofVariant>>,
    /// Field behavior annotations from google.api.field_behavior
    pub field_behavior: Vec<FieldBehavior>,
    /// Whether the field is marked with `debug_redact = true` in the proto definition.
    /// Indicates the field contains sensitive data that should be routed to secret storage.
    pub is_sensitive: bool,
    /// Resource reference annotation from `google.api.resource_reference` (ext 1055).
    ///
    /// - `child_type` non-empty: this field identifies a parent container for the named resource
    ///   (e.g. `catalog_name` with `child_type = "example.io/Schema"`).
    /// - `r#type` non-empty: this field directly identifies a resource of that type
    ///   (e.g. `full_name` with `type = "example.io/Table"`).
    pub resource_reference: Option<ResourceReference>,
}

/// Information about a variant in a oneof field
#[derive(Debug, Clone)]
pub struct OneofVariant {
    pub field_name: String,      // e.g., "azure_service_principal"
    pub variant_name: String,    // e.g., "AzureServicePrincipal"
    pub field_type: UnifiedType, // the unified type (language-agnostic)
    pub documentation: Option<String>,
}

/// Information about a protobuf enum
#[derive(Debug, Clone)]
pub struct EnumInfo {
    pub name: String,
    pub values: Vec<EnumValue>,
    pub documentation: Option<String>,
}

/// Information about an enum value
#[derive(Debug, Clone)]
pub struct EnumValue {
    pub name: String,
    pub number: i32,
    pub documentation: Option<String>,
}

/// Information about a protobuf service
#[derive(Debug, Clone)]
pub struct ServiceInfo {
    pub name: String,
    /// Proto package name (e.g. `"example.catalogs.v1"`).
    pub package: String,
    pub documentation: Option<String>,
    pub methods: Vec<MethodMetadata>,
}

/// Metadata extracted from a service method
#[derive(Debug, Clone)]
pub struct MethodMetadata {
    pub service_name: String,
    pub method_name: String,
    pub input_type: String,
    pub output_type: String,
    pub operation: Option<Operation>,
    pub http_rule: HttpRule,
    /// Pre-parsed HTTP URL pattern. Analysis should use this directly instead of re-parsing.
    pub http_pattern: HttpPattern,
    pub documentation: Option<String>,
    /// A server-streaming RPC (`returns (stream T)`): NDJSON over REST.
    pub server_streaming: bool,
    /// The `(olai.trestle.v1.authz)` override of the method's securable, if any.
    pub authz: Option<AuthzRule>,
}

/// The `(olai.trestle.v1.authz)` method option: how to find the securable a request acts on
/// when codegen can't infer it from the request's resource fields.
#[derive(Clone, PartialEq, Eq, ::prost::Message)]
pub struct AuthzRule {
    /// The `google.api.resource` type of the securable (e.g. `example.io/Table`); empty for the
    /// service root.
    #[prost(string, tag = "1")]
    pub securable_type: String,
    /// The request fields whose values, in order, make up the securable's name. Each is split
    /// on `.`, so a full-name field gives every part.
    #[prost(string, repeated, tag = "2")]
    pub securable_fields: Vec<String>,
    /// Emit no `Authorizable` impl: the server authorizes this operation by hand. The operation
    /// and its Cedar action (on `securable_type`) are still emitted.
    #[prost(bool, tag = "3")]
    pub skip: bool,
}

impl MethodMetadata {
    /// Return the HTTP method string (e.g. "GET", "POST").
    pub fn http_method(&self) -> Option<&str> {
        use crate::google::api::http_rule::Pattern;
        self.http_rule.pattern.as_ref().map(|p| match p {
            Pattern::Get(_) => "GET",
            Pattern::Post(_) => "POST",
            Pattern::Put(_) => "PUT",
            Pattern::Delete(_) => "DELETE",
            Pattern::Patch(_) => "PATCH",
            Pattern::Custom(c) => c.kind.as_str(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_based_method_structure() {
        let mut codegen_metadata = CodeGenMetadata {
            messages: HashMap::new(),
            services: HashMap::new(),
            enums: HashMap::new(),
        };

        // Create a test service with methods
        let mut service_info = ServiceInfo {
            name: "TestService".to_string(),
            package: "test.v1".to_string(),
            documentation: Some("Test service documentation".to_string()),
            methods: Vec::new(),
        };

        // Add a method to the service
        let http_rule = crate::google::api::HttpRule {
            pattern: Some(crate::google::api::http_rule::Pattern::Get(
                "/test".to_string(),
            )),
            ..Default::default()
        };

        let method = MethodMetadata {
            service_name: "TestService".to_string(),
            method_name: "TestMethod".to_string(),
            input_type: ".test.TestRequest".to_string(),
            output_type: ".test.TestResponse".to_string(),
            server_streaming: false,
            authz: None,
            operation: None,
            http_pattern: crate::parsing::http::HttpPattern::parse("/test"),
            http_rule,
            documentation: Some("Test method documentation".to_string()),
        };
        service_info.methods.push(method);

        codegen_metadata
            .services
            .insert("TestService".to_string(), service_info);

        // Test that we can access the service and its methods directly
        let service = codegen_metadata.services.get("TestService").unwrap();
        assert_eq!(service.name, "TestService");
        assert_eq!(service.methods.len(), 1);
        assert_eq!(service.methods[0].method_name, "TestMethod");
    }
}
