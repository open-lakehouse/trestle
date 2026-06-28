//! Analysis module for processing protobuf metadata into code generation plans
//!
//! This module takes the raw metadata extracted from protobuf files and analyzes it
//! to create a structured plan for code generation. It handles:
//!
//! - Grouping methods by service
//! - Extracting HTTP routing information
//! - Determining parameter types and sources
//! - Planning the structure of generated code
//! - Extracting managed resources from method return types
//!
//! ## Managed Resources
//!
//! Services often manage one or more resource types. These resources are automatically
//! extracted from the return types of get, create, and update methods. For example:
//!
//! ```proto
//! message Catalog {
//!   option (google.api.resource) = {
//!     type: "example.io/Catalog"
//!     pattern: "catalogs/{catalog}"
//!     plural: "catalogs"
//!     singular: "catalog"
//!   };
//!   string name = 1;
//!   // ... other fields
//! }
//!
//! service CatalogService {
//!   rpc GetCatalog(GetCatalogRequest) returns (Catalog);
//!   rpc CreateCatalog(CreateCatalogRequest) returns (Catalog);
//!   rpc UpdateCatalog(UpdateCatalogRequest) returns (Catalog);
//! }
//! ```
//!
//! The analysis will extract that `CatalogService` manages the `Catalog` resource,
//! making this information available for subsequent code generation phases.

use std::collections::{HashMap, HashSet};

use convert_case::{Case, Casing};
use tracing::warn;

use crate::Result;
use crate::google::api::FieldBehavior;
use crate::parsing::types::BaseType;
use crate::parsing::{CodeGenMetadata, HttpPattern, MessageField, MethodMetadata, ServiceInfo};
use crate::utils::strings;

pub(crate) use types::MethodPlanner;
pub use types::{
    BodyField, ChildLink, EmitShape, GenerationPlan, ManagedResource, MethodPlan, MethodShape,
    PathParam, QueryParam, RequestParam, RequestType, ResourceHierarchy, ServicePlan,
    SkippedMethod, extract_managed_resources, split_body_fields,
};
pub(crate) use types::{emit_shape, is_collection_method};

mod types;

/// Analyze collected metadata and create a generation plan.
///
/// Analysis is **protocol-agnostic**: every method gets a [`MethodPlan`]. REST analysis is a
/// superset of what ConnectRPC needs, so a method with a `google.api.http` annotation carries the
/// full REST shape (path/query/body split, URL template) and one without carries the routeless
/// shape (all request fields as body). [`MethodPlan::has_http_route`] distinguishes them: the REST
/// emitters generate only routed methods; the ConnectRPC emitter generates all. No method is
/// dropped here, so [`GenerationPlan::skipped_methods`] is empty.
///
/// Hierarchy derivation uses a two-phase cross-service algorithm:
/// 1. Build all service plans (hierarchy empty).
/// 2. Construct a global parent map across all services, then assign depth-sorted hierarchies.
pub fn analyze_metadata(metadata: &CodeGenMetadata) -> Result<GenerationPlan> {
    // Phase 1: analyze all services without hierarchy.
    //
    // `metadata.services` is a `HashMap`, whose iteration order is non-deterministic. Sort by
    // service name first so the resulting plan order (and thus emitted module declarations and the
    // global parent map's insertion order) is reproducible across runs.
    let mut service_infos: Vec<_> = metadata.services.values().collect();
    service_infos.sort_by(|a, b| a.name.cmp(&b.name));

    let mut plans = Vec::new();
    let mut skipped_methods = Vec::new();
    for service_info in service_infos {
        let (plan, skipped) = analyze_service(metadata, service_info)?;
        plans.push(plan);
        skipped_methods.extend(skipped);
    }

    // Phase 2: build the cross-service global parent map, then assign depth-sorted hierarchies
    // and (once the hierarchy is known) the resource accessor param list.
    let global_map = build_global_parent_map(&plans, metadata);
    let mut services = plans
        .into_iter()
        .map(|mut plan| {
            plan.hierarchy = derive_ordered_hierarchy(&plan, &global_map, metadata)?;
            plan.resource_accessor_params = derive_resource_accessor_params(&plan);
            Ok(plan)
        })
        .collect::<Result<Vec<_>>>()?;

    // Phase 2b: derive direct children. This needs every service's accessor params (filled above),
    // so it runs as a separate sub-pass over the now-complete set.
    assign_direct_children(&mut services);

    Ok(GenerationPlan {
        services,
        skipped_methods,
    })
}

// ── Cross-service hierarchy resolution ────────────────────────────────────────────────────────

/// Maps `(parent_resource_type, child_resource_type)` → proto field name that carries the
/// parent identifier on the child service's List request (e.g. `"catalog_name"`).
type GlobalParentMap = HashMap<(String, String), String>;

/// Build a map of immediate-parent relationships by scanning every service's List method query
/// params for `child_type`-annotated fields.
///
/// **Key semantic filter:** only params where `child_type == this service's managed resource type`
/// are recorded. This ensures that `catalog_name` on `ListTablesRequest` (child_type = Table) is
/// recorded as `(Catalog, Table)`, and `schema_name` as `(Schema, Table)` — but when walking
/// Schema's own List method, `catalog_name` (child_type = Schema) is recorded as `(Catalog, Schema)`.
/// The chain `Catalog → Schema → Table` is then reconstructable via depth analysis.
fn build_global_parent_map(plans: &[ServicePlan], metadata: &CodeGenMetadata) -> GlobalParentMap {
    let mut map: GlobalParentMap = HashMap::new();

    for plan in plans {
        let managed_type = match plan.managed_resources.first() {
            Some(r) => &r.descriptor.r#type,
            None => continue,
        };
        if managed_type.is_empty() {
            continue;
        }

        for method in &plan.methods {
            if method.request_type != RequestType::List {
                continue;
            }
            for param in method.query_parameters() {
                let Some(ref rr) = param.resource_reference else {
                    continue;
                };
                // Only record when child_type matches this service's own managed resource type.
                if rr.child_type != *managed_type {
                    continue;
                }
                if let Some(parent_type) = resolve_parent_type_from_field(&param.name, metadata) {
                    map.entry((parent_type, managed_type.clone()))
                        .or_insert_with(|| param.name.clone());
                }
            }
        }
    }

    map
}

