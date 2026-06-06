use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::analysis::{GenerationPlan, RequestType, is_standard_list_field};
use crate::google::api::FieldBehavior;
use crate::parsing::CodeGenMetadata;
use crate::parsing::types::BaseType;

use super::{CodeGenConfig, format_tokens};

/// Generate the `labels.rs` file containing `Resource` and `ObjectLabel` enums
/// derived from `google.api.resource` annotations on message types.
///
/// The package prefix is inferred from the service packages in `plan`: the longest
/// common dot-delimited prefix across all services, formatted as `".<prefix>."`.
/// The `super::` depth is always `1` since `labels.rs` is placed one level inside
/// the models subdirectory alongside the service `pub mod` blocks.
///
/// When `error_type_path` is `Some`, also emits:
/// - An inherent `Resource::resource_label()` method
/// - `From<T> for Resource` and `TryFrom<Resource> for T` impls for each resource type
///
/// When `config.generate_object_conversions` is `true`, also emits:
/// - A `::olai_derive::object_conversions!` invocation for all resources
///   that have an `IDENTIFIER`-annotated field
/// - A `qualified_name()` inherent method on each resource type
pub(crate) fn generate_resource_enum(
    plan: &GenerationPlan,
    metadata: &CodeGenMetadata,
    config: &CodeGenConfig,
    error_type_path: Option<&str>,
) -> crate::error::Result<String> {
    if !config.generate_resource_enum {
        return Ok(String::new());
    }

    // Infer package prefix from service packages (e.g. "unitycatalog.catalogs.v1" → ".unitycatalog.")
    let package_prefix = infer_package_prefix(
        &plan
            .services
            .iter()
            .map(|s| s.package.as_str())
            .collect::<Vec<_>>(),
    );

    // Collect all messages that have a resource annotation matching the inferred prefix.
    //
    // Proto-derived `rust_path` strings are parsed (and validated) exactly once here. A
    // malformed path hard-fails with [`Error::InvalidRustPath`] naming the offending proto
    // message, rather than panicking deep in token generation.
    let mut resources: Vec<ResourceEntry> = Vec::new();
    for (name, info) in &metadata.messages {
        let Some(rd) = info.resource_descriptor.as_ref() else {
            continue;
        };
        // Only include packages matching the inferred prefix (excludes google/gnostic messages)
        if !name.starts_with(&package_prefix) {
            continue;
        }
        // Extract variant name from resource type (e.g. "acme.io/Widget" -> "Widget")
        let variant_name = match rd.r#type.split('/').next_back() {
            Some(v) if !v.is_empty() => v.to_string(),
            _ => {
                tracing::warn!(
                    "Skipping resource `{}`: type `{}` has no `/`-separated variant name",
                    name,
                    rd.r#type
                );
                continue;
            }
        };
        // labels.rs always lives one level inside the models subdir, so super:: reaches the subdir
        // module which has all the service pub mods as siblings.
        let Some(rust_path) = message_name_to_rust_path(name, &package_prefix, 1) else {
            continue;
        };

        // Validate the proto-derived path once, attributing failures to the proto message.
        let rust_type: syn::Type =
            syn::parse_str(&rust_path).map_err(|e| crate::error::Error::InvalidRustPath {
                path: rust_path.clone(),
                message: name.clone(),
                source: e,
            })?;

        // Find the IDENTIFIER-annotated field
        let id_field = info
            .fields
            .iter()
            .find(|f| f.field_behavior.contains(&FieldBehavior::Identifier));
        let (id_field_name, id_is_optional) = match id_field {
            Some(f) => (Some(f.name.clone()), f.unified_type.is_optional),
            None => (None, false),
        };

        // Derive path_names from the service plan for this resource.
        // A resource is hierarchical if its descriptor explicitly sets name_field (any value)
        // OR if the message has a full_name field (server-computed dot-joined composite).
        let message_has_full_name = info.fields.iter().any(|f| f.name == "full_name");
        // The leaf name component. Defaults to `name`, but `google.api.resource.name_field`
        // may point at a different scalar leaf field (e.g. `tag_key`). `full_name` is excluded:
        // it denotes a composite full name that is *decomposed* into parents + leaf, not the leaf
        // itself.
        let leaf_name = if !rd.name_field.is_empty()
            && rd.name_field != "full_name"
            && info.fields.iter().any(|f| f.name == rd.name_field)
        {
            rd.name_field.clone()
        } else {
            "name".to_string()
        };
        let path_names = derive_path_names(
            &rd.singular,
            !rd.name_field.is_empty() || message_has_full_name,
            &leaf_name,
            plan,
            metadata,
        );

        // Compute field descriptors with roles for the resource registry.
        let known_managed_fields: &[&str] =
            &["created_at", "updated_at", "created_by", "updated_by"];
        let field_descriptors: Vec<FieldDescriptorEntry> = info
            .fields
            .iter()
            .map(|f| {
                let role = if f.field_behavior.contains(&FieldBehavior::Identifier) {
                    FieldRoleEntry::Identifier
                } else if f.is_sensitive {
                    FieldRoleEntry::Sensitive
                } else if f.field_behavior.contains(&FieldBehavior::OutputOnly)
                    && known_managed_fields.contains(&f.name.as_str())
                {
                    FieldRoleEntry::Managed
                } else {
                    FieldRoleEntry::Data
                };
                FieldDescriptorEntry {
                    name: f.name.clone(),
                    role,
                }
            })
            .collect();

        resources.push(ResourceEntry {
            variant_name,
            rust_path,
            rust_type,
            singular: rd.singular.clone(),
            id_field: id_field_name,
            id_is_optional,
            path_names,
            has_full_name: message_has_full_name,
            field_descriptors,
        });
    }

    // Sort deterministically by singular name
    resources.sort_by(|a, b| a.singular.cmp(&b.singular));

    let (resource_variants, label_variants) = build_variants(&resources);

    // Inherent impl and From/TryFrom impls — only emitted when error_type_path is set.
    let extra_impls = build_tryfrom_impls(&resources, error_type_path)?;

    // Object conversion impl blocks and qualified_name() methods.
    let object_conversions_impl = build_object_conversions(&resources, config)?;

    let tokens = quote! {
        /// All resource types managed by the service.
        #[allow(clippy::derive_partial_eq_without_eq)]
        #[derive(Clone, Debug, PartialEq)]
        pub enum Resource {
            #(#resource_variants),*
        }

        /// Discriminant label for each resource type.
        #[derive(
            ::strum::AsRefStr,
            ::strum::Display,
            ::strum::EnumIter,
            ::strum::EnumString,
            ::serde::Serialize,
            ::serde::Deserialize,
            Hash,
            Clone,
            Copy,
            Debug,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
        )]
        #[strum(serialize_all = "snake_case", ascii_case_insensitive)]
        #[serde(rename_all = "snake_case")]
        #[cfg_attr(feature = "sqlx", derive(::sqlx::Type))]
        #[cfg_attr(
            feature = "sqlx",
            sqlx(type_name = "object_label", rename_all = "snake_case")
        )]
        pub enum ObjectLabel {
            #(#label_variants),*
        }

        #extra_impls

        #object_conversions_impl
    };

    // Generate the resource descriptor registry and Label impl (only when store integration is enabled)
    let registry_impl = if config.generate_store_integration {
        generate_resource_registry(&resources, config, plan, metadata)
    } else {
        quote! {}
    };

    let all_tokens = quote! {
        #tokens

        #registry_impl
    };

    format_tokens(all_tokens)
}

