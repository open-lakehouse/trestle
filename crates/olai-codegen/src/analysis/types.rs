use std::collections::HashSet;

use convert_case::{Case, Casing};
use quote::format_ident;
use syn::Ident;

use crate::error::{Error, Result};
use crate::google::api::{ResourceDescriptor, http_rule::Pattern};
use crate::parsing::types::UnifiedType;
use crate::parsing::{CodeGenMetadata, HttpPattern, MethodMetadata, OneofVariant};

/// The Operation a method is performing
///
/// There are standard CRUD operations, as well as custom operations.
///
/// Standard operations on collections are:
/// - List: Retrieve a list of resources
/// - Create: Create a new resource
///
/// Standard operations on individual resources are:
/// - Get: Retrieve a single resource
/// - Update: Update an existing resource
/// - Delete: Delete a resource
///
/// Custom operations are:
/// - Custom(Pattern): custom HTTP operation
#[derive(Debug, Clone, PartialEq)]
pub enum RequestType {
    List,
    Create,
    Get,
    Update,
    Delete,
    Custom(Pattern),
}

/// Where a method sits relative to a resource, independent of its CRUD verb.
///
/// Computed once during analysis from the method's [`RequestType`], name, and whether the owning
/// service manages a `google.api.resource`. This makes the scoped/flat and collection/instance
/// decisions explicit on the IR so emitters stop re-deriving them (replacing the old
/// `MethodHandler::is_collection_method()` + `BindingMode` logic scattered across emit sites).
///
/// The three cases map onto the binding layout:
/// - [`MethodShape::CollectionScoped`] and [`MethodShape::InstanceScoped`] both belong to a
///   resource-scoped service; collection methods live on the root client, instance methods on the
///   per-resource scoped client.
/// - [`MethodShape::Unbound`] belongs to a resource-less service; every method lives on the root
///   client and passes all params (the old "flat" lowering).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodShape {
    /// Collection-style operation on a resource service (list / create / factory RPC). Lives on the
    /// root/aggregate client and passes every param.
    CollectionScoped,
    /// Instance operation on a resource service (get / update / delete / resource-targeted custom).
    /// Lives on the per-resource scoped client, which already holds the path params.
    InstanceScoped,
    /// Method of a resource-less ("flat") service. Lives on the root client and passes every param,
    /// including path params.
    Unbound,
}

/// The structural template a language binding uses to emit a method, independent of language.
///
/// Collapses the per-binding `match request_type { … }` dispatch (which Python, Node, and
/// TypeScript each repeated identically) into a single analysis decision. Each variant corresponds
/// to one emit-shape helper every binding already has (`*_list_method_impl`,
/// `*_create_method_impl`, `*_get_update_method_impl`, `*_delete_method_impl`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmitShape {
    /// Streamed/collected list — returns the repeated item type. (`List`, or a custom `GET` named
    /// `List*`.)
    List,
    /// Factory/create — body in, returns a (possibly unrelated) response message. (`Create`, or a
    /// collection-style custom `POST`/`PATCH` factory like `Generate*`.)
    Create,
    /// Get-or-update — path params (+ optional body), returns the resource/response. (`Get`,
    /// `Update`, a resource-targeted custom `POST`/`PATCH`, or any other custom verb on a flat
    /// service.)
    GetUpdate,
    /// Delete — path params, no response. (`Delete`.)
    Delete,
}

/// Classify a method's [`EmitShape`] from its request type and proto name.
///
/// Mirrors **exactly** the per-impl dispatch the bindings previously each performed inline (the
/// `match request_type` arms in `collection_client_method` / `flat_client_method` /
/// `resource_client_method`), so emit shape is decided in one place without behavior drift.
///
/// `is_collection` (from [`is_collection_method`]) only distinguishes a collection-style custom
/// `POST`/`PATCH` factory (→ [`EmitShape::Create`]) from a resource-targeted custom `POST`/`PATCH`
/// (→ [`EmitShape::GetUpdate`]). Note a resource-less `Custom(Get)` named `List*` is **not**
/// List-shaped here: the old flat dispatch routed it through the `Custom(_) => get_update` arm
/// (returning the full response message, keeping `page_token`), and only a true
/// [`RequestType::List`] gets streamed/`List` emit. Collection *routing* (which client a method
/// lands on) is a separate decision driven by [`is_collection_method`].
pub(crate) fn emit_shape(request_type: &RequestType, is_collection: bool) -> EmitShape {
    match request_type {
        RequestType::List => EmitShape::List,
        RequestType::Create => EmitShape::Create,
        RequestType::Delete => EmitShape::Delete,
        RequestType::Get | RequestType::Update => EmitShape::GetUpdate,
        // A collection-style custom POST/PATCH is a factory (Create-shaped); every other custom verb
        // — including a `Custom(Get)` named `List*` — is GetUpdate-shaped (path + optional body,
        // return the full response).
        RequestType::Custom(Pattern::Post(_) | Pattern::Patch(_)) if is_collection => {
            EmitShape::Create
        }
        RequestType::Custom(_) => EmitShape::GetUpdate,
    }
}