/// Derive the ordered ancestor chain for a service's managed resource using the global parent map.
///
/// Returns entries sorted **root-first** (shallowest ancestor first), so iterating them gives
/// the correct left-to-right param order (e.g. `[catalog_name, schema_name]` for a Table service).
///
/// Returns `Err` if a cycle is detected in the resource hierarchy annotations.
fn derive_ordered_hierarchy(
    plan: &ServicePlan,
    global_map: &GlobalParentMap,
    metadata: &CodeGenMetadata,
) -> Result<Vec<ResourceHierarchy>> {
    let managed_type = match plan.managed_resources.first() {
        Some(r) => r.descriptor.r#type.clone(),
        None => return Ok(vec![]),
    };

    // Collect all (parent_resource_type, parent_field_name) entries for this managed resource.
    let mut ancestors: Vec<(usize, String, String)> = global_map
        .iter()
        .filter(|((_, child), _)| child == &managed_type)
        .map(|((parent_type, _), field_name)| {
            let depth = compute_depth(parent_type, global_map, &mut HashSet::new())?;
            Ok((depth, parent_type.clone(), field_name.clone()))
        })
        .collect::<Result<Vec<_>>>()?;

    if ancestors.is_empty() {
        return Ok(vec![]);
    }

    // Sort root-first (ascending depth).
    ancestors.sort_by_key(|(depth, _, _)| *depth);

    Ok(ancestors
        .into_iter()
        .map(|(_, parent_type, field_name)| {
            let parent_singular = field_name.strip_suffix("_name").and_then(|stem| {
                metadata
                    .resource_from_singular(stem)
                    .map(|_| stem.to_string())
            });
            ResourceHierarchy {
                child_resource_type: managed_type.clone(),
                parent_resource_type: parent_type,
                parent_field_name: field_name,
                parent_singular,
            }
        })
        .collect())
}

/// Compute the depth of a resource type in the parent chain (0 = root, no parent in map).
///
/// Returns `Err` if a cycle is detected in the resource hierarchy annotations.
fn compute_depth(
    resource_type: &str,
    map: &GlobalParentMap,
    visited: &mut HashSet<String>,
) -> Result<usize> {
    if !visited.insert(resource_type.to_string()) {
        return Err(crate::Error::Build(format!(
            "Cycle detected in resource hierarchy at type: {resource_type}"
        )));
    }
    let parent = map
        .iter()
        .find(|((_, child), _)| child == resource_type)
        .map(|((parent, _), _)| parent.clone());

    match parent {
        None => Ok(0),
        Some(parent_type) => Ok(1 + compute_depth(&parent_type, map, visited)?),
    }
}