/// Build the `Resource` (typed) and `ObjectLabel` (discriminant) enum variant token lists.
fn build_variants(resources: &[ResourceEntry]) -> (Vec<TokenStream>, Vec<TokenStream>) {
    let resource_variants = resources
        .iter()
        .map(|r| {
            let variant = format_ident!("{}", r.variant_name);
            let path = &r.rust_type;
            quote! { #variant(#path) }
        })
        .collect();

    let label_variants = resources
        .iter()
        .map(|r| {
            let variant = format_ident!("{}", r.variant_name);
            quote! { #variant }
        })
        .collect();

    (resource_variants, label_variants)
}

/// Build the `resource_label()` accessor plus `From<T>`/`TryFrom<Resource>` impls.
///
/// Emitted only when `error_type_path` is set; returns an empty token stream otherwise.
/// Fails with [`crate::error::Error::InvalidErrorTypePath`] if `error_type_path` is not a
/// valid Rust type.
fn build_tryfrom_impls(
    resources: &[ResourceEntry],
    error_type_path: Option<&str>,
) -> crate::error::Result<TokenStream> {
    let Some(error_path) = error_type_path else {
        return Ok(quote! {});
    };

    let error_ty: syn::Type =
        syn::parse_str(error_path).map_err(|e| crate::error::Error::InvalidErrorTypePath {
            path: error_path.to_string(),
            source: e,
        })?;

    let label_arms: Vec<TokenStream> = resources
        .iter()
        .map(|r| {
            let variant = format_ident!("{}", r.variant_name);
            quote! { Resource::#variant(_) => &ObjectLabel::#variant, }
        })
        .collect();

    // With a single resource variant, `Resource::Variant(v) => Ok(v)` is already exhaustive;
    // a trailing `_ =>` arm would be unreachable. Only emit the catch-all when needed.
    let single_variant = resources.len() == 1;
    let from_impls: Vec<TokenStream> = resources
        .iter()
        .map(|r| {
            let variant = format_ident!("{}", r.variant_name);
            let path = &r.rust_type;
            let mismatch_arm = if single_variant {
                quote! {}
            } else {
                quote! {
                    _ => Err(<#error_ty>::generic(concat!(
                        "Resource is not a ",
                        stringify!(#variant)
                    ))),
                }
            };
            quote! {
                impl From<#path> for Resource {
                    fn from(v: #path) -> Self {
                        Resource::#variant(v)
                    }
                }

                impl TryFrom<Resource> for #path {
                    type Error = #error_ty;

                    fn try_from(r: Resource) -> Result<Self, Self::Error> {
                        match r {
                            Resource::#variant(v) => Ok(v),
                            #mismatch_arm
                        }
                    }
                }
            }
        })
        .collect();

    Ok(quote! {
        impl Resource {
            /// Return the discriminant label for this resource.
            pub fn resource_label(&self) -> &ObjectLabel {
                match self {
                    #(#label_arms)*
                }
            }
        }

        #(#from_impls)*
    })
}