/// A method that was skipped during analysis due to incomplete metadata.
#[derive(Debug, Clone)]
pub struct SkippedMethod {
    /// Fully-qualified name of the service containing the skipped method.
    pub service_name: String,
    /// Name of the skipped method (e.g. `"GetCatalog"`).
    pub method_name: String,
    /// Human-readable reason the method was skipped (e.g. `"missing HTTP annotation"`).
    pub reason: String,
}

/// High-level plan for what code to generate
#[derive(Debug)]
pub struct GenerationPlan {
    /// Services to generate handlers for
    pub services: Vec<ServicePlan>,
    /// Methods that were excluded from the plan due to incomplete metadata.
    ///
    /// Callers can inspect this list to distinguish "service has zero methods" from "all methods
    /// were skipped due to missing HTTP annotations", and to surface actionable warnings.
    pub skipped_methods: Vec<SkippedMethod>,
}

/// Plan for generating code for a single service
#[derive(Debug, Clone)]
pub struct ServicePlan {
    /// Service name (e.g., "CatalogsService")
    pub service_name: String,
    /// Handler trait name (e.g., "CatalogHandler")
    pub handler_name: String,
    /// Base URL path for this service (e.g., "catalogs")
    pub base_path: String,
    /// Proto package name (e.g., "unitycatalog.catalogs.v1")
    pub package: String,
    /// Methods to generate for this service
    pub methods: Vec<MethodPlan>,
    /// Resources managed by this service
    pub managed_resources: Vec<ManagedResource>,
    /// Documentation from protobuf service comments
    pub documentation: Option<String>,
    /// Ancestor chain for this service's managed resource, derived cross-service from
    /// `resource_reference { child_type }` annotations.
    ///
    /// Entries are ordered **root-first** (shallowest ancestor first). For example, for
    /// a Table service this would be `[catalog_name (depth 0), schema_name (depth 1)]`.
    ///
    /// Empty when no `resource_reference` annotations are present — codegen falls back to
    /// naming heuristics in that case.
    pub hierarchy: Vec<ResourceHierarchy>,
    /// Parameter names for this service's resource accessor on the aggregate client, e.g.
    /// `["catalog_name", "schema_name"]` for a nested Schema or `["name"]` for a flat Catalog.
    ///
    /// The trailing element is the resource's own `<singular>_name`; any preceding elements are
    /// ancestor identifiers. A list of length > 1 means the resource is *nested* and its full name
    /// is the dot-joined components.
    ///
    /// `None` for resource-less services (no accessor is generated). Computed once during analysis
    /// (Phase 2, after `hierarchy` is populated) so the four accessor emitters — Rust aggregate,
    /// Python, Node, TypeScript — all read the same list instead of each re-deriving it.
    pub resource_accessor_params: Option<Vec<String>>,
    /// Direct child resources of this service's managed resource, used to generate child-navigation
    /// accessors (e.g. `catalog.schema(name)`) and child-create methods (e.g. `catalog.create_schema`)
    /// on the scoped client.
    ///
    /// A resource C is a *direct child* of P when C's [`Self::resource_accessor_params`] equals P's
    /// plus exactly one additional trailing component (P's params are a prefix of C's, and
    /// `C.len() == P.len() + 1`). Sorted by `child_singular`. Empty for leaf or flat resources.
    /// Computed in Phase 2 of `analyze_metadata` after every service's accessor params are known.
    pub direct_children: Vec<ChildLink>,
}