/// Resolve the resource type string for a proto field name by stripping `"_name"` and looking up
/// the resulting singular in `metadata`.
///
/// Returns `None` if the field name doesn't end in `"_name"` or no resource matches.
fn resolve_parent_type_from_field(field_name: &str, metadata: &CodeGenMetadata) -> Option<String> {
    field_name
        .strip_suffix("_name")
        .and_then(|s| metadata.resource_from_singular(s))
        .map(|rd| rd.r#type.clone())
}

/// Standard AIP-132 `List` request fields that are never part of a resource's name.
///
/// These are pagination/filtering knobs, not parent-hierarchy components. proto3 scalars carry no
/// presence info, so a required `page_token` looks identical to a real parent name like
/// `catalog_name` — the heuristic derivation must exclude these by name explicitly, or pagination
/// fields leak into accessor params (e.g. a flat Catalog getting a bogus `(page_token, name)`).
pub(crate) fn is_standard_list_field(name: &str) -> bool {
    matches!(
        name,
        "page_token" | "page_size" | "max_results" | "order_by" | "filter"
    )
}

/// Derive the ordered ancestor field names for a service's managed resource, *excluding* the
/// resource's own leaf name. e.g. `["catalog_name"]` for Schema, `[]` for a flat Catalog.
///
/// **Annotation-driven path** (preferred): when the service has `hierarchy` entries from
/// `resource_reference { child_type }` annotations, the parent field names come straight from those
/// entries (in List-method param order).
///
/// **Heuristic fallback** (no annotations): the resource name is composite when either
/// 1. the descriptor declares a `name_field` (an explicit dot-joined `full_name`), or
/// 2. the Get method's path param is `"name"` (an opaque full path) *and* the List method has
///    required string query params that aren't standard pagination fields — those are the parents.
///
/// Returns an empty vec for a flat (top-level) resource.
pub(crate) fn derive_accessor_parent_fields(plan: &ServicePlan) -> Vec<String> {
    let Some(resource) = plan.managed_resources.first() else {
        return vec![];
    };

    // --- Annotation-driven path ---
    let resource_type = &resource.descriptor.r#type;
    if !plan.hierarchy.is_empty() && !resource_type.is_empty() {
        let annotation_parents: Vec<String> = plan
            .hierarchy
            .iter()
            .filter(|h| &h.child_resource_type == resource_type)
            .map(|h| h.parent_field_name.clone())
            .collect();
        if !annotation_parents.is_empty() {
            return annotation_parents;
        }
    }

    // --- Heuristic fallback ---
    let has_explicit_name_field = !resource.descriptor.name_field.is_empty();

    let get_path_param_name = plan
        .methods
        .iter()
        .find(|m| m.request_type == RequestType::Get)
        .and_then(|m| m.path_parameters().next().map(|p| p.name.clone()));

    let parent_params: Vec<String> = plan
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

    let should_decompose = has_explicit_name_field
        || (get_path_param_name.as_deref() == Some("name") && !parent_params.is_empty());

    if should_decompose {
        parent_params
    } else {
        vec![]
    }
}

/// Derive the full resource accessor param list (ancestor fields + the resource's own
/// `<singular>_name` leaf), or `None` for resource-less services.
///
/// Single source of truth consumed by all four accessor emitters (Rust aggregate + Python + Node +
/// TypeScript). A returned list of length 1 (`["<singular>_name"]`) is a flat resource; length > 1
/// is nested (its full name is the dot-joined components).
fn derive_resource_accessor_params(plan: &ServicePlan) -> Option<Vec<String>> {
    let resource = plan.managed_resources.first()?;
    let mut params = derive_accessor_parent_fields(plan);
    params.push(format!("{}_name", resource.descriptor.singular));
    Some(params)
}

/// Populate each service's [`ServicePlan::direct_children`] using the accessor-param prefix relation.
///
/// A resource-scoped service C is a *direct child* of resource-scoped service P when C's accessor
/// params extend P's by exactly one trailing component (P's params are a prefix of C's and
/// `C.len() == P.len() + 1`). Children are sorted by singular for deterministic output.
fn assign_direct_children(services: &mut [ServicePlan]) {
    // Snapshot the (accessor params, base_path, singular) of every resource-scoped service so we can
    // scan for children while mutating each parent's `direct_children` in place.
    let candidates: Vec<(Vec<String>, String, String)> = services
        .iter()
        .filter_map(|s| {
            let params = s.resource_accessor_params.clone()?;
            let singular = s.managed_resources.first()?.descriptor.singular.clone();
            Some((params, s.base_path.clone(), singular))
        })
        .collect();

    for plan in services.iter_mut() {
        let Some(parent_params) = plan.resource_accessor_params.clone() else {
            continue;
        };
        let mut children: Vec<ChildLink> = candidates
            .iter()
            .filter(|(child_params, child_base_path, _)| {
                *child_base_path != plan.base_path
                    && child_params.len() == parent_params.len() + 1
                    && child_params[..parent_params.len()] == parent_params[..]
            })
            .map(
                |(child_params, child_base_path, child_singular)| ChildLink {
                    child_singular: child_singular.clone(),
                    child_base_path: child_base_path.clone(),
                    child_accessor_params: child_params.clone(),
                },
            )
            .collect();
        children.sort_by(|a, b| a.child_singular.cmp(&b.child_singular));
        plan.direct_children = children;
    }
}

/// Analyze a single service and create a service plan.
///
/// Returns the plan and a list of methods that were skipped due to incomplete metadata.
fn analyze_service(
    metadata: &CodeGenMetadata,
    info: &ServiceInfo,
) -> Result<(ServicePlan, Vec<SkippedMethod>)> {
    let handler_name = strings::service_to_handler_name(&info.name);
    let base_path = strings::service_to_base_path(&info.name);

    // Analysis is protocol-agnostic: every method gets a plan. Methods without an HTTP annotation
    // get the routeless shape (`has_http_route == false`); the REST emitters skip them while the
    // ConnectRPC emitter uses them. So no method is dropped at analysis time.
    let mut method_plans = Vec::new();
    let skipped = Vec::new();

    for method in &info.methods {
        method_plans.push(analyze_method(metadata, method)?);
    }

    let managed_resources = types::extract_managed_resources(metadata, &method_plans);

    // Second pass: now that we know whether this service manages a resource, classify each method's
    // shape (collection / instance / unbound). `analyze_method` couldn't do this — it has no view of
    // the service's resources.
    let service_has_resource = !managed_resources.is_empty();
    for plan in &mut method_plans {
        plan.shape = types::method_shape(
            &plan.request_type,
            &plan.metadata.method_name,
            service_has_resource,
        );
    }

    Ok((
        ServicePlan {
            service_name: info.name.clone(),
            handler_name,
            base_path,
            package: info.package.clone(),
            methods: method_plans,
            managed_resources,
            documentation: info.documentation.clone(),
            hierarchy: vec![],              // filled in Phase 2 of analyze_metadata
            resource_accessor_params: None, // filled in Phase 2 of analyze_metadata
            direct_children: vec![],        // filled in Phase 2b of analyze_metadata
        },
        skipped,
    ))
}

/// Analyze a single method and create a method plan.
///
/// Analysis is **protocol-agnostic** and produces the superset of information both the REST and the
/// ConnectRPC emitters consume. Every method gets a plan:
///
/// - With a `google.api.http` annotation: the full REST shape (path/query/body split, URL template,
///   request type). [`MethodPlan::has_http_route`] is `true`. The Connect emitter ignores the
///   routing fields and reads only the body fields + input/output types.
/// - Without one: the routeless shape (all request fields as body, no path/query — see
///   [`analyze_routeless_method`]). `has_http_route` is `false`; the REST emitters skip it (it has
///   no route to call), the Connect emitter emits it.
pub(crate) fn analyze_method(
    metadata: &CodeGenMetadata,
    method: &MethodMetadata,
) -> Result<MethodPlan> {
    let Some(http_method) = method.http_method().map(|m| m.to_string()) else {
        // No HTTP annotation: still produce a plan (the routeless / Connect-degenerate shape) so the
        // ConnectRPC emitter can use it. REST emitters gate on `has_http_route`.
        return analyze_routeless_method(metadata, method);
    };

    let planner = MethodPlanner::try_new(method, metadata)?;
    let request_type = planner.request_type();
    let has_response = planner.has_response();
    let output_resource_type = planner.output_resource_type();
    let http_pattern = planner.into_http_pattern();

    let input_fields = metadata.get_message_fields(&method.input_type);
    let (path_params, query_params, body_fields) = extract_request_fields(method, &input_fields)?;

    let parameters = path_params
        .into_iter()
        .map(Into::into)
        .chain(query_params.into_iter().map(Into::into))
        .chain(body_fields.into_iter().map(Into::into))
        .collect();

    let has_request_body = types::request_has_body(&request_type);
    let needs_request_parts = types::request_needs_request_parts(&request_type);
    let scoped_verb = types::scoped_verb(&request_type);

    Ok(MethodPlan {
        metadata: method.clone(),
        handler_function_name: method.method_name.to_case(Case::Snake),
        http_method,
        parameters,
        has_response,
        request_type,
        output_resource_type,
        http_pattern,
        has_request_body,
        needs_request_parts,
        scoped_verb,
        // Provisional: `shape` depends on the owning service's managed resources, which aren't
        // known here. `analyze_service` overwrites this once `extract_managed_resources` has run.
        shape: types::MethodShape::Unbound,
        has_http_route: true,
    })
}

/// Analyze a method that has no `google.api.http` annotation (the ConnectRPC-degenerate shape).
///
/// ConnectRPC dispatch sends the whole request message as the body and reads the whole response, so
/// there is no path/query/body split and no URL template. Every non-`OUTPUT_ONLY` request field
/// becomes a [`BodyField`], partitioned required-vs-optional purely from
/// `google.api.field_behavior` — the same `BodyField` shape the REST builders consume, so the
/// builder layer ([`crate::codegen::builder`]) is reused unchanged.
///
/// The HTTP-only fields of [`MethodPlan`] (`http_method`, `http_pattern`) are left empty/default and
/// [`MethodPlan::has_http_route`] is `false`; the REST emitters skip such methods, so those fields
/// are never read for them.
fn analyze_routeless_method(
    metadata: &CodeGenMetadata,
    method: &MethodMetadata,
) -> Result<MethodPlan> {
    let input_fields = metadata.get_message_fields(&method.input_type);
    let body_fields = routeless_body_fields(&input_fields);

    // A routeless method carries no REST request shape. `Custom` keeps it off the REST collection /
    // instance code paths (resource-scoped clients, pagination heuristics) while still flowing
    // through the shared builder generator. The verb is irrelevant for Connect dispatch; `POST`
    // is the conventional placeholder.
    let request_type =
        RequestType::Custom(crate::google::api::http_rule::Pattern::Post(String::new()));
    let has_response = !method.output_type.is_empty() && !method.output_type.ends_with("Empty");

    Ok(MethodPlan {
        metadata: method.clone(),
        handler_function_name: method.method_name.to_case(Case::Snake),
        http_method: String::new(),
        parameters: body_fields.into_iter().map(Into::into).collect(),
        has_response,
        request_type,
        output_resource_type: None,
        http_pattern: HttpPattern::default(),
        // The request message is always carried as the body for Connect dispatch.
        has_request_body: true,
        needs_request_parts: false,
        scoped_verb: None,
        shape: types::MethodShape::Unbound,
        has_http_route: false,
    })
}

/// Classify a routeless request message's fields into [`BodyField`]s.
///
/// `OUTPUT_ONLY` fields are dropped (server-generated). Oneof fields are optional body fields with
/// their variants preserved. Every other field is a body field whose `required` flag is taken from
/// `google.api.field_behavior` REQUIRED — mirroring the `body: "*"` branch of
/// [`extract_request_fields`], minus the path/query split that ConnectRPC has no notion of.
fn routeless_body_fields(input_fields: &[MessageField]) -> Vec<BodyField> {
    let mut body_fields = Vec::new();
    for field in input_fields {
        if field.field_behavior.contains(&FieldBehavior::OutputOnly) {
            continue;
        }

        if matches!(field.unified_type.base_type, BaseType::OneOf(_)) {
            body_fields.push(BodyField {
                name: field.name.clone(),
                field_type: field.unified_type.clone().optional(),
                repeated: false,
                required: false,
                oneof_variants: field.oneof_variants.clone(),
                documentation: field.documentation.clone(),
            });
            continue;
        }

        let required = field.field_behavior.contains(&FieldBehavior::Required);
        // Match the REST body branch: mark non-required singular message bodies optional on the
        // type so they become `with_*(impl Into<Option<T>>)` setters rather than required args.
        let needs_optional_marker = !required
            && !field.unified_type.is_repeated
            && matches!(
                field.unified_type.base_type,
                BaseType::Message(_) | BaseType::OneOf(_)
            );
        let field_type = if needs_optional_marker {
            field.unified_type.clone().optional()
        } else {
            field.unified_type.clone()
        };
        body_fields.push(BodyField {
            name: field.name.clone(),
            field_type,
            repeated: field.unified_type.is_repeated,
            required,
            oneof_variants: None,
            documentation: field.documentation.clone(),
        });
    }
    body_fields
}

/// Extract and classify request fields from an input message into path, query, and body buckets.
///
/// - Path parameters are matched against URL template parameters and ordered accordingly.
/// - Fields annotated `OUTPUT_ONLY` are excluded entirely — they are server-generated and
///   must not appear in request extractors or client request builders.
/// - Fields matching the `body` spec (`"*"`, `""`, or a specific field name) become body fields.
/// - All remaining fields become query parameters. Fields not explicitly marked optional
///   via `UnifiedType.is_optional` are treated as required query parameters.
/// - Oneof fields are always placed in the body as optional variants.
fn extract_request_fields(
    method: &MethodMetadata,
    input_fields: &[MessageField],
) -> Result<(Vec<PathParam>, Vec<QueryParam>, Vec<BodyField>)> {
    let mut path_params = Vec::new();
    let mut query_params = Vec::new();
    let mut body_fields = Vec::new();

    let path_param_names = method.http_pattern.parameter_names();
    let body_spec = method.http_rule.body.as_str();

    // Build an O(1) lookup map for input fields by name.
    let fields_by_name: HashMap<&str, &MessageField> =
        input_fields.iter().map(|f| (f.name.as_str(), f)).collect();

    let mut processed_fields = HashSet::new();

    // Add path parameters in URL template order. `parameter_names()` yields normalized flat field
    // names from the URL template (segment-binding `=pattern` suffixes already stripped by the
    // template parser; unsupported dotted placeholders are kept out of `parameters` entirely).
    for path_param_name in path_param_names {
        if let Some(field) = fields_by_name.get(path_param_name.as_str()) {
            // Path params are always included even if OUTPUT_ONLY (they appear in the URL, not
            // the request body), but in practice OUTPUT_ONLY fields should never be path params.
            path_params.push(PathParam {
                name: field.name.clone(),
                field_type: field.unified_type.clone(),
                documentation: field.documentation.clone(),
            });
            processed_fields.insert(field.name.as_str());
        } else {
            // Unresolved placeholder: warn rather than silently misrouting the field to query/body.
            warn!(
                "Method {}.{}: URL path placeholder `{}` does not match any request field; \
                 the param will be omitted from the path",
                method.service_name, method.method_name, path_param_name
            );
        }
    }

    // Classify remaining fields as body or query.
    for field in input_fields {
        let field_name = field.name.as_str();

        if processed_fields.contains(field_name) {
            continue;
        }

        // Skip OUTPUT_ONLY fields — they are server-generated and never provided by clients.
        if field.field_behavior.contains(&FieldBehavior::OutputOnly) {
            processed_fields.insert(field_name);
            continue;
        }

        // Oneof fields are always body fields and always optional.
        if matches!(field.unified_type.base_type, BaseType::OneOf(_)) {
            body_fields.push(BodyField {
                name: field.name.clone(),
                field_type: field.unified_type.clone().optional(),
                repeated: false,
                required: false,
                oneof_variants: field.oneof_variants.clone(),
                documentation: field.documentation.clone(),
            });
            processed_fields.insert(field_name);
            continue;
        }

        let is_body = match body_spec {
            "*" => true,
            "" => false,
            specific => specific == field_name,
        };

        if is_body {
            let required = field.field_behavior.contains(&FieldBehavior::Required);
            // Mark non-required singular message/oneof bodies as optional on the type so FFI
            // bindings render them as `Option<T>` (matching their `= None` default). Required
            // bodies keep the bare type and become required constructor params. Scalars and
            // collections are unaffected (their optionality is already correct).
            let needs_optional_marker = !required
                && !field.unified_type.is_repeated
                && matches!(
                    field.unified_type.base_type,
                    BaseType::Message(_) | BaseType::OneOf(_)
                );
            let field_type = if needs_optional_marker {
                field.unified_type.clone().optional()
            } else {
                field.unified_type.clone()
            };
            body_fields.push(BodyField {
                name: field.name.clone(),
                field_type,
                repeated: field.unified_type.is_repeated,
                required,
                oneof_variants: None,
                documentation: field.documentation.clone(),
            });
        } else {
            query_params.push(QueryParam {
                name: field.name.clone(),
                field_type: field.unified_type.clone(),
                documentation: field.documentation.clone(),
                resource_reference: field.resource_reference.clone(),
            });
        }
        processed_fields.insert(field_name);
    }

    Ok((path_params, query_params, body_fields))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google::api::{HttpRule, ResourceDescriptor, http_rule::Pattern};
    use crate::parsing::types::UnifiedType;
    use crate::parsing::{CodeGenMetadata, HttpPattern, MessageInfo, MethodMetadata, ServiceInfo};
    use std::collections::HashMap;

    fn make_metadata_with_catalog() -> CodeGenMetadata {
        let catalog_resource = ResourceDescriptor {
            r#type: "example.io/Catalog".to_string(),
            pattern: vec!["catalogs/{catalog}".to_string()],
            name_field: "name".to_string(),
            history: 0,
            plural: "catalogs".to_string(),
            singular: "catalog".to_string(),
            style: vec![],
        };
        let catalog_info = MessageInfo {
            name: "Catalog".to_string(),
            fields: vec![],
            resource_descriptor: Some(catalog_resource),
            documentation: None,
        };
        let mut messages = HashMap::new();
        messages.insert("Catalog".to_string(), catalog_info);
        CodeGenMetadata {
            messages,
            ..Default::default()
        }
    }

    fn make_get_method() -> MethodMetadata {
        MethodMetadata {
            service_name: "CatalogService".to_string(),
            method_name: "GetCatalog".to_string(),
            input_type: "GetCatalogRequest".to_string(),
            output_type: "Catalog".to_string(),
            operation: None,
            http_rule: HttpRule {
                selector: "".to_string(),
                pattern: Some(Pattern::Get("/catalogs/{name}".to_string())),
                body: "".to_string(),
                response_body: "".to_string(),
                additional_bindings: vec![],
            },
            http_pattern: HttpPattern::parse("/catalogs/{name}"),
            documentation: None,
        }
    }

    #[test]
    fn test_managed_resources_extraction() {
        let metadata = make_metadata_with_catalog();
        let service_info = ServiceInfo {
            name: "CatalogService".to_string(),
            package: "example.catalogs.v1".to_string(),
            documentation: None,
            methods: vec![make_get_method()],
        };
        let (service_plan, skipped) = analyze_service(&metadata, &service_info).unwrap();

        assert!(skipped.is_empty());
        assert_eq!(service_plan.managed_resources.len(), 1);
        assert_eq!(service_plan.managed_resources[0].type_name, "Catalog");
        assert_eq!(
            service_plan.managed_resources[0].descriptor.r#type,
            "example.io/Catalog"
        );
        assert_eq!(
            service_plan.managed_resources[0].descriptor.singular,
            "catalog"
        );
        assert_eq!(
            service_plan.managed_resources[0].descriptor.plural,
            "catalogs"
        );
    }

    #[test]
    fn test_no_duplicate_managed_resources() {
        let metadata = make_metadata_with_catalog();
        let update_method = MethodMetadata {
            service_name: "CatalogService".to_string(),
            method_name: "UpdateCatalog".to_string(),
            input_type: "UpdateCatalogRequest".to_string(),
            output_type: "Catalog".to_string(),
            operation: None,
            http_rule: HttpRule {
                selector: "".to_string(),
                pattern: Some(Pattern::Patch("/catalogs/{name}".to_string())),
                body: "*".to_string(),
                response_body: "".to_string(),
                additional_bindings: vec![],
            },
            http_pattern: HttpPattern::parse("/catalogs/{name}"),
            documentation: None,
        };
        let service_info = ServiceInfo {
            name: "CatalogService".to_string(),
            package: "example.catalogs.v1".to_string(),
            documentation: None,
            methods: vec![make_get_method(), update_method],
        };
        let (service_plan, _skipped) = analyze_service(&metadata, &service_info).unwrap();

        assert_eq!(service_plan.managed_resources.len(), 1);
        assert_eq!(service_plan.managed_resources[0].type_name, "Catalog");
    }

    #[test]
    fn test_analyze_method_missing_http_pattern_is_routeless() {
        let metadata = CodeGenMetadata::default();
        let method = MethodMetadata {
            service_name: "SomeService".to_string(),
            method_name: "SomeMethod".to_string(),
            input_type: "".to_string(),
            output_type: "".to_string(),
            operation: None,
            http_rule: HttpRule {
                selector: "".to_string(),
                pattern: None,
                body: "".to_string(),
                response_body: "".to_string(),
                additional_bindings: vec![],
            },
            http_pattern: HttpPattern::parse(""),
            documentation: None,
        };
        // Protocol-agnostic analysis still produces a plan, but flags it as having no HTTP route so
        // the REST emitters skip it while the Connect emitter uses it.
        let plan = analyze_method(&metadata, &method).unwrap();
        assert!(!plan.has_http_route);
        assert!(plan.http_method.is_empty());
    }

    #[test]
    fn test_routeless_method_partitions_body_by_field_behavior() {
        use crate::parsing::MessageInfo;

        // A request with one REQUIRED field and one unmarked (optional) field, and NO HTTP rule.
        let req = MessageInfo {
            name: "DoThingRequest".to_string(),
            fields: vec![
                {
                    let mut f = make_string_field("name", false);
                    f.field_behavior = vec![FieldBehavior::Required];
                    f
                },
                make_string_field("comment", false),
            ],
            resource_descriptor: None,
            documentation: None,
        };
        let mut messages = HashMap::new();
        messages.insert("DoThingRequest".to_string(), req);
        let metadata = CodeGenMetadata {
            messages,
            ..Default::default()
        };

        let method = MethodMetadata {
            service_name: "ThingService".to_string(),
            method_name: "DoThing".to_string(),
            input_type: "DoThingRequest".to_string(),
            output_type: "DoThingResponse".to_string(),
            operation: None,
            http_rule: HttpRule {
                selector: "".to_string(),
                pattern: None, // no HTTP annotation
                body: "".to_string(),
                response_body: "".to_string(),
                additional_bindings: vec![],
            },
            http_pattern: HttpPattern::default(),
            documentation: None,
        };

        let plan = analyze_method(&metadata, &method).unwrap();
        assert!(!plan.has_http_route);

        // No path/query params: every field is a body field.
        assert_eq!(plan.path_parameters().count(), 0);
        assert_eq!(plan.query_parameters().count(), 0);
        let body: Vec<_> = plan.body_fields().collect();
        assert_eq!(body.len(), 2);

        // Required vs optional follows field_behavior: `name` (REQUIRED) is required, `comment` not.
        let name = body.iter().find(|b| b.name == "name").unwrap();
        let comment = body.iter().find(|b| b.name == "comment").unwrap();
        assert!(name.required, "REQUIRED field should be a constructor arg");
        assert!(
            !comment.required,
            "unmarked field should be an optional with_* setter"
        );
        assert_eq!(plan.handler_function_name, "do_thing");
        assert!(plan.has_response);
    }

    #[test]
    fn test_routeless_method_drops_output_only_fields() {
        use crate::parsing::MessageInfo;

        let req = MessageInfo {
            name: "MakeReq".to_string(),
            fields: vec![make_string_field("input", false), {
                let mut f = make_string_field("server_set", false);
                f.field_behavior = vec![FieldBehavior::OutputOnly];
                f
            }],
            resource_descriptor: None,
            documentation: None,
        };
        let mut messages = HashMap::new();
        messages.insert("MakeReq".to_string(), req);
        let metadata = CodeGenMetadata {
            messages,
            ..Default::default()
        };

        let method = MethodMetadata {
            service_name: "S".to_string(),
            method_name: "Make".to_string(),
            input_type: "MakeReq".to_string(),
            output_type: "MakeResp".to_string(),
            operation: None,
            http_rule: HttpRule::default(),
            http_pattern: HttpPattern::default(),
            documentation: None,
        };

        let plan = analyze_method(&metadata, &method).unwrap();
        let body: Vec<_> = plan.body_fields().collect();
        assert_eq!(body.len(), 1, "OUTPUT_ONLY field must be dropped");
        assert_eq!(body[0].name, "input");
    }

    // --- extract_request_fields unit tests ---

    fn make_string_field(name: &str, optional: bool) -> MessageField {
        use crate::parsing::types::BaseType;
        MessageField {
            name: name.to_string(),
            unified_type: UnifiedType {
                base_type: BaseType::String,
                is_optional: optional,
                is_repeated: false,
            },
            documentation: None,
            oneof_variants: None,
            field_behavior: vec![],
            is_sensitive: false,
            resource_reference: None,
        }
    }

    fn make_repeated_field(name: &str) -> MessageField {
        use crate::parsing::types::BaseType;
        MessageField {
            name: name.to_string(),
            unified_type: UnifiedType {
                base_type: BaseType::String,
                is_optional: false,
                is_repeated: true,
            },
            documentation: None,
            oneof_variants: None,
            field_behavior: vec![],
            is_sensitive: false,
            resource_reference: None,
        }
    }

    fn make_method_with_pattern(pattern: Pattern, body: &str, path: &str) -> MethodMetadata {
        MethodMetadata {
            service_name: "Svc".to_string(),
            method_name: "Method".to_string(),
            input_type: "".to_string(),
            output_type: "".to_string(),
            operation: None,
            http_rule: HttpRule {
                selector: "".to_string(),
                pattern: Some(pattern),
                body: body.to_string(),
                response_body: "".to_string(),
                additional_bindings: vec![],
            },
            http_pattern: HttpPattern::parse(path),
            documentation: None,
        }
    }

    #[test]
    fn test_extract_path_params_in_url_order() {
        let method =
            make_method_with_pattern(Pattern::Get("/a/{x}/b/{y}".to_string()), "", "/a/{x}/b/{y}");
        let fields = vec![make_string_field("y", false), make_string_field("x", false)];
        let (path, query, body) = extract_request_fields(&method, &fields).unwrap();
        // Path params should be in URL order: x, y — not field declaration order
        assert_eq!(path.len(), 2);
        assert_eq!(path[0].name, "x");
        assert_eq!(path[1].name, "y");
        assert!(query.is_empty());
        assert!(body.is_empty());
    }

    #[test]
    fn test_extract_body_wildcard() {
        let method = make_method_with_pattern(Pattern::Post("/items".to_string()), "*", "/items");
        let fields = vec![
            make_string_field("name", false),
            make_string_field("description", true),
        ];
        let (path, query, body) = extract_request_fields(&method, &fields).unwrap();
        assert!(path.is_empty());
        assert!(query.is_empty());
        assert_eq!(body.len(), 2);
    }

    #[test]
    fn test_extract_specific_body_field() {
        let method = make_method_with_pattern(
            Pattern::Patch("/items/{name}".to_string()),
            "payload",
            "/items/{name}",
        );
        let fields = vec![
            make_string_field("name", false),    // path
            make_string_field("payload", false), // body (specific)
            make_string_field("extra", true),    // query
        ];
        let (path, query, body) = extract_request_fields(&method, &fields).unwrap();
        assert_eq!(path.len(), 1);
        assert_eq!(path[0].name, "name");
        assert_eq!(body.len(), 1);
        assert_eq!(body[0].name, "payload");
        assert_eq!(query.len(), 1);
        assert_eq!(query[0].name, "extra");
    }

    #[test]
    fn test_extract_no_body_spec_all_query() {
        let method = make_method_with_pattern(Pattern::Get("/items".to_string()), "", "/items");
        let fields = vec![
            make_string_field("filter", true),
            make_string_field("page_size", true),
        ];
        let (path, query, body) = extract_request_fields(&method, &fields).unwrap();
        assert!(path.is_empty());
        assert_eq!(query.len(), 2);
        assert!(body.is_empty());
    }

    #[test]
    fn test_extract_repeated_field_becomes_body_with_repeated_flag() {
        let method = make_method_with_pattern(Pattern::Post("/items".to_string()), "*", "/items");
        let fields = vec![make_repeated_field("tags")];
        let (_, _, body) = extract_request_fields(&method, &fields).unwrap();
        assert_eq!(body.len(), 1);
        assert!(body[0].repeated);
    }

    // ── Cross-service hierarchy chain resolution tests ─────────────────────────────────────────

    /// Build a three-level metadata fixture: Catalog → Schema → Table.
    ///
    /// Three services:
    /// - CatalogService: ListCatalogs (no child_type params), GetCatalog
    /// - SchemaService: ListSchemas (catalog_name with child_type = Schema), GetSchema
    /// - TableService: ListTables (catalog_name and schema_name both with child_type = Table), GetTable
    fn make_three_level_metadata() -> (CodeGenMetadata, ServiceInfo, ServiceInfo, ServiceInfo) {
        use crate::google::api::ResourceReference;
        use crate::parsing::types::BaseType;

        let mut messages = HashMap::new();

        // Catalog resource
        messages.insert(
            "Catalog".to_string(),
            MessageInfo {
                name: "Catalog".to_string(),
                fields: vec![],
                resource_descriptor: Some(ResourceDescriptor {
                    r#type: "example.io/Catalog".to_string(),
                    pattern: vec!["catalogs/{catalog}".to_string()],
                    name_field: "name".to_string(),
                    history: 0,
                    plural: "catalogs".to_string(),
                    singular: "catalog".to_string(),
                    style: vec![],
                }),
                documentation: None,
            },
        );

        // Schema resource
        messages.insert(
            "Schema".to_string(),
            MessageInfo {
                name: "Schema".to_string(),
                fields: vec![],
                resource_descriptor: Some(ResourceDescriptor {
                    r#type: "example.io/Schema".to_string(),
                    pattern: vec!["schemas/{schema}".to_string()],
                    name_field: "name".to_string(),
                    history: 0,
                    plural: "schemas".to_string(),
                    singular: "schema".to_string(),
                    style: vec![],
                }),
                documentation: None,
            },
        );

        // Table resource
        messages.insert(
            "Table".to_string(),
            MessageInfo {
                name: "Table".to_string(),
                fields: vec![],
                resource_descriptor: Some(ResourceDescriptor {
                    r#type: "example.io/Table".to_string(),
                    pattern: vec!["tables/{table}".to_string()],
                    name_field: "full_name".to_string(),
                    history: 0,
                    plural: "tables".to_string(),
                    singular: "table".to_string(),
                    style: vec![],
                }),
                documentation: None,
            },
        );

        // ListCatalogsRequest — no child_type params
        messages.insert(
            "ListCatalogsRequest".to_string(),
            MessageInfo {
                name: "ListCatalogsRequest".to_string(),
                fields: vec![],
                resource_descriptor: None,
                documentation: None,
            },
        );

        // ListSchemasRequest — catalog_name with child_type = Schema
        messages.insert(
            "ListSchemasRequest".to_string(),
            MessageInfo {
                name: "ListSchemasRequest".to_string(),
                fields: vec![MessageField {
                    name: "catalog_name".to_string(),
                    unified_type: UnifiedType {
                        base_type: BaseType::String,
                        is_optional: false,
                        is_repeated: false,
                    },
                    documentation: None,
                    oneof_variants: None,
                    field_behavior: vec![crate::google::api::FieldBehavior::Required],
                    is_sensitive: false,
                    resource_reference: Some(ResourceReference {
                        r#type: String::new(),
                        child_type: "example.io/Schema".to_string(),
                    }),
                }],
                resource_descriptor: None,
                documentation: None,
            },
        );

        // ListTablesRequest — catalog_name and schema_name, both child_type = Table (flat API)
        messages.insert(
            "ListTablesRequest".to_string(),
            MessageInfo {
                name: "ListTablesRequest".to_string(),
                fields: vec![
                    MessageField {
                        name: "catalog_name".to_string(),
                        unified_type: UnifiedType {
                            base_type: BaseType::String,
                            is_optional: false,
                            is_repeated: false,
                        },
                        documentation: None,
                        oneof_variants: None,
                        field_behavior: vec![crate::google::api::FieldBehavior::Required],
                        is_sensitive: false,
                        resource_reference: Some(ResourceReference {
                            r#type: String::new(),
                            child_type: "example.io/Table".to_string(),
                        }),
                    },
                    MessageField {
                        name: "schema_name".to_string(),
                        unified_type: UnifiedType {
                            base_type: BaseType::String,
                            is_optional: false,
                            is_repeated: false,
                        },
                        documentation: None,
                        oneof_variants: None,
                        field_behavior: vec![crate::google::api::FieldBehavior::Required],
                        is_sensitive: false,
                        resource_reference: Some(ResourceReference {
                            r#type: String::new(),
                            child_type: "example.io/Table".to_string(),
                        }),
                    },
                ],
                resource_descriptor: None,
                documentation: None,
            },
        );

        let metadata = CodeGenMetadata {
            messages,
            ..Default::default()
        };

        let catalog_svc = ServiceInfo {
            name: "CatalogService".to_string(),
            package: "example.v1".to_string(),
            documentation: None,
            methods: vec![
                MethodMetadata {
                    service_name: "CatalogService".to_string(),
                    method_name: "ListCatalogs".to_string(),
                    input_type: "ListCatalogsRequest".to_string(),
                    output_type: "ListCatalogsResponse".to_string(),
                    operation: None,
                    http_rule: HttpRule {
                        selector: "".to_string(),
                        pattern: Some(Pattern::Get("/catalogs".to_string())),
                        body: "".to_string(),
                        response_body: "".to_string(),
                        additional_bindings: vec![],
                    },
                    http_pattern: HttpPattern::parse("/catalogs"),
                    documentation: None,
                },
                MethodMetadata {
                    service_name: "CatalogService".to_string(),
                    method_name: "GetCatalog".to_string(),
                    input_type: "GetCatalogRequest".to_string(),
                    output_type: "Catalog".to_string(),
                    operation: None,
                    http_rule: HttpRule {
                        selector: "".to_string(),
                        pattern: Some(Pattern::Get("/catalogs/{name}".to_string())),
                        body: "".to_string(),
                        response_body: "".to_string(),
                        additional_bindings: vec![],
                    },
                    http_pattern: HttpPattern::parse("/catalogs/{name}"),
                    documentation: None,
                },
            ],
        };

        let schema_svc = ServiceInfo {
            name: "SchemaService".to_string(),
            package: "example.v1".to_string(),
            documentation: None,
            methods: vec![
                MethodMetadata {
                    service_name: "SchemaService".to_string(),
                    method_name: "ListSchemas".to_string(),
                    input_type: "ListSchemasRequest".to_string(),
                    output_type: "ListSchemasResponse".to_string(),
                    operation: None,
                    http_rule: HttpRule {
                        selector: "".to_string(),
                        pattern: Some(Pattern::Get("/schemas".to_string())),
                        body: "".to_string(),
                        response_body: "".to_string(),
                        additional_bindings: vec![],
                    },
                    http_pattern: HttpPattern::parse("/schemas"),
                    documentation: None,
                },
                MethodMetadata {
                    service_name: "SchemaService".to_string(),
                    method_name: "GetSchema".to_string(),
                    input_type: "GetSchemaRequest".to_string(),
                    output_type: "Schema".to_string(),
                    operation: None,
                    http_rule: HttpRule {
                        selector: "".to_string(),
                        pattern: Some(Pattern::Get("/schemas/{full_name}".to_string())),
                        body: "".to_string(),
                        response_body: "".to_string(),
                        additional_bindings: vec![],
                    },
                    http_pattern: HttpPattern::parse("/schemas/{full_name}"),
                    documentation: None,
                },
            ],
        };

        let table_svc = ServiceInfo {
            name: "TableService".to_string(),
            package: "example.v1".to_string(),
            documentation: None,
            methods: vec![
                MethodMetadata {
                    service_name: "TableService".to_string(),
                    method_name: "ListTables".to_string(),
                    input_type: "ListTablesRequest".to_string(),
                    output_type: "ListTablesResponse".to_string(),
                    operation: None,
                    http_rule: HttpRule {
                        selector: "".to_string(),
                        pattern: Some(Pattern::Get("/tables".to_string())),
                        body: "".to_string(),
                        response_body: "".to_string(),
                        additional_bindings: vec![],
                    },
                    http_pattern: HttpPattern::parse("/tables"),
                    documentation: None,
                },
                MethodMetadata {
                    service_name: "TableService".to_string(),
                    method_name: "GetTable".to_string(),
                    input_type: "GetTableRequest".to_string(),
                    output_type: "Table".to_string(),
                    operation: None,
                    http_rule: HttpRule {
                        selector: "".to_string(),
                        pattern: Some(Pattern::Get("/tables/{full_name}".to_string())),
                        body: "".to_string(),
                        response_body: "".to_string(),
                        additional_bindings: vec![],
                    },
                    http_pattern: HttpPattern::parse("/tables/{full_name}"),
                    documentation: None,
                },
            ],
        };

        (metadata, catalog_svc, schema_svc, table_svc)
    }

    /// Build service plans from three-level fixture metadata for hierarchy testing.
    fn make_plans_from_fixture() -> (Vec<ServicePlan>, CodeGenMetadata) {
        let (metadata, catalog_svc, schema_svc, table_svc) = make_three_level_metadata();
        let mut plans = Vec::new();
        for svc in &[&catalog_svc, &schema_svc, &table_svc] {
            let (plan, _) = analyze_service(&metadata, svc).unwrap();
            plans.push(plan);
        }
        (plans, metadata)
    }

    #[test]
    fn test_build_global_parent_map() {
        let (plans, metadata) = make_plans_from_fixture();
        let map = build_global_parent_map(&plans, &metadata);

        // Catalog → Schema (from ListSchemasRequest.catalog_name with child_type=Schema)
        assert_eq!(
            map.get(&(
                "example.io/Catalog".to_string(),
                "example.io/Schema".to_string()
            )),
            Some(&"catalog_name".to_string()),
            "Catalog→Schema mapping missing"
        );
        // Schema → Table (from ListTablesRequest.schema_name with child_type=Table)
        assert_eq!(
            map.get(&(
                "example.io/Schema".to_string(),
                "example.io/Table".to_string()
            )),
            Some(&"schema_name".to_string()),
            "Schema→Table mapping missing"
        );
        // Catalog → Table (flat-API artifact from ListTablesRequest.catalog_name)
        assert_eq!(
            map.get(&(
                "example.io/Catalog".to_string(),
                "example.io/Table".to_string()
            )),
            Some(&"catalog_name".to_string()),
            "Catalog→Table flat-API mapping missing"
        );
    }

    #[test]
    fn test_derive_ordered_hierarchy_three_levels() {
        let (plans, metadata) = make_plans_from_fixture();
        let map = build_global_parent_map(&plans, &metadata);
        let table_plan = plans
            .iter()
            .find(|p| p.service_name == "TableService")
            .unwrap();
        let hierarchy = derive_ordered_hierarchy(table_plan, &map, &metadata)
            .expect("no cycles in test fixture");

        assert_eq!(hierarchy.len(), 2, "expected 2 ancestors for Table");
        // Root-first: catalog_name (depth 0) before schema_name (depth 1)
        assert_eq!(hierarchy[0].parent_field_name, "catalog_name");
        assert_eq!(hierarchy[1].parent_field_name, "schema_name");
        assert_eq!(hierarchy[0].parent_resource_type, "example.io/Catalog");
        assert_eq!(hierarchy[1].parent_resource_type, "example.io/Schema");
        assert_eq!(hierarchy[0].parent_singular, Some("catalog".to_string()));
        assert_eq!(hierarchy[1].parent_singular, Some("schema".to_string()));
        // child_resource_type is the managed resource (Table) for all entries
        assert!(
            hierarchy
                .iter()
                .all(|h| h.child_resource_type == "example.io/Table")
        );
    }

    #[test]
    fn test_derive_ordered_hierarchy_two_levels() {
        let (plans, metadata) = make_plans_from_fixture();
        let map = build_global_parent_map(&plans, &metadata);
        let schema_plan = plans
            .iter()
            .find(|p| p.service_name == "SchemaService")
            .unwrap();
        let hierarchy = derive_ordered_hierarchy(schema_plan, &map, &metadata)
            .expect("no cycles in test fixture");

        assert_eq!(hierarchy.len(), 1);
        assert_eq!(hierarchy[0].parent_field_name, "catalog_name");
        assert_eq!(hierarchy[0].parent_resource_type, "example.io/Catalog");
    }

    #[test]
    fn test_derive_ordered_hierarchy_root_resource() {
        let (plans, metadata) = make_plans_from_fixture();
        let map = build_global_parent_map(&plans, &metadata);
        let catalog_plan = plans
            .iter()
            .find(|p| p.service_name == "CatalogService")
            .unwrap();
        let hierarchy = derive_ordered_hierarchy(catalog_plan, &map, &metadata)
            .expect("no cycles in test fixture");

        assert!(
            hierarchy.is_empty(),
            "root resource should have empty hierarchy"
        );
    }

    #[test]
    fn test_compute_depth_cycle_guard() {
        // A ↔ B mutual cycle: compute_depth should return Err, not panic or recurse infinitely.
        let mut map: GlobalParentMap = HashMap::new();
        map.insert(("A".to_string(), "B".to_string()), "a_name".to_string());
        map.insert(("B".to_string(), "A".to_string()), "b_name".to_string());

        assert!(compute_depth("A", &map, &mut HashSet::new()).is_err());
        assert!(compute_depth("B", &map, &mut HashSet::new()).is_err());
    }

    #[test]
    fn test_full_analyze_metadata_hierarchy_ordering() {
        let (metadata, catalog_svc, schema_svc, table_svc) = make_three_level_metadata();
        let mut services_map = HashMap::new();
        services_map.insert("CatalogService".to_string(), catalog_svc);
        services_map.insert("SchemaService".to_string(), schema_svc);
        services_map.insert("TableService".to_string(), table_svc);
        let full_metadata = CodeGenMetadata {
            messages: metadata.messages,
            services: services_map,
            ..Default::default()
        };

        let plan = analyze_metadata(&full_metadata).unwrap();
        let table_svc_plan = plan
            .services
            .iter()
            .find(|s| s.service_name == "TableService")
            .expect("TableService plan not found");

        assert_eq!(table_svc_plan.hierarchy.len(), 2);
        assert_eq!(
            table_svc_plan.hierarchy[0].parent_field_name,
            "catalog_name"
        );
        assert_eq!(table_svc_plan.hierarchy[1].parent_field_name, "schema_name");
    }

    #[test]
    fn resource_accessor_params_nested_and_flat() {
        let (metadata, catalog_svc, schema_svc, table_svc) = make_three_level_metadata();
        let mut services_map = HashMap::new();
        services_map.insert("CatalogService".to_string(), catalog_svc);
        services_map.insert("SchemaService".to_string(), schema_svc);
        services_map.insert("TableService".to_string(), table_svc);
        let full_metadata = CodeGenMetadata {
            messages: metadata.messages,
            services: services_map,
            ..Default::default()
        };

        let plan = analyze_metadata(&full_metadata).unwrap();
        let params = |name: &str| {
            plan.services
                .iter()
                .find(|s| s.service_name == name)
                .unwrap()
                .resource_accessor_params
                .clone()
        };

        // Nested resource (annotation-driven): ancestors root-first + own leaf.
        assert_eq!(
            params("TableService"),
            Some(vec![
                "catalog_name".to_string(),
                "schema_name".to_string(),
                "table_name".to_string(),
            ])
        );
        // One ancestor.
        assert_eq!(
            params("SchemaService"),
            Some(vec!["catalog_name".to_string(), "schema_name".to_string()])
        );
        // Flat top-level resource: just its own name.
        assert_eq!(
            params("CatalogService"),
            Some(vec!["catalog_name".to_string()])
        );
    }

    #[test]
    fn direct_children_via_accessor_param_prefix() {
        let (metadata, catalog_svc, schema_svc, table_svc) = make_three_level_metadata();
        let mut services_map = HashMap::new();
        services_map.insert("CatalogService".to_string(), catalog_svc);
        services_map.insert("SchemaService".to_string(), schema_svc);
        services_map.insert("TableService".to_string(), table_svc);
        let full_metadata = CodeGenMetadata {
            messages: metadata.messages,
            services: services_map,
            ..Default::default()
        };

        let plan = analyze_metadata(&full_metadata).unwrap();
        let children = |name: &str| -> Vec<String> {
            plan.services
                .iter()
                .find(|s| s.service_name == name)
                .unwrap()
                .direct_children
                .iter()
                .map(|c| c.child_singular.clone())
                .collect()
        };

        // Catalog → Schema (params extend by one); Catalog is NOT a direct parent of Table (extends
        // by two), so Table must not appear under Catalog.
        assert_eq!(children("CatalogService"), vec!["schema".to_string()]);
        // Schema → Table.
        assert_eq!(children("SchemaService"), vec!["table".to_string()]);
        // Table is a leaf: no children.
        assert!(children("TableService").is_empty());

        // The child link carries enough to construct the child clients.
        let catalog = plan
            .services
            .iter()
            .find(|s| s.service_name == "CatalogService")
            .unwrap();
        let schema_link = &catalog.direct_children[0];
        // base_path comes from the child service name (here `SchemaService` → `schema`).
        let schema_base_path = plan
            .services
            .iter()
            .find(|s| s.service_name == "SchemaService")
            .unwrap()
            .base_path
            .clone();
        assert_eq!(schema_link.child_base_path, schema_base_path);
        assert_eq!(
            schema_link.child_accessor_params,
            vec!["catalog_name".to_string(), "schema_name".to_string()]
        );
    }

    // The pagination-leak regression (a flat resource's List `page_token`/`max_results` no longer
    // bleed into its accessor params) is covered end-to-end by the golden snapshot test
    // (`tests/golden_integration.rs`), whose `example.bin` ListCatalogs request carries those very
    // fields. `is_standard_list_field` is unit-tested below.

    #[test]
    fn standard_list_fields_recognized() {
        for f in [
            "page_token",
            "page_size",
            "max_results",
            "order_by",
            "filter",
        ] {
            assert!(
                is_standard_list_field(f),
                "{f} should be a standard list field"
            );
        }
        for f in ["name", "catalog_name", "schema_name"] {
            assert!(
                !is_standard_list_field(f),
                "{f} should NOT be a standard list field"
            );
        }
    }
}