/// Build the `Object` conversion impls and `qualified_name()` inherent methods.
///
/// Emitted only when `config.generate_object_conversions` is set; resources without an
/// `IDENTIFIER`-annotated field are skipped. Returns an empty token stream when disabled.
fn build_object_conversions(
    resources: &[ResourceEntry],
    config: &CodeGenConfig,
) -> crate::error::Result<TokenStream> {
    if !config.generate_object_conversions {
        return Ok(quote! {});
    }

    let mut conversion_impls: Vec<TokenStream> = Vec::new();
    let mut qualified_name_impls: Vec<TokenStream> = Vec::new();

    for r in resources {
        let Some(ref id_field) = r.id_field else {
            // No IDENTIFIER annotation — skip
            continue;
        };

        let path = &r.rust_type;
        // `ObjectLabel::<variant>` is built from a validated variant ident, so it always
        // parses; surface any failure as an error rather than panicking.
        let label_expr: syn::Expr = syn::parse_str(&format!("ObjectLabel::{}", r.variant_name))
            .map_err(|e| crate::error::Error::InvalidRustPath {
                path: format!("ObjectLabel::{}", r.variant_name),
                message: r.rust_path.clone(),
                source: e,
            })?;
        let id_ident = format_ident!("{}", id_field);
        let is_optional = r.id_is_optional;

        let path_name_idents: Vec<proc_macro2::Ident> = r
            .path_names
            .iter()
            .map(|n| format_ident!("{}", n))
            .collect();

        conversion_impls.push(emit_from_object(path, &id_ident, is_optional));
        conversion_impls.push(emit_to_object(path, &label_expr, &id_ident, is_optional));
        conversion_impls.push(emit_resource_impl(
            path,
            &label_expr,
            &id_ident,
            &path_name_idents,
            is_optional,
        ));

        // qualified_name() impl
        let format_expr: TokenStream = build_qualified_name_expr(&r.path_names);
        qualified_name_impls.push(quote! {
            impl #path {
                /// Returns the fully-qualified dot-separated name computed from component fields.
                pub fn qualified_name(&self) -> String {
                    #format_expr
                }
            }
        });
    }

    // Imports are only used by the conversion impls; emitting them when every resource was
    // skipped (no IDENTIFIER field) would leave dead `use` statements behind.
    let imports = if conversion_impls.is_empty() {
        quote! {}
    } else {
        quote! {
            use crate::Error;
            use crate::models::object::Object;
            use crate::models::resources::{ResourceExt, ResourceIdent, ResourceName, ResourceRef};
        }
    };

    Ok(quote! {
        #imports

        #(#conversion_impls)*

        #(#qualified_name_impls)*
    })
}