/// A direct child resource of a parent resource, for generating child-navigation and child-create
/// methods on the parent's scoped client. See [`ServicePlan::direct_children`].
#[derive(Debug, Clone)]
pub struct ChildLink {
    /// The child resource singular — used as the navigation accessor method name (e.g. `schema`).
    pub child_singular: String,
    /// The child service's `base_path` / module segment (e.g. `schemas`), used to reference the
    /// child's generated clients as `crate::codegen::<base_path>::…`.
    pub child_base_path: String,
    /// The child's full accessor params (ancestors + own leaf), e.g. `["catalog_name","schema_name"]`.
    /// The leading entries equal the parent's accessor params (the prefix relation); the final entry
    /// is the child's own name component.
    pub child_accessor_params: Vec<String>,
}

/// Plan for generating code for a single method
#[derive(Debug, Clone)]
pub struct MethodPlan {
    /// Original method metadata
    pub metadata: MethodMetadata,
    /// Rust function name for the handler method
    pub handler_function_name: String,
    /// Pre-parsed HTTP URL pattern
    pub http_pattern: HttpPattern,
    /// HTTP method string for routing (e.g., "GET", "POST")
    pub http_method: String,
    /// Parameters passed to the method (path, query, and body)
    pub parameters: Vec<RequestParam>,
    /// Whether this method returns a response body
    pub has_response: bool,
    /// Request type for this method
    pub request_type: RequestType,
    /// The resource type name returned by this method (if any)
    pub output_resource_type: Option<String>,
    /// Whether the HTTP request carries a JSON body (create / update / custom POST·PATCH).
    ///
    /// Derived once from [`RequestType`] so the client emitter doesn't re-match the enum to
    /// decide whether to attach `.json(request)`.
    pub has_request_body: bool,
    /// Whether the Axum server extractor reads from request *parts* (path/query only) rather
    /// than the body — i.e. this is a `FromRequestParts` method, not `FromRequest`.
    ///
    /// Derived once from [`RequestType`] so the server emitter (extractor selection and the
    /// `RequestPartsExt` import) doesn't re-match the enum.
    pub needs_request_parts: bool,
    /// The verb name to call on a resource-scoped client (`get` / `update` / `delete`), or
    /// `None` for methods that are not standard instance verbs.
    ///
    /// Derived once from [`RequestType`]; the scoped binding call uses this instead of
    /// re-deciding the verb from the enum (see `MethodPlan::resource_client_method`).
    pub scoped_verb: Option<String>,
    /// Where this method sits relative to a resource (collection / instance / unbound).
    ///
    /// Assigned in `analyze_service` once the owning service's managed resources are known, so it
    /// reflects both the method's verb/name shape and whether the service is resource-scoped.
    pub shape: MethodShape,
    /// Whether this method has a `google.api.http` route.
    ///
    /// Analysis is protocol-agnostic: every method gets a plan. A method without an HTTP
    /// annotation still produces a plan (with all request fields as body, no path/query) so the
    /// ConnectRPC emitter can use it — but it has no REST route, so the REST client/handler
    /// emitters skip it. `true` for methods with an HTTP annotation, `false` otherwise.
    pub has_http_route: bool,
}

impl MethodPlan {
    pub fn path_parameters(&self) -> impl Iterator<Item = &PathParam> {
        self.parameters.iter().filter_map(|param| match param {
            RequestParam::Path(path_param) => Some(path_param),
            _ => None,
        })
    }

    pub fn query_parameters(&self) -> impl Iterator<Item = &QueryParam> {
        self.parameters.iter().filter_map(|param| match param {
            RequestParam::Query(query_param) => Some(query_param),
            _ => None,
        })
    }

    pub fn body_fields(&self) -> impl Iterator<Item = &BodyField> {
        self.parameters.iter().filter_map(|param| match param {
            RequestParam::Body(body_field) => Some(body_field),
            _ => None,
        })
    }
}

#[derive(Debug, Clone)]
pub enum RequestParam {
    Path(PathParam),
    Query(QueryParam),
    Body(BodyField),
}

impl RequestParam {
    pub fn name(&self) -> &str {
        match self {
            RequestParam::Path(param) => &param.name,
            RequestParam::Query(param) => &param.name,
            RequestParam::Body(param) => &param.name,
        }
    }

    pub fn field_type(&self) -> &UnifiedType {
        match self {
            RequestParam::Path(param) => &param.field_type,
            RequestParam::Query(param) => &param.field_type,
            RequestParam::Body(param) => &param.field_type,
        }
    }

    pub fn field_ident(&self) -> Ident {
        format_ident!("{}", self.name())
    }

