use prost::Message as _;
use protobuf::Message;
use protobuf::descriptor::{MethodDescriptorProto, ServiceDescriptorProto, SourceCodeInfo};
use tracing::{debug, warn};

use super::{CodeGenMetadata, MethodMetadata, ServiceInfo, extract_documentation};
use crate::gnostic::openapi::v3::Operation;
use crate::google::api::HttpRule;
use crate::parsing::http::{HttpPattern, extract_http_rule_pattern};
use crate::{Error, Result};

/// Process a protobuf service definition
pub(super) fn process_service(
    service: &ServiceDescriptorProto,
    package: &str,
    codegen_metadata: &mut CodeGenMetadata,
    source_code_info: Option<&SourceCodeInfo>,
    service_index: usize,
) -> Result<()> {
    let service_name = service.name();

    // Extract service-level documentation
    let service_path = vec![6, service_index as i32]; // Services are at path [6, service_index]
    let service_documentation = extract_documentation(source_code_info, &service_path);

    // Store service information
    let service_info = ServiceInfo {
        name: service_name.to_string(),
        package: package.to_string(),
        documentation: service_documentation,
        methods: Vec::new(),
    };
    codegen_metadata
        .services
        .insert(service_name.to_string(), service_info);

    // Process methods in the service
    for (method_index, method) in service.method.iter().enumerate() {
        process_method(
            method,
            service_name,
            codegen_metadata,
            source_code_info,
            service_index,
            method_index,
        )?;
    }

    Ok(())
}

/// Process a gRPC method definition and extract REST API metadata
fn process_method(
    method: &MethodDescriptorProto,
    service_name: &str,
    codegen_metadata: &mut CodeGenMetadata,
    source_code_info: Option<&SourceCodeInfo>,
    service_index: usize,
    method_index: usize,
) -> Result<()> {
    let method_name = method.name();
    let input_type = method.input_type();
    let output_type = method.output_type();

    // Extract method-level documentation
    // Methods are at path [6, service_index, 2, method_index]
    let method_path = vec![6, service_index as i32, 2, method_index as i32];
    let method_documentation = extract_documentation(source_code_info, &method_path);

    // Extract gnostic method-level annotations first to get required http_rule
    let (operation, http_rule) = extract_method_annotations(method, service_name)?;

    // Pre-parse the HTTP URL pattern so analysis can use it directly.
    let http_pattern =
        extract_http_rule_pattern(&http_rule).unwrap_or_else(|| HttpPattern::parse(""));

    let method_metadata = MethodMetadata {
        service_name: service_name.to_string(),
        method_name: method_name.to_string(),
        input_type: input_type.to_string(),
        output_type: output_type.to_string(),
        operation,
        http_rule,
        http_pattern,
        documentation: method_documentation,
    };

    // Add to the service's methods
    if let Some(service_info) = codegen_metadata.services.get_mut(service_name) {
        service_info.methods.push(method_metadata);
    }

    Ok(())
}

/// Extract gnostic annotations from method-level options
///
/// This is where we extract the key information for REST API generation:
/// - HTTP method and path from google.api.http extension
/// - Operation ID from gnostic.openapi.v3.operation extension
fn extract_method_annotations(
    method: &MethodDescriptorProto,
    service_name: &str,
) -> Result<(Option<Operation>, HttpRule)> {
    let missing_http_rule = || Error::MissingAnnotation {
        object: method.name().to_string(),
        message: "missing required google.api.http annotation".to_string(),
    };
    let Some(options) = method.options.as_ref() else {
        return Err(missing_http_rule());
    };
    let unknown_fields = options.unknown_fields();

    let mut operation = None;
    let mut http_rule = None;

    // Process each extension field
    for (field_number, field_value) in unknown_fields.iter() {
        let data = match field_value {
            protobuf::UnknownValueRef::LengthDelimited(bytes) => bytes,
            _ => {
                debug!("Skipping non-length-delimited field {}", field_number);
                continue;
            }
        };

        match field_number {
            super::GOOGLE_API_HTTP_EXTENSION => {
                let rule = HttpRule::decode(data).map_err(|e| Error::InvalidAnnotation {
                    object: method.name().to_string(),
                    message: format!(
                        "Failed to parse HTTP rule for {service_name}.{}: {e}",
                        method.name()
                    ),
                })?;
                http_rule = Some(rule);
            }
            super::GNOSTIC_OPERATION_EXTENSION => match Operation::decode(data) {
                Ok(op) => operation = Some(op),
                Err(e) => warn!("Failed to parse gnostic operation: {}", e),
            },
            _ => {
                debug!(
                    "Unknown extension field {} in {}.{}",
                    field_number,
                    service_name,
                    method.name()
                );
            }
        }
    }

    let http_rule = http_rule.ok_or_else(missing_http_rule)?;

    Ok((operation, http_rule))
}

#[cfg(test)]
mod tests {
    use protobuf::descriptor::MethodOptions;

    use super::*;

    #[test]
    fn test_extract_service_documentation() {
        let mut sci = SourceCodeInfo::new();
        let mut location = protobuf::descriptor::source_code_info::Location::new();
        location.path = vec![6, 0];
        location.set_leading_comments("This is a test service for documentation.".to_string());
        sci.location.push(location);

        let result = extract_documentation(Some(&sci), &[6, 0]);
        assert_eq!(
            result.as_deref(),
            Some("This is a test service for documentation.")
        );
    }

    #[test]
    fn test_extract_method_documentation() {
        let mut sci = SourceCodeInfo::new();
        let mut location = protobuf::descriptor::source_code_info::Location::new();
        location.path = vec![6, 0, 2, 0];
        location.set_leading_comments("This method does something useful.".to_string());
        sci.location.push(location);

        let result = extract_documentation(Some(&sci), &[6, 0, 2, 0]);
        assert_eq!(
            result.as_deref(),
            Some("This method does something useful.")
        );
    }

    #[test]
    fn test_missing_http_rule_causes_error() {
        // Create a method without any HTTP rule annotation
        let method = MethodDescriptorProto {
            name: Some("TestMethod".to_string()),
            input_type: Some(".test.TestRequest".to_string()),
            output_type: Some(".test.TestResponse".to_string()),
            options: Some(MethodOptions::default()).into(), // Empty options, no HTTP rule
            ..Default::default()
        };

        // This should return an error since HTTP rule is required
        let result = extract_method_annotations(&method, "TestService");
        assert!(result.is_err());

        let error_message = result.unwrap_err().to_string();
        assert!(error_message.contains("missing required google.api.http annotation"));
    }

    #[test]
    fn test_method_without_options_causes_error() {
        // Create a method without any options at all
        let method = MethodDescriptorProto {
            name: Some("TestMethod".to_string()),
            input_type: Some(".test.TestRequest".to_string()),
            output_type: Some(".test.TestResponse".to_string()),
            options: None.into(), // No options at all
            ..Default::default()
        };

        // This should return an error since HTTP rule is required
        let result = extract_method_annotations(&method, "TestService");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            Error::MissingAnnotation { .. }
        ));
    }
}