struct ResourceEntry {
    variant_name: String,
    rust_path: String,
    /// Pre-parsed form of [`rust_path`](ResourceEntry::rust_path), validated once at
    /// analysis time so token generation cannot fail.
    rust_type: syn::Type,
    singular: String,
    /// Field name carrying `FieldBehavior::Identifier`, if present.
    id_field: Option<String>,
    /// Whether the IDENTIFIER field is `optional`.
    id_is_optional: bool,
    /// Ordered list of field names used to build `ResourceName`, e.g. `["catalog_name", "schema_name", "name"]`.
    path_names: Vec<String>,
    /// Whether the message has a `full_name` field (used for `qualified_name()` generation).
    #[allow(dead_code)]
    has_full_name: bool,
    /// All fields with their computed roles for the resource descriptor registry.
    field_descriptors: Vec<FieldDescriptorEntry>,
}

/// A field entry for the generated resource descriptor registry.
struct FieldDescriptorEntry {
    name: String,
    role: FieldRoleEntry,
}

/// The computed role of a field, matching `olai_store::FieldRole`.
enum FieldRoleEntry {
    Data,
    Identifier,
    Sensitive,
    Managed,
}

/// Derive the ordered list of field names used to build a `ResourceName` for a resource.
///
/// **Annotation-driven path** (preferred): when the service for `singular` has
/// `hierarchy` entries from `resource_reference { child_type }` annotations, the
/// parent field names are taken directly from those entries (in the order they appear
/// as List method query params), followed by `"name"`.
///
/// **Heuristic fallback** (when no annotations present): uses the same two-signal logic:
/// 1. `name_field` non-empty on the descriptor → resource has decomposable composite name
/// 2. Check the List method's required string-typed query params for parent names
///
/// Returns e.g. `["catalog_name", "schema_name", "name"]` for Table,
/// `["catalog_name", "name"]` for Schema, `["name"]` for Catalog.
fn derive_path_names(
    singular: &str,
    has_full_name_field: bool,
    leaf_name: &str,
    plan: &GenerationPlan,
    metadata: &CodeGenMetadata,
) -> Vec<String> {
    // Find the service whose singular resource name matches
    let service = plan.services.iter().find(|s| {
        s.managed_resources
            .iter()
            .any(|r| r.descriptor.singular == singular)
    });

    let Some(service) = service else {
        return vec![leaf_name.to_string()];
    };

    // Find this resource's type string from metadata
    let resource_type = metadata
        .resource_from_singular(singular)
        .map(|rd| rd.r#type.clone())
        .unwrap_or_default();

    // --- Annotation-driven path ---
    // Collect hierarchy entries for this resource type, in List-method param order.
    if !service.hierarchy.is_empty() && !resource_type.is_empty() {
        let annotation_parents: Vec<String> = service
            .hierarchy
            .iter()
            .filter(|h| h.child_resource_type == resource_type)
            .map(|h| h.parent_field_name.clone())
            .collect();

        if !annotation_parents.is_empty() {
            let mut params = annotation_parents;
            params.push(leaf_name.to_string());
            return params;
        }
    }

    // --- Heuristic fallback ---
    // Get the Get method's path param name
    let get_path_param = service
        .methods
        .iter()
        .find(|m| m.request_type == RequestType::Get)
        .and_then(|m| m.path_parameters().next().map(|p| p.name.clone()));

    // Get the List method's string query params that look like parent-hierarchy names.
    //
    // proto3 scalars have no presence, so `is_optional()` is effectively always false for them
    // — we can't use it to tell a real parent name (e.g. `catalog_name`) apart from a standard
    // pagination/listing field. Exclude the well-known AIP-132 List fields explicitly so they
    // don't get misread as path components (e.g. `page_token` leaking into a resource name).
    let parent_params: Vec<String> = service
        .methods
        .iter()
        .find(|m| m.request_type == RequestType::List)
        .map(|m| {
            m.parameters
                .iter()
                .filter(|p| !p.is_path_param() && !p.is_optional())
                .filter(|p| matches!(p.field_type().base_type, BaseType::String))
                .filter(|p| !is_standard_list_field(p.name()))
                .map(|p| p.name().to_string())
                .collect()
        })
        .unwrap_or_default();

    let should_decompose = has_full_name_field
        || (get_path_param.as_deref() == Some("name") && !parent_params.is_empty());

    if should_decompose {
        let mut params = parent_params;
        params.push(format!("{singular}_name"));
        // Replace the final `{singular}_name` with the leaf field name (usually `name`, or the
        // resource's `name_field` when it diverges, e.g. `tag_key`).
        // last_mut() is infallible: we just pushed an element above.
        let last = params.last_mut().unwrap();
        *last = leaf_name.to_string();
        params
    } else {
        vec![leaf_name.to_string()]
    }
}

/// Build a `qualified_name()` return expression from an ordered list of path field names.
///
/// - `["name"]` → `self.name.clone()`
/// - `["catalog_name", "name"]` → `format!("{}.{}", self.catalog_name, self.name)`
/// - `["catalog_name", "schema_name", "name"]` → `format!("{}.{}.{}", ...)`
fn build_qualified_name_expr(path_names: &[String]) -> TokenStream {
    if path_names.len() == 1 {
        let field = format_ident!("{}", &path_names[0]);
        return quote! { self.#field.clone() };
    }
    let format_str = path_names
        .iter()
        .map(|_| "{}")
        .collect::<Vec<_>>()
        .join(".");
    let field_refs: Vec<TokenStream> = path_names
        .iter()
        .map(|n| {
            let ident = format_ident!("{}", n);
            quote! { self.#ident }
        })
        .collect();
    quote! { format!(#format_str, #(#field_refs),*) }
}

/// Infer the package prefix from a list of proto package names.
///
/// Finds the longest common leading dot-segment and returns it as `".<prefix>."`.
///
/// Examples:
/// - `["unitycatalog.catalogs.v1", "unitycatalog.tables.v1"]` → `".unitycatalog."`
/// - `["example.catalog.v1"]` → `".example."`
fn infer_package_prefix(packages: &[&str]) -> String {
    if packages.is_empty() {
        return String::new();
    }
    let first_parts: Vec<&str> = packages[0].split('.').collect();
    let _common_len = first_parts
        .iter()
        .enumerate()
        .take_while(|(i, seg)| {
            packages
                .iter()
                .skip(1)
                .all(|p| p.split('.').nth(*i) == Some(seg))
        })
        .count();
    // Take only the top-level shared segment (one dot-level), not the full common prefix,
    // so version segments like "v1" don't get included when all packages share them.
    // Use the first segment as the meaningful namespace prefix.
    format!(".{}.", first_parts[0])
}

/// Convert a fully-qualified protobuf message name to a Rust type path relative to
/// `labels.rs` inside the models subdirectory.
///
/// `prefix` is stripped from the message name (e.g. `".unitycatalog."`).
/// One `super::` hop is prepended since `labels.rs` is a sibling of the service modules
/// inside the same generated subdirectory.
///
/// Examples (prefix = `".unitycatalog."`):
/// - `.unitycatalog.catalogs.v1.Catalog` → `super::catalogs::v1::Catalog`
/// - `.unitycatalog.external_locations.v1.ExternalLocation` → `super::external_locations::v1::ExternalLocation`
fn message_name_to_rust_path(name: &str, prefix: &str, super_levels: u32) -> Option<String> {
    // Strip leading prefix (e.g. ".unitycatalog.")
    let without_prefix = name.strip_prefix(prefix)?;
    // Split remaining parts and join with `::`
    let parts: Vec<&str> = without_prefix.split('.').collect();
    if parts.is_empty() {
        return None;
    }
    let super_prefix = "super::".repeat(super_levels as usize);
    Some(format!("{}{}", super_prefix, parts.join("::")))
}

/// Generate the `RESOURCE_DESCRIPTORS` static registry and `Label` impl for `ObjectLabel`.
///
/// This emits:
/// 1. `impl olai_store::Label for ObjectLabel` — making the generated
///    label type compatible with the generic resource store.
/// 2. `pub static RESOURCE_DESCRIPTORS: &[ResourceTypeDescriptor]` — a static registry
///    of all resource types with field roles, path names, and parent relationships.
fn generate_resource_registry(
    resources: &[ResourceEntry],
    config: &CodeGenConfig,
    plan: &GenerationPlan,
    metadata: &CodeGenMetadata,
) -> TokenStream {
    let store_crate = format_ident!("{}", config.resource_store_crate_name);

    // --- Label impl for ObjectLabel ---
    let label_impl = quote! {
        impl ::#store_crate::Label for ObjectLabel {
            fn as_str(&self) -> &str {
                // strum's AsRefStr gives us the snake_case string
                self.as_ref()
            }
        }
    };

    // --- RESOURCE_DESCRIPTORS static ---
    // Compute parent_label for each resource.
    // Annotation-driven: look for hierarchy entries across all services where
    // child_resource_type matches this resource's type string. The parent singular
    // is stored directly on the hierarchy entry.
    // Heuristic fallback: for resources without annotation data, strip "_name" from
    // the second-to-last path_names component and match against known resource singulars.
    let parent_labels: Vec<Option<String>> = resources
        .iter()
        .map(|r| {
            if r.path_names.len() <= 1 {
                return None;
            }

            // Try annotation-driven path first
            let resource_type = metadata
                .resource_from_singular(&r.singular)
                .map(|rd| rd.r#type.as_str())
                .unwrap_or("");
            if !resource_type.is_empty() {
                for service in &plan.services {
                    for h in &service.hierarchy {
                        if h.child_resource_type == resource_type {
                            if let Some(ref parent_sing) = h.parent_singular {
                                let found = resources.iter().find_map(|candidate| {
                                    if candidate.singular == *parent_sing {
                                        Some(candidate.variant_name.clone())
                                    } else {
                                        None
                                    }
                                });
                                if found.is_some() {
                                    return found;
                                }
                            }
                        }
                    }
                }
            }

            // Heuristic fallback: strip "_name" from second-to-last path component
            let parent_path_component = &r.path_names[r.path_names.len() - 2];
            let parent_singular = parent_path_component
                .strip_suffix("_name")
                .unwrap_or(parent_path_component);
            resources.iter().find_map(|candidate| {
                if candidate.singular == parent_singular {
                    Some(candidate.variant_name.clone())
                } else {
                    None
                }
            })
        })
        .collect();

    let descriptor_entries: Vec<TokenStream> = resources
        .iter()
        .zip(parent_labels.iter())
        .map(|(r, parent)| {
            let label_variant = format_ident!("{}", r.variant_name);

            let field_entries: Vec<TokenStream> = r
                .field_descriptors
                .iter()
                .map(|fd| {
                    let name = &fd.name;
                    let role = match fd.role {
                        FieldRoleEntry::Data => {
                            quote! { ::#store_crate::FieldRole::Data }
                        }
                        FieldRoleEntry::Identifier => {
                            quote! { ::#store_crate::FieldRole::Identifier }
                        }
                        FieldRoleEntry::Sensitive => {
                            quote! { ::#store_crate::FieldRole::Sensitive }
                        }
                        FieldRoleEntry::Managed => {
                            quote! { ::#store_crate::FieldRole::Managed }
                        }
                    };
                    quote! {
                        ::#store_crate::ResourceFieldDescriptor {
                            name: #name,
                            role: #role,
                        }
                    }
                })
                .collect();

            let path_name_strs: Vec<&str> = r.path_names.iter().map(|s| s.as_str()).collect();

            let parent_expr = match parent {
                Some(parent_name) => {
                    let parent_variant = format_ident!("{}", parent_name);
                    quote! { Some(ObjectLabel::#parent_variant) }
                }
                None => quote! { None },
            };

            quote! {
                ::#store_crate::ResourceTypeDescriptor {
                    label: ObjectLabel::#label_variant,
                    fields: &[#(#field_entries),*],
                    path_names: &[#(#path_name_strs),*],
                    parent_label: #parent_expr,
                }
            }
        })
        .collect();

    let registry = quote! {
        /// Static resource type descriptors derived from proto annotations.
        ///
        /// Each entry describes a resource type's fields (with roles: data, identifier,
        /// sensitive, managed), hierarchical name components, and parent relationship.
        ///
        /// Use `ResourceRegistry::from_static` to build a runtime registry from this data.
        pub static RESOURCE_DESCRIPTORS: &[::#store_crate::ResourceTypeDescriptor<ObjectLabel>] = &[
            #(#descriptor_entries),*
        ];
    };

    quote! {
        #label_impl
        #registry
    }
}

// ---------------------------------------------------------------------------
// Object conversion helpers (emit the impl blocks formerly produced by
// olai_derive::object_conversions!)
// ---------------------------------------------------------------------------

fn emit_from_object(
    path: &syn::Type,
    id_ident: &proc_macro2::Ident,
    is_optional: bool,
) -> TokenStream {
    let id_assignment = if is_optional {
        quote! { res.#id_ident = Some(object.id.hyphenated().to_string()); }
    } else {
        quote! { res.#id_ident = object.id.hyphenated().to_string(); }
    };
    quote! {
        impl TryFrom<Object> for #path {
            type Error = Error;

            fn try_from(object: Object) -> Result<Self, Self::Error> {
                let props = object
                    .properties
                    .ok_or_else(|| Error::generic("expected properties"))?;
                let mut res: #path = ::serde_json::from_value(props)?;
                #id_assignment
                Ok(res)
            }
        }
    }
}

fn emit_to_object(
    path: &syn::Type,
    label_expr: &syn::Expr,
    id_ident: &proc_macro2::Ident,
    is_optional: bool,
) -> TokenStream {
    let id_field = if is_optional {
        quote! {
            let id = obj
                .#id_ident
                .as_ref()
                .map(|id| ::uuid::Uuid::parse_str(id))
                .transpose()?
                .unwrap_or_else(|| ::uuid::Uuid::nil());
        }
    } else {
        quote! {
            let id = ::uuid::Uuid::parse_str(&obj.#id_ident).unwrap_or_else(|_| ::uuid::Uuid::nil());
        }
    };
    quote! {
        impl TryFrom<#path> for Object {
            type Error = Error;

            fn try_from(obj: #path) -> Result<Self, Self::Error> {
                #id_field
                Ok(Object {
                    id,
                    name: obj.resource_name(),
                    label: #label_expr,
                    properties: Some(::serde_json::to_value(obj)?),
                    updated_at: None,
                    created_at: chrono::Utc::now(),
                })
            }
        }
    }
}

fn emit_resource_impl(
    path: &syn::Type,
    label_expr: &syn::Expr,
    id_ident: &proc_macro2::Ident,
    path_name_idents: &[proc_macro2::Ident],
    is_optional: bool,
) -> TokenStream {
    let resource_ref = if is_optional {
        quote! {
            self
                .#id_ident
                .as_ref()
                .and_then(|id| ::uuid::Uuid::parse_str(id).ok())
                .map(ResourceRef::Uuid)
                .unwrap_or_else(|| ResourceRef::Name(self.resource_name()))
        }
    } else {
        quote! {
            ::uuid::Uuid::parse_str(&self.#id_ident)
                .ok()
                .map(ResourceRef::Uuid)
                .unwrap_or_else(|| ResourceRef::Name(self.resource_name()))
        }
    };
    quote! {
        impl ResourceExt for #path {
            fn resource_name(&self) -> ResourceName {
                ResourceName::new([#(&self.#path_name_idents),*])
            }
            fn resource_ref(&self) -> ResourceRef {
                #resource_ref
            }
            fn resource_ident(&self) -> ResourceIdent {
                (#label_expr).to_ident(self.resource_ref())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{GenerationPlan, ServicePlan};
    use crate::google::api::ResourceDescriptor;
    use crate::parsing::{CodeGenMetadata, MessageInfo};
    use std::collections::HashMap;

    fn config_with_resource_enum(error_type_path: Option<String>) -> CodeGenConfig {
        CodeGenConfig {
            context_type_path: "crate::Context".into(),
            result_type_path: "crate::Result".into(),
            models_path_template: "example_common::models::{service}::v1".into(),
            models_path_crate_template: "crate::models::{service}::v1".into(),
            resource_store_crate_name: "olai_store".into(),
            output: crate::codegen::CodeGenOutput {
                common: "/tmp/common".into(),
                models: Some("/tmp/models".into()),
                models_subdir: "_gen".into(),
                server: None,
                client: None,
                python: None,
                node: None,
                node_ts: None,
                python_typings_filename: "client.pyi".into(),
                generate_resource_clients: false,
            },
            generate_resource_enum: true,
            generate_store_integration: false,
            error_type_path,
            generate_object_conversions: false,
            bindings: None,
            models_gen_dir: None,
        }
    }

    fn plan_for_package(package: &str) -> GenerationPlan {
        GenerationPlan {
            services: vec![ServicePlan {
                service_name: "WidgetService".into(),
                handler_name: "WidgetHandler".into(),
                base_path: "widgets".into(),
                package: package.into(),
                methods: vec![],
                managed_resources: vec![],
                documentation: None,
                hierarchy: vec![],
                resource_accessor_params: None,
                direct_children: vec![],
            }],
            skipped_methods: vec![],
        }
    }

    fn metadata_with_message(name: &str) -> CodeGenMetadata {
        let mut messages = HashMap::new();
        messages.insert(
            name.to_string(),
            MessageInfo {
                name: name.rsplit('.').next().unwrap_or(name).to_string(),
                fields: vec![],
                resource_descriptor: Some(ResourceDescriptor {
                    r#type: "example.io/Widget".into(),
                    pattern: vec!["widgets/{widget}".into()],
                    name_field: String::new(),
                    history: 0,
                    plural: "widgets".into(),
                    singular: "widget".into(),
                    style: vec![],
                }),
                documentation: None,
            },
        );
        CodeGenMetadata {
            messages,
            ..Default::default()
        }
    }

    /// 1.3 — a proto message whose derived Rust path is not a valid `syn::Type`
    /// must produce `Error::InvalidRustPath` naming the offending message, not a panic.
    #[test]
    fn malformed_rust_path_returns_invalid_rust_path_error() {
        // Package prefix is ".example.", so ".example.1bad.v1.Widget" → "super::1bad::v1::Widget".
        // `1bad` is not a valid Rust identifier, so the path fails to parse.
        let bad_name = ".example.1bad.v1.Widget";
        let plan = plan_for_package("example.1bad.v1");
        let metadata = metadata_with_message(bad_name);
        let config = config_with_resource_enum(None);

        let err = generate_resource_enum(&plan, &metadata, &config, None)
            .expect_err("expected a typed error, not a panic or stub");
        match err {
            crate::error::Error::InvalidRustPath { message, path, .. } => {
                assert_eq!(
                    message, bad_name,
                    "error should name the offending proto message"
                );
                assert!(
                    path.contains("1bad"),
                    "error should include the bad path: {path}"
                );
            }
            other => panic!("expected InvalidRustPath, got {other:?}"),
        }
    }

    /// 1.3 — a malformed `error_type_path` must produce `Error::InvalidErrorTypePath`.
    #[test]
    fn malformed_error_type_path_returns_error() {
        let plan = plan_for_package("example.widgets.v1");
        let metadata = metadata_with_message(".example.widgets.v1.Widget");
        let config = config_with_resource_enum(Some("not a type!!".into()));

        let err = generate_resource_enum(&plan, &metadata, &config, Some("not a type!!"))
            .expect_err("expected a typed error");
        assert!(
            matches!(err, crate::error::Error::InvalidErrorTypePath { .. }),
            "expected InvalidErrorTypePath, got {err:?}"
        );
    }

    /// Sanity: a well-formed proto-derived path generates successfully.
    #[test]
    fn well_formed_path_generates() {
        let plan = plan_for_package("example.widgets.v1");
        let metadata = metadata_with_message(".example.widgets.v1.Widget");
        let config = config_with_resource_enum(Some("crate::Error".into()));

        let out = generate_resource_enum(&plan, &metadata, &config, Some("crate::Error"))
            .expect("well-formed path should generate");
        assert!(out.contains("super::widgets::v1::Widget"), "output: {out}");
    }

    #[test]
    fn test_message_name_to_rust_path() {
        assert_eq!(
            message_name_to_rust_path(".unitycatalog.catalogs.v1.Catalog", ".unitycatalog.", 1),
            Some("super::catalogs::v1::Catalog".to_string())
        );
        assert_eq!(
            message_name_to_rust_path(
                ".unitycatalog.external_locations.v1.ExternalLocation",
                ".unitycatalog.",
                1
            ),
            Some("super::external_locations::v1::ExternalLocation".to_string())
        );
        assert_eq!(
            message_name_to_rust_path(".google.api.Something", ".unitycatalog.", 1),
            None
        );
    }

    #[test]
    fn test_infer_package_prefix() {
        assert_eq!(
            infer_package_prefix(&["unitycatalog.catalogs.v1", "unitycatalog.tables.v1"]),
            ".unitycatalog."
        );
        assert_eq!(infer_package_prefix(&["example.catalog.v1"]), ".example.");
        assert_eq!(
            infer_package_prefix(&["example.catalog.v1", "example.items.v1"]),
            ".example."
        );
    }

    #[test]
    fn test_build_qualified_name_expr_flat() {
        let expr = build_qualified_name_expr(&["name".to_string()]);
        let s = expr.to_string();
        assert!(s.contains("self"), "expr: {s}");
        assert!(s.contains("name"), "expr: {s}");
        assert!(s.contains("clone"), "expr: {s}");
    }

    #[test]
    fn test_build_qualified_name_expr_hierarchical() {
        let expr = build_qualified_name_expr(&[
            "catalog_name".to_string(),
            "schema_name".to_string(),
            "name".to_string(),
        ]);
        let s = expr.to_string();
        assert!(s.contains("format"), "expr: {s}");
        assert!(s.contains("catalog_name"), "expr: {s}");
        assert!(s.contains("schema_name"), "expr: {s}");
    }
}