    pub fn is_optional(&self) -> bool {
        match self {
            RequestParam::Path(_) => false,
            RequestParam::Query(param) => param.is_optional(),
            RequestParam::Body(param) => param.is_optional(),
        }
    }

    pub fn is_path_param(&self) -> bool {
        matches!(self, RequestParam::Path(_))
    }

    pub fn documentation(&self) -> Option<&str> {
        match self {
            RequestParam::Path(param) => param.documentation.as_deref(),
            RequestParam::Query(param) => param.documentation.as_deref(),
            RequestParam::Body(param) => param.documentation.as_deref(),
        }
    }
}

/// A path parameter in a URL template
#[derive(Debug, Clone)]
pub struct PathParam {
    /// Field name in the request struct (e.g., "full_name")
    pub name: String,
    /// Parsed type of the path parameter
    pub field_type: UnifiedType,
    /// Documentation from protobuf field comments
    pub documentation: Option<String>,
}

impl From<PathParam> for RequestParam {
    fn from(param: PathParam) -> Self {
        RequestParam::Path(param)
    }
}

/// A query parameter for HTTP requests
#[derive(Debug, Clone)]
pub struct QueryParam {
    /// Parameter name
    pub name: String,
    /// Parsed type of the query parameter
    pub field_type: UnifiedType,
    /// Documentation from protobuf field comments
    pub documentation: Option<String>,
    /// Resource reference annotation, if present on the corresponding proto field.
    ///
    /// - `child_type` non-empty: this param scopes a parent of that resource type
    ///   (e.g. `catalog_name` with `child_type = "unitycatalog.io/Schema"`).
    /// - `r#type` non-empty: this param directly identifies a resource of that type.
    pub resource_reference: Option<crate::google::api::ResourceReference>,
}

impl QueryParam {
    /// Denotes if the parameter is optional
    pub fn is_optional(&self) -> bool {
        self.field_type.is_optional
    }
}

impl From<QueryParam> for RequestParam {
    fn from(param: QueryParam) -> Self {
        RequestParam::Query(param)
    }
}

/// A body field that should be extracted from the request body
#[derive(Debug, Clone)]
pub struct BodyField {
    /// Field name
    pub name: String,
    /// Parsed type of the body parameter
    pub field_type: UnifiedType,
    /// Whether this field is a repeated (Vec) type
    pub repeated: bool,
    /// Whether the field is explicitly marked `(google.api.field_behavior) = REQUIRED`.
    pub required: bool,
    /// For oneof fields, the variants with their names and types
    pub oneof_variants: Option<Vec<OneofVariant>>,
    /// Documentation from protobuf field comments
    pub documentation: Option<String>,
}

impl BodyField {
    /// Denotes whether this field should be treated as optional in builder APIs.
    ///
    /// A field marked `REQUIRED` is never optional — even for complex types — so it becomes a
    /// required constructor parameter rather than a `with_*` setter. (Without this, a required
    /// message body like `tag_assignment` would be a no-arg-constructor + optional setter, and the
    /// NAPI binding would execute with an empty body.)
    ///
    /// Otherwise a field is optional when its `UnifiedType.is_optional` flag is set, when it is
    /// repeated, or when its base type is `Map`, `Message`, or `OneOf` (complex types have a valid
    /// default and are treated as optional constructor parameters).
    pub fn is_optional(&self) -> bool {
        use crate::parsing::types::BaseType;
        if self.required {
            return false;
        }
        self.field_type.is_optional
            || self.repeated
            || matches!(
                self.field_type.base_type,
                BaseType::Map(_, _) | BaseType::Message(_) | BaseType::OneOf(_)
            )
    }
}

impl From<BodyField> for RequestParam {
    fn from(field: BodyField) -> Self {
        RequestParam::Body(field)
    }
}

/// Information about a resource managed by a service
#[derive(Debug, Clone)]
pub struct ManagedResource {
    /// Resource type name (e.g., "Catalog")
    pub type_name: String,
    /// Resource descriptor information
    pub descriptor: ResourceDescriptor,
}

