//! Shared utilities for the olai-codegen crate
//!
//! This module contains common functions used across different parts of the code generation
//! pipeline to reduce duplication and improve maintainability.
use convert_case::{Case, Casing};

/// Extract the last dot-separated segment of a fully-qualified protobuf type name.
///
/// For example, `".unitycatalog.catalog.v1.Catalog"` returns `"Catalog"`.
/// If the name contains no dots, the original string is returned unchanged.
pub fn extract_simple_type_name(name: &str) -> String {
    name.split('.').next_back().unwrap_or(name).to_string()
}

/// Derive the **emitted** type name for a (possibly nested) protobuf type from its fully-qualified
/// name, disambiguating nested types by their parent message.
///
/// Package and version segments are lowercase; type segments are PascalCase. This joins the trailing
/// run of type segments (everything from the first PascalCase segment onward), so a top-level type is
/// unchanged while a nested type is prefixed by its enclosing message(s):
///
/// - `".pkg.v1.Catalog"` → `"Catalog"`
/// - `".pkg.v1.GenerateTemporaryTableCredentialsRequest.Operation"` →
///   `"GenerateTemporaryTableCredentialsRequestOperation"`
///
/// This keeps emitted names collision-free when the same nested name (e.g. `Operation`) appears in
/// multiple messages, and must be used consistently for both type *definitions* and *references*.
pub fn extract_qualified_type_name(name: &str) -> String {
    let segments: Vec<&str> = name.split('.').filter(|s| !s.is_empty()).collect();
    // A type segment starts with an ASCII uppercase letter (PascalCase); package/version segments
    // do not. Find the first such segment and join from there to the end.
    let start = segments
        .iter()
        .position(|s| s.starts_with(|c: char| c.is_ascii_uppercase()));
    match start {
        Some(i) => segments[i..].concat(),
        // No PascalCase segment (shouldn't happen for a real type) — fall back to the last segment.
        None => extract_simple_type_name(name),
    }
}

/// String manipulation utilities
pub mod strings {
    use super::*;

    /// Convert service name to handler trait name
    /// e.g., "CatalogsService" -> "CatalogHandler", "TagPoliciesService" -> "TagPolicyHandler"
    pub fn service_to_handler_name(service_name: &str) -> String {
        if let Some(base) = service_name.strip_suffix("Service") {
            format!("{}Handler", singularize(base))
        } else {
            format!("{service_name}Handler")
        }
    }

    /// Naive singularization of a PascalCase plural noun.
    ///
    /// Handles the common `-ies -> -y` rule (e.g. `TagPolicies` -> `TagPolicy`) before
    /// falling back to trimming a trailing `s` (e.g. `Catalogs` -> `Catalog`).
    fn singularize(word: &str) -> String {
        if let Some(stem) = word.strip_suffix("ies") {
            format!("{stem}y")
        } else {
            word.trim_end_matches('s').to_string()
        }
    }

    /// Convert operation ID to handler method name
    /// e.g., "ListCatalogs" -> "list_catalogs"
    pub fn operation_to_method_name(operation_id: &str) -> String {
        operation_id.to_case(Case::Snake)
    }

    /// Extract base path from service name
    /// e.g., "CatalogsService" -> "catalogs"
    pub fn service_to_base_path(service_name: &str) -> String {
        if let Some(base) = service_name.strip_suffix("Service") {
            base.to_case(Case::Snake)
        } else {
            service_name.to_case(Case::Snake)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod string_tests {
        use super::*;

        #[test]
        fn test_service_to_handler_name() {
            assert_eq!(
                strings::service_to_handler_name("CatalogsService"),
                "CatalogHandler"
            );
            assert_eq!(
                strings::service_to_handler_name("RecipientsService"),
                "RecipientHandler"
            );
            assert_eq!(
                strings::service_to_handler_name("SchemasService"),
                "SchemaHandler"
            );
            assert_eq!(
                strings::service_to_handler_name("TagPoliciesService"),
                "TagPolicyHandler"
            );
        }

        #[test]
        fn test_operation_to_method_name() {
            assert_eq!(
                strings::operation_to_method_name("ListCatalogs"),
                "list_catalogs"
            );
            assert_eq!(
                strings::operation_to_method_name("CreateCatalog"),
                "create_catalog"
            );
            assert_eq!(
                strings::operation_to_method_name("GetCatalog"),
                "get_catalog"
            );
        }

        #[test]
        fn test_service_to_base_path() {
            assert_eq!(strings::service_to_base_path("CatalogsService"), "catalogs");
            assert_eq!(
                strings::service_to_base_path("RecipientsService"),
                "recipients"
            );
        }
    }

    mod qualified_type_name {
        use super::*;

        #[test]
        fn top_level_type_is_unchanged() {
            assert_eq!(
                extract_qualified_type_name(".unitycatalog.catalog.v1.Catalog"),
                "Catalog"
            );
            assert_eq!(extract_qualified_type_name(".pkg.v1.UpdateX"), "UpdateX");
        }

        #[test]
        fn nested_type_is_prefixed_by_parent() {
            assert_eq!(
                extract_qualified_type_name(
                    ".unitycatalog.temporary_credentials.v1.GenerateTemporaryTableCredentialsRequest.Operation"
                ),
                "GenerateTemporaryTableCredentialsRequestOperation"
            );
        }

        #[test]
        fn distinct_parents_yield_distinct_names() {
            let a = extract_qualified_type_name(".pkg.v1.GenerateTableCredsRequest.Operation");
            let b = extract_qualified_type_name(".pkg.v1.GenerateVolumeCredsRequest.Operation");
            assert_ne!(a, b);
        }

        #[test]
        fn no_leading_dot_or_package() {
            // A bare PascalCase name is returned as-is.
            assert_eq!(extract_qualified_type_name("Catalog"), "Catalog");
        }
    }
}