/// Describes one ancestor step in a managed resource's parent chain, derived from
/// `google.api.resource_reference { child_type }` annotations on List request fields.
///
/// Entries in [`ServicePlan::hierarchy`] are ordered **root-first** (shallowest ancestor first),
/// so iterating them in order produces the correct param list for resource accessors (e.g.
/// `["catalog_name", "schema_name"]` for a Table, where catalog is depth 0 and schema depth 1).
///
/// Built during analysis via the cross-service global parent map and stored on [`ServicePlan`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ResourceHierarchy {
    /// The service's managed resource type string (e.g. `"unitycatalog.io/Table"`).
    ///
    /// Note: on flat APIs this equals the `child_type` annotation value, but the *actual*
    /// resource type of the ancestor is `parent_resource_type`, which may differ.
    pub child_resource_type: String,
    /// The actual resource type of the ancestor identified by `parent_field_name`
    /// (e.g. `"unitycatalog.io/Catalog"` for the `catalog_name` field on ListTablesRequest).
    ///
    /// This may differ from `child_resource_type` for grandparent fields on flat APIs
    /// (e.g. `catalog_name` on ListTablesRequest has `child_type = Table` but the field
    /// actually identifies a Catalog resource).
    pub parent_resource_type: String,
    /// The proto field name carrying the ancestor identifier (e.g. `"catalog_name"`).
    pub parent_field_name: String,
    /// The singular name of the ancestor resource (e.g. `"catalog"`), resolved by stripping
    /// `"_name"` from `parent_field_name` and matching against known resource descriptors.
    /// `None` when the singular cannot be resolved.
    pub parent_singular: Option<String>,
}

/// Classifies an RPC method as a standard CRUD operation or custom operation.
///
/// This is an internal helper used by [`super::analyze_method`]. Construct via
/// [`MethodPlanner::try_new`] and consume with [`MethodPlanner::request_type`] and
/// [`MethodPlanner::http_pattern`].
pub(crate) struct MethodPlanner<'a> {
    method: &'a MethodMetadata,
    pattern: Pattern,
    path: HttpPattern,
    metadata: &'a CodeGenMetadata,
}

impl<'a> MethodPlanner<'a> {
    pub(crate) fn try_new(
        method: &'a MethodMetadata,
        metadata: &'a CodeGenMetadata,
    ) -> Result<Self> {
        let Some(pattern) = &method.http_rule.pattern else {
            return Err(Error::MissingAnnotation {
                object: method.method_name.clone(),
                message: "Missing HTTP rule pattern".to_string(),
            });
        };
        Ok(Self {
            method,
            path: method.http_pattern.clone(),
            pattern: pattern.clone(),
            metadata,
        })
    }

    /// Consume the planner and return the pre-parsed HTTP URL pattern.
    pub(crate) fn into_http_pattern(self) -> HttpPattern {
        self.path
    }

    /// Classify the RPC as a standard CRUD operation per Google AIP 131-135.
    ///
    /// Each standard operation is identified by matching (verb, HTTP method, path shape,
    /// resource lookup). See:
    /// - [AIP-131](https://google.aip.dev/131) Get
    /// - [AIP-132](https://google.aip.dev/132) List
    /// - [AIP-133](https://google.aip.dev/133) Create
    /// - [AIP-134](https://google.aip.dev/134) Update
    /// - [AIP-135](https://google.aip.dev/135) Delete
    pub(crate) fn request_type(&self) -> RequestType {
        let snake_name = self.method.method_name.to_case(Case::Snake);
        let verb_resource = snake_name.split_once('_');

        if let Some((verb, resource)) = verb_resource {
            // Table of (verb, expected pattern, path must end with parameter?, lookup by plural?)
            #[allow(clippy::type_complexity)]
            let standard_ops: &[(
                &str,
                fn(&Pattern) -> bool,
                bool,
                bool,
                RequestType,
            )] = &[
                (
                    "get",
                    |p| matches!(p, Pattern::Get(_)),
                    true,
                    false,
                    RequestType::Get,
                ),
                (
                    "list",
                    |p| matches!(p, Pattern::Get(_)),
                    false,
                    true,
                    RequestType::List,
                ),
                (
                    "create",
                    |p| matches!(p, Pattern::Post(_)),
                    false,
                    false,
                    RequestType::Create,
                ),
                (
                    "update",
                    |p| matches!(p, Pattern::Patch(_)),
                    true,
                    false,
                    RequestType::Update,
                ),
                (
                    "delete",
                    |p| matches!(p, Pattern::Delete(_)),
                    true,
                    false,
                    RequestType::Delete,
                ),
            ];

            for &(expected_verb, pattern_check, ends_with_param, use_plural, ref result_type) in
                standard_ops
            {
                if verb != expected_verb || !pattern_check(&self.pattern) {
                    continue;
                }
                if ends_with_param && self.path.ends_with_static() {
                    continue;
                }
                if !ends_with_param && self.path.ends_with_parameter() {
                    continue;
                }
                let found = if use_plural {
                    self.metadata.resource_from_plural(resource).is_some()
                } else {
                    self.metadata.resource_from_singular(resource).is_some()
                };
                if found {
                    return result_type.clone();
                }
            }
        }

        RequestType::Custom(self.pattern.clone())
    }

    pub(crate) fn has_response(&self) -> bool {
        !self.method.output_type.is_empty() && !self.method.output_type.ends_with("Empty")
    }

    /// Extract the simple resource type name from the method's output type.
    ///
    /// Strips the package prefix (e.g., `.example.catalog.v1.Catalog` → `Catalog`).
    pub(crate) fn output_resource_type(&self) -> Option<String> {
        if self.has_response() {
            let output_type = &self.method.output_type;
            let simple = output_type
                .rfind('.')
                .map(|i| &output_type[i + 1..])
                .unwrap_or(output_type);
            Some(simple.to_string())
        } else {
            None
        }
    }
}

/// Whether a request of this type carries a JSON body (create / update / custom POST·PATCH).
///
/// Single source of truth for the body-attach decision the client emitter previously re-derived.
pub(crate) fn request_has_body(request_type: &RequestType) -> bool {
    matches!(
        request_type,
        RequestType::Create
            | RequestType::Update
            | RequestType::Custom(Pattern::Post(_))
            | RequestType::Custom(Pattern::Patch(_))
    )
}

/// Whether the Axum server extractor reads request *parts* (path/query) rather than the body.
///
/// Single source of truth for the `FromRequestParts`-vs-`FromRequest` decision (and the
/// `RequestPartsExt` import) the server emitter previously re-derived.
pub(crate) fn request_needs_request_parts(request_type: &RequestType) -> bool {
    matches!(
        request_type,
        RequestType::List | RequestType::Get | RequestType::Delete
    ) || matches!(
        request_type,
        RequestType::Custom(Pattern::Get(_) | Pattern::Delete(_))
    )
}

/// Whether a method is a collection-style operation (list / create / factory RPC).
///
/// Single source of truth for the collection/instance split. Standard `List`/`Create` always
/// qualify; custom RPCs qualify by name convention — a `GET` named `List*` (a resource-less list)
/// or a `POST` named `Generate*` (a factory RPC like `GenerateCredentials`). This is the only place
/// the `List`/`Generate` proto-name heuristics live.
pub(crate) fn is_collection_method(request_type: &RequestType, method_name: &str) -> bool {
    matches!(request_type, RequestType::List | RequestType::Create)
        || (matches!(request_type, RequestType::Custom(Pattern::Get(_)))
            && method_name.starts_with("List"))
        || (matches!(request_type, RequestType::Custom(Pattern::Post(_)))
            && method_name.starts_with("Generate"))
}

/// Classify a method's [`MethodShape`] from its request type, proto name, and whether the owning
/// service manages a `google.api.resource`.
///
/// - Resource-less service → [`MethodShape::Unbound`] for every method.
/// - Resource service, collection-style method → [`MethodShape::CollectionScoped`].
/// - Resource service, otherwise → [`MethodShape::InstanceScoped`].
pub(crate) fn method_shape(
    request_type: &RequestType,
    method_name: &str,
    service_has_resource: bool,
) -> MethodShape {
    if !service_has_resource {
        MethodShape::Unbound
    } else if is_collection_method(request_type, method_name) {
        MethodShape::CollectionScoped
    } else {
        MethodShape::InstanceScoped
    }
}

/// The verb to call on a resource-scoped client for standard instance operations, or `None`.
///
/// Single source of truth for the scoped-verb mapping the binding emitters previously re-derived.
pub(crate) fn scoped_verb(request_type: &RequestType) -> Option<String> {
    match request_type {
        RequestType::Get => Some("get".to_string()),
        RequestType::Update => Some("update".to_string()),
        RequestType::Delete => Some("delete".to_string()),
        _ => None,
    }
}

/// Split body fields from a `MethodPlan` into required and optional subsets.
///
/// Delegates to [`BodyField::is_optional`] for the classification. Optional fields
/// become `with_*` setter methods; required fields become constructor parameters.
pub fn split_body_fields(plan: &MethodPlan) -> (Vec<&BodyField>, Vec<&BodyField>) {
    let mut required = Vec::new();
    let mut optional = Vec::new();
    for field in plan.body_fields() {
        if field.is_optional() {
            optional.push(field);
        } else {
            required.push(field);
        }
    }
    (required, optional)
}

/// Extract managed resources from service methods, deduplicating by type name.
pub fn extract_managed_resources(
    metadata: &CodeGenMetadata,
    methods: &[MethodPlan],
) -> Vec<ManagedResource> {
    let mut resources = Vec::new();
    let mut seen_types = HashSet::<String>::new();

    for method in methods {
        if let Some(ref resource_type) = method.output_resource_type {
            if seen_types.contains(resource_type) {
                continue;
            }
            if let Some(descriptor) = metadata.get_resource_descriptor(resource_type) {
                resources.push(ManagedResource {
                    type_name: resource_type.clone(),
                    descriptor: descriptor.clone(),
                });
                seen_types.insert(resource_type.clone());
            }
        }
    }

    resources
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom(p: Pattern) -> RequestType {
        RequestType::Custom(p)
    }

    #[test]
    fn request_has_body_matches_write_verbs() {
        assert!(request_has_body(&RequestType::Create));
        assert!(request_has_body(&RequestType::Update));
        assert!(request_has_body(&custom(Pattern::Post(String::new()))));
        assert!(request_has_body(&custom(Pattern::Patch(String::new()))));

        assert!(!request_has_body(&RequestType::List));
        assert!(!request_has_body(&RequestType::Get));
        assert!(!request_has_body(&RequestType::Delete));
        assert!(!request_has_body(&custom(Pattern::Get(String::new()))));
        assert!(!request_has_body(&custom(Pattern::Delete(String::new()))));
    }

    #[test]
    fn needs_request_parts_matches_read_verbs() {
        assert!(request_needs_request_parts(&RequestType::List));
        assert!(request_needs_request_parts(&RequestType::Get));
        assert!(request_needs_request_parts(&RequestType::Delete));
        assert!(request_needs_request_parts(&custom(Pattern::Get(
            String::new()
        ))));
        assert!(request_needs_request_parts(&custom(Pattern::Delete(
            String::new()
        ))));

        assert!(!request_needs_request_parts(&RequestType::Create));
        assert!(!request_needs_request_parts(&RequestType::Update));
        assert!(!request_needs_request_parts(&custom(Pattern::Post(
            String::new()
        ))));
        assert!(!request_needs_request_parts(&custom(Pattern::Patch(
            String::new()
        ))));
    }

    #[test]
    fn has_body_and_needs_parts_are_mutually_exclusive_for_known_verbs() {
        // A method either reads from parts or carries a body — never both, never neither —
        // for the verb shapes the emitters actually produce extractors/clients for.
        for rt in [
            RequestType::List,
            RequestType::Create,
            RequestType::Get,
            RequestType::Update,
            RequestType::Delete,
            custom(Pattern::Get(String::new())),
            custom(Pattern::Post(String::new())),
            custom(Pattern::Patch(String::new())),
            custom(Pattern::Delete(String::new())),
        ] {
            assert_ne!(
                request_has_body(&rt),
                request_needs_request_parts(&rt),
                "body vs request-parts should partition {rt:?}"
            );
        }
    }

    #[test]
    fn is_collection_method_standard_and_name_heuristics() {
        // Standard collection verbs always qualify, regardless of name.
        assert!(is_collection_method(&RequestType::List, "ListThings"));
        assert!(is_collection_method(&RequestType::Create, "CreateThing"));

        // Custom GET named `List*` (resource-less list) qualifies; other custom GETs don't.
        assert!(is_collection_method(
            &custom(Pattern::Get(String::new())),
            "ListTagAssignments"
        ));
        assert!(!is_collection_method(
            &custom(Pattern::Get(String::new())),
            "GetTagAssignment"
        ));

        // Custom POST named `Generate*` (factory RPC) qualifies; other custom POSTs don't.
        assert!(is_collection_method(
            &custom(Pattern::Post(String::new())),
            "GenerateCredentials"
        ));
        assert!(!is_collection_method(
            &custom(Pattern::Post(String::new())),
            "RotateToken"
        ));

        // Instance verbs never qualify.
        assert!(!is_collection_method(&RequestType::Get, "GetThing"));
        assert!(!is_collection_method(&RequestType::Update, "UpdateThing"));
        assert!(!is_collection_method(&RequestType::Delete, "DeleteThing"));
    }

    #[test]
    fn method_shape_classification() {
        // Resource-less service → everything Unbound, regardless of verb.
        assert_eq!(
            method_shape(&RequestType::List, "ListTagAssignments", false),
            MethodShape::Unbound
        );
        assert_eq!(
            method_shape(&RequestType::Get, "GetTagAssignment", false),
            MethodShape::Unbound
        );
        assert_eq!(
            method_shape(&custom(Pattern::Post(String::new())), "TouchTag", false),
            MethodShape::Unbound
        );

        // Resource service → collection verbs are CollectionScoped, instance verbs InstanceScoped.
        assert_eq!(
            method_shape(&RequestType::List, "ListCatalogs", true),
            MethodShape::CollectionScoped
        );
        assert_eq!(
            method_shape(&RequestType::Create, "CreateCatalog", true),
            MethodShape::CollectionScoped
        );
        assert_eq!(
            method_shape(
                &custom(Pattern::Post(String::new())),
                "GenerateCredentials",
                true
            ),
            MethodShape::CollectionScoped
        );
        assert_eq!(
            method_shape(&RequestType::Get, "GetCatalog", true),
            MethodShape::InstanceScoped
        );
        assert_eq!(
            method_shape(&RequestType::Update, "UpdateCatalog", true),
            MethodShape::InstanceScoped
        );
        assert_eq!(
            method_shape(&RequestType::Delete, "DeleteCatalog", true),
            MethodShape::InstanceScoped
        );
        // Resource-targeted custom POST (not a factory) stays instance-scoped.
        assert_eq!(
            method_shape(&custom(Pattern::Post(String::new())), "RotateToken", true),
            MethodShape::InstanceScoped
        );
    }

    #[test]
    fn emit_shape_classification() {
        // Standard verbs map directly.
        assert_eq!(emit_shape(&RequestType::List, false), EmitShape::List);
        assert_eq!(emit_shape(&RequestType::Create, false), EmitShape::Create);
        assert_eq!(emit_shape(&RequestType::Get, false), EmitShape::GetUpdate);
        assert_eq!(
            emit_shape(&RequestType::Update, false),
            EmitShape::GetUpdate
        );
        assert_eq!(emit_shape(&RequestType::Delete, false), EmitShape::Delete);

        // A collection-style custom POST/PATCH (a factory, e.g. `Generate*`) is Create-shaped.
        assert_eq!(
            emit_shape(&custom(Pattern::Post(String::new())), true),
            EmitShape::Create
        );
        assert_eq!(
            emit_shape(&custom(Pattern::Patch(String::new())), true),
            EmitShape::Create
        );

        // A resource-targeted (non-collection) custom POST/PATCH is GetUpdate-shaped.
        assert_eq!(
            emit_shape(&custom(Pattern::Post(String::new())), false),
            EmitShape::GetUpdate
        );

        // A custom GET (even named `List*`, is_collection=true) is NOT List-shaped — it returns the
        // full response message, matching the old flat dispatch's `Custom(_) => get_update` arm.
        assert_eq!(
            emit_shape(&custom(Pattern::Get(String::new())), true),
            EmitShape::GetUpdate
        );
        assert_eq!(
            emit_shape(&custom(Pattern::Get(String::new())), false),
            EmitShape::GetUpdate
        );
        // A custom DELETE is GetUpdate-shaped (the old flat `Custom(_)` arm), not Delete.
        assert_eq!(
            emit_shape(&custom(Pattern::Delete(String::new())), false),
            EmitShape::GetUpdate
        );
    }

    #[test]
    fn scoped_verb_only_for_standard_instance_ops() {
        assert_eq!(scoped_verb(&RequestType::Get).as_deref(), Some("get"));
        assert_eq!(scoped_verb(&RequestType::Update).as_deref(), Some("update"));
        assert_eq!(scoped_verb(&RequestType::Delete).as_deref(), Some("delete"));

        assert_eq!(scoped_verb(&RequestType::List), None);
        assert_eq!(scoped_verb(&RequestType::Create), None);
        assert_eq!(scoped_verb(&custom(Pattern::Post(String::new()))), None);
        assert_eq!(scoped_verb(&custom(Pattern::Get(String::new()))), None);
    }
}
