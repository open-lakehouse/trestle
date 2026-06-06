//! Ergonomic resource-scoped client generation.
//!
//! For a resource-scoped service this emits a thin client (e.g. `CatalogClient`) that captures the
//! resource's name components and exposes:
//! - constructors: `new(components…, client)` and — for nested resources — `from_full_name(full_name,
//!   client)`, which splits the dot-joined name into components once and forwards to `new`;
//! - instance operations (`get`/`update`/`delete` and resource-targeted custom RPCs), each
//!   returning the matching generated builder with the captured components injected as the path arg;
//! - child-navigation accessors (`catalog.schema(name) -> SchemaClient`) and child-create methods
//!   (`catalog.create_schema(name) -> CreateSchemaBuilder`) for each direct child resource, reusing
//!   the parent's captured components.
//!
//! This replaces the previously hand-written scoped clients in consuming crates. The struct (with
//! `pub(crate)` fields) is generated into the consuming crate's source tree alongside the low-level
//! client and builders, so hand-written extension `impl` blocks (pagination streams, bespoke
//! helpers) in that crate compose with it as additional inherent-impl blocks and can read the
//! captured components + low-level client.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::{MethodHandler, ServiceHandler, doc_tokens, format_tokens};
use crate::analysis::{GenerationPlan, RequestParam, RequestType};
use crate::error::Result;

/// Generate the `resource.rs` module for one resource-scoped service.
///
/// Returns `None` for resource-less services (they have no scoped client — their methods live on
/// the aggregate/root client). `plan` is used to resolve direct-child services for the navigation
/// and create accessors.
pub(crate) fn generate(
    service: &ServiceHandler<'_>,
    plan: &GenerationPlan,
) -> Result<Option<String>> {
    let Some(scoped_ident) = service.scoped_client_type() else {
        return Ok(None);
    };
    let Some(spec) = service.accessor_spec() else {
        return Ok(None);
    };

    let low_level_ident = service.low_level_client_type();
    let components: Vec<_> = spec.params.iter().map(|p| format_ident!("{}", p)).collect();
    let join_format = spec.join_format();

    let struct_def = scoped_struct(&scoped_ident, &components, &low_level_ident);
    let constructor = scoped_constructor(&components, &low_level_ident);
    // For nested resources, also offer a `from_full_name` constructor that splits a dot-joined
    // full name into the component fields once (no round-trip). Flat resources skip it — it would
    // duplicate `new`.
    let from_full_name = if spec.nested {
        from_full_name_constructor(&components, &low_level_ident, &spec.singular)
    } else {
        quote! {}
    };
    let methods = instance_methods(service, &components, &join_format);
    let child_methods = child_methods(service, plan, &components);

    // Child-create methods take parameter types (enums/messages) from each child's own models, so
    // import those models modules in addition to this service's. Deduped, excluding our own path.
    let child_model_imports = child_model_import_paths(service, plan);

    let singular_doc = format!(" A client scoped to a single `{}`.", spec.singular);
    let mod_path = service.models_path();

    let tokens = quote! {
        use #mod_path::*;
        #(#child_model_imports)*
        use super::builders::*;
        use super::client::#low_level_ident;

        #[doc = #singular_doc]
        #[derive(Clone)]
        pub struct #scoped_ident {
            #struct_def
            pub(crate) client: #low_level_ident,
        }

        impl #scoped_ident {
            #constructor
            #from_full_name
            #(#methods)*
            #(#child_methods)*
        }
    };

    Ok(Some(format_tokens(tokens)?))
}

/// `use <child models path>::*;` imports for every direct child, deduped and excluding this
/// service's own models path (already imported by the caller).
///
/// Child-create methods reference enum/message parameter types from the *child's* models (e.g.
/// `SchemaClient::create_function` uses `ParameterStyle`/`RoutineBody` from the functions models),
/// so those modules must be in scope. A child whose Create method takes only scalars still gets an
/// (unused) glob import — harmless and simpler than tracking per-type provenance.
fn child_model_import_paths(
    service: &ServiceHandler<'_>,
    plan: &GenerationPlan,
) -> Vec<TokenStream> {
    let own = service.models_path();
    let own_str = quote! { #own }.to_string();

    let mut seen = std::collections::BTreeSet::new();
    let mut imports = Vec::new();
    for link in &service.plan.direct_children {
        let Some(child_plan) = plan
            .services
            .iter()
            .find(|s| s.base_path == link.child_base_path)
        else {
            continue;
        };
        let child = ServiceHandler {
            plan: child_plan,
            metadata: service.metadata,
            config: service.config,
        };
        let path = child.models_path();
        let key = quote! { #path }.to_string();
        if key == own_str || !seen.insert(key) {
            continue;
        }
        imports.push(quote! { use #path::*; });
    }
    imports
}

/// Emit child-navigation and child-create methods for each direct child of this resource.
///
/// `parent_components` are the parent's captured struct fields (in order); by the prefix relation
/// they are exactly the leading components of every child's accessor params.
fn child_methods(
    service: &ServiceHandler<'_>,
    plan: &GenerationPlan,
    parent_components: &[proc_macro2::Ident],
) -> Vec<TokenStream> {
    let mut out = Vec::new();
    for link in &service.plan.direct_children {
        // Resolve the child service's plan to build a handler for its client/builder idents.
        let Some(child_plan) = plan
            .services
            .iter()
            .find(|s| s.base_path == link.child_base_path)
        else {
            continue;
        };
        let child = ServiceHandler {
            plan: child_plan,
            metadata: service.metadata,
            config: service.config,
        };

        if let Some(nav) = child_nav_method(&child, link, parent_components) {
            out.push(nav);
        }
        if let Some(create) = child_create_method(&child, link, parent_components) {
            out.push(create);
        }
    }
    out
}

/// `pub fn <child>(&self, <leaf>: impl Into<String>) -> crate::codegen::<base>::<Child>Client`.
///
/// Forwards the parent's captured components plus the one new trailing component to the child's
/// scoped-client `new`, building the child's low-level client from this client's cloud client + URL.
fn child_nav_method(
    child: &ServiceHandler<'_>,
    link: &crate::analysis::ChildLink,
    parent_components: &[proc_macro2::Ident],
) -> Option<TokenStream> {
    let child_scoped = child.scoped_client_type()?;
    let child_low_level = child.low_level_client_type();
    let module = format_ident!("{}", link.child_base_path);

    let method_name = format_ident!("{}", link.child_singular);
    // The child's accessor params are parent params + one trailing leaf; the leaf is the new arg.
    let leaf = link.child_accessor_params.last()?;
    let leaf_ident = format_ident!("{}", leaf);

    let parent_args = parent_components.iter().map(|c| quote! { &self.#c });
    let doc = format!(" Access a `{}` within this resource.", link.child_singular);

    Some(quote! {
        #[doc = #doc]
        pub fn #method_name(&self, #leaf_ident: impl Into<String>) -> crate::codegen::#module::#child_scoped {
            crate::codegen::#module::#child_scoped::new(
                #(#parent_args,)*
                #leaf_ident,
                crate::codegen::#module::#child_low_level::new(
                    self.client.client.clone(),
                    self.client.base_url.clone(),
                ),
            )
        }
    })
}

/// `pub fn create_<child>(&self, <args>) -> crate::codegen::<base>::Create<Child>Builder`.
///
/// Reuses the parent's captured components for the child Create builder's matching path params; the
/// child's own name + other required fields become method arguments, in the builder's `::new` order.
fn child_create_method(
    child: &ServiceHandler<'_>,
    link: &crate::analysis::ChildLink,
    parent_components: &[proc_macro2::Ident],
) -> Option<TokenStream> {
    let create = child
        .methods()
        .find(|m| m.plan.request_type == RequestType::Create)?;
    let child_low_level = child.low_level_client_type();
    let module = format_ident!("{}", link.child_base_path);
    let builder_ty = create.builder_type();
    let method_name = format_ident!("create_{}", link.child_singular);

    // Names of the parent's captured components, for matching the child Create builder's params.
    let parent_names: Vec<String> = parent_components.iter().map(|c| c.to_string()).collect();
    let required: Vec<&RequestParam> = create.required_parameters().collect();

    // The builder's `::new` takes the Create method's required params in order. Classify each as a
    // parent-component fill or a method argument (see `classify_create_args`), then render.
    let mut new_args: Vec<TokenStream> = Vec::new();
    let mut method_param_defs: Vec<TokenStream> = Vec::new();
    for (param, source) in required
        .iter()
        .zip(classify_create_args(&required, &parent_names))
    {
        match source {
            CreateArg::ParentComponent => {
                let field = format_ident!("{}", param.name());
                new_args.push(quote! { &self.#field });
            }
            CreateArg::MethodArg => {
                let ident = param.field_ident();
                let ty = create.field_type(
                    param.field_type(),
                    crate::parsing::types::RenderContext::Constructor,
                );
                method_param_defs.push(quote! { #ident: #ty });
                new_args.push(quote! { #ident });
            }
        }
    }

    let doc = format!(" Create a `{}` within this resource.", link.child_singular);
    Some(quote! {
        #[doc = #doc]
        pub fn #method_name(&self, #(#method_param_defs),*) -> crate::codegen::#module::#builder_ty {
            crate::codegen::#module::#builder_ty::new(
                crate::codegen::#module::#child_low_level::new(
                    self.client.client.clone(),
                    self.client.base_url.clone(),
                ),
                #(#new_args),*
            )
        }
    })
}

/// How a child Create builder argument is supplied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CreateArg {
    /// Filled from the parent's captured component of the same name (`&self.<name>`).
    ParentComponent,
    /// Supplied by the caller as a method argument.
    MethodArg,
}

/// Classify each of a child Create method's required params (in builder `::new` order) as either a
/// parent-component fill or a caller-supplied method argument.
///
/// A param whose name matches one of the parent's captured component names is a `ParentComponent`;
/// everything else (the child's own name, other required fields) is a `MethodArg`.
fn classify_create_args(required: &[&RequestParam], parent_names: &[String]) -> Vec<CreateArg> {
    required
        .iter()
        .map(|p| {
            if parent_names.iter().any(|n| n == p.name()) {
                CreateArg::ParentComponent
            } else {
                CreateArg::MethodArg
            }
        })
        .collect()
}

/// The struct's captured-component fields (each a `String`).
fn scoped_struct(
    _scoped_ident: &proc_macro2::Ident,
    components: &[proc_macro2::Ident],
    _low_level_ident: &proc_macro2::Ident,
) -> TokenStream {
    // Fields are `pub(crate)` so hand-written extension impls in the consuming crate can read the
    // captured name components (and the low-level `client`) when adding bespoke methods.
    let fields = components.iter().map(|c| quote! { pub(crate) #c: String, });
    quote! { #(#fields)* }
}

/// `pub fn new(<component>: impl Into<String>, …, client: <LowLevel>) -> Self`.
fn scoped_constructor(
    components: &[proc_macro2::Ident],
    low_level_ident: &proc_macro2::Ident,
) -> TokenStream {
    let params = components.iter().map(|c| quote! { #c: impl Into<String> });
    let assigns = components.iter().map(|c| quote! { #c: #c.into() });
    quote! {
        /// Create a client bound to the resource's name components.
        pub fn new(#(#params,)* client: #low_level_ident) -> Self {
            Self {
                #(#assigns,)*
                client,
            }
        }
    }
}

/// `pub fn from_full_name(full_name, client) -> Self` for a nested resource.
///
/// Splits the dot-joined `full_name` into its component fields **once** (`splitn(N, '.')`, so the
/// final component keeps any trailing dots) and forwards to [`Self::new`] — no parse-then-rejoin
/// round-trip. Only emitted for nested resources (`components.len() > 1`); for a flat resource it
/// would duplicate `new`.
fn from_full_name_constructor(
    components: &[proc_macro2::Ident],
    low_level_ident: &proc_macro2::Ident,
    singular: &str,
) -> TokenStream {
    let n = components.len();
    // Bind each component from the split iterator, in order.
    let bindings = components.iter().map(|c| {
        quote! { let #c = parts.next().unwrap_or_default(); }
    });
    let args = components.iter().map(|c| quote! { #c });
    let doc = format!(
        " Create a `{singular}` client from its dot-joined full name (e.g. `\"{}\"`).",
        components
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(".")
    );
    quote! {
        #[doc = #doc]
        pub fn from_full_name(full_name: impl Into<String>, client: #low_level_ident) -> Self {
            let full_name = full_name.into();
            let mut parts = full_name.splitn(#n, '.');
            #(#bindings)*
            Self::new(#(#args,)* client)
        }
    }
}

/// One method per `is_scoped_instance_method()` RPC, returning its builder.
fn instance_methods(
    service: &ServiceHandler<'_>,
    components: &[proc_macro2::Ident],
    join_format: &str,
) -> Vec<TokenStream> {
    service
        .methods()
        .filter(|m| m.is_scoped_instance_method())
        .map(|m| instance_method(&m, components, join_format))
        .collect()
}

/// Emit a single instance method: `pub fn <verb>(&self, <non-path args>) -> <Builder> { … }`.
///
/// The builder's `::new` takes the method's required params in order (path params + required body
/// fields). Path params are filled from the captured components; the remaining required params
/// become arguments of the generated method.
fn instance_method(
    method: &MethodHandler<'_>,
    components: &[proc_macro2::Ident],
    join_format: &str,
) -> TokenStream {
    let doc = doc_tokens(method.plan.metadata.documentation.as_deref());
    let method_name = method.plan.resource_client_method();
    let builder_ty = method.builder_type();

    let required: Vec<&RequestParam> = method.required_parameters().collect();
    let path_param_count = required.iter().filter(|p| p.is_path_param()).count();

    // Build the ordered argument expressions for `<Builder>::new(self.client.clone(), <args>)`, and
    // collect the non-path required params that must become method arguments.
    let mut new_args: Vec<TokenStream> = Vec::new();
    let mut method_param_defs: Vec<TokenStream> = Vec::new();
    for param in &required {
        if param.is_path_param() {
            new_args.push(path_arg_expr(
                param,
                components,
                join_format,
                path_param_count,
            ));
        } else {
            // A required non-path field (e.g. a required body field) becomes a method argument,
            // typed like the builder's constructor expects.
            let ident = param.field_ident();
            let ty = method.field_type(
                param.field_type(),
                crate::parsing::types::RenderContext::Constructor,
            );
            method_param_defs.push(quote! { #ident: #ty });
            new_args.push(quote! { #ident });
        }
    }

    quote! {
        #doc
        pub fn #method_name(&self, #(#method_param_defs),*) -> #builder_ty {
            #builder_ty::new(self.client.clone(), #(#new_args),*)
        }
    }
}

/// The expression filling a single path parameter from the captured components.
///
/// - **Composite**: a single path param on a multi-component resource is the dot-joined full name
///   (e.g. `format!("{}.{}", self.catalog_name, self.schema_name)`).
/// - **Direct**: otherwise the path param maps to the same-named captured component (`&self.name`).
fn path_arg_expr(
    param: &RequestParam,
    components: &[proc_macro2::Ident],
    join_format: &str,
    path_param_count: usize,
) -> TokenStream {
    if path_param_count == 1 && components.len() > 1 {
        // Composite full name: join all captured components in order.
        let field_refs = components.iter().map(|c| quote! { self.#c });
        quote! { format!(#join_format, #(#field_refs),*) }
    } else {
        // Direct: this path param corresponds to a captured component of the same name. Fall back to
        // the first component if no exact name match (single-component resources name their path
        // param `name` while the component is e.g. `catalog_name`).
        let name = param.name();
        let field = components
            .iter()
            .find(|c| *c == name)
            .cloned()
            .unwrap_or_else(|| components[0].clone());
        quote! { &self.#field }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{PathParam, QueryParam};
    use crate::parsing::types::{BaseType, UnifiedType};

    fn idents(names: &[&str]) -> Vec<proc_macro2::Ident> {
        names.iter().map(|n| format_ident!("{}", n)).collect()
    }

    fn string_type() -> UnifiedType {
        UnifiedType {
            base_type: BaseType::String,
            is_optional: false,
            is_repeated: false,
        }
    }

    fn path(name: &str) -> RequestParam {
        RequestParam::Path(PathParam {
            name: name.to_string(),
            field_type: string_type(),
            documentation: None,
        })
    }

    fn join_format(n: usize) -> String {
        std::iter::repeat_n("{}", n).collect::<Vec<_>>().join(".")
    }

    /// Flat resource: one component, one path param named `name` (mismatch with the component name)
    /// → falls back to the single captured component by reference.
    #[test]
    fn flat_single_component_direct_ref() {
        let components = idents(&["catalog_name"]);
        let expr = path_arg_expr(&path("name"), &components, &join_format(1), 1);
        assert_eq!(expr.to_string(), quote! { &self.catalog_name }.to_string());
    }

    /// Flat resource where the path param name matches the component → direct ref to that component.
    #[test]
    fn flat_matching_name_direct_ref() {
        let components = idents(&["catalog_name"]);
        let expr = path_arg_expr(&path("catalog_name"), &components, &join_format(1), 1);
        assert_eq!(expr.to_string(), quote! { &self.catalog_name }.to_string());
    }

    /// Nested resource with a single composite path param (e.g. `full_name`) and multiple captured
    /// components → dot-joined `format!`.
    #[test]
    fn nested_single_path_param_joins_components() {
        let components = idents(&["catalog_name", "schema_name"]);
        let expr = path_arg_expr(&path("full_name"), &components, &join_format(2), 1);
        assert_eq!(
            expr.to_string(),
            quote! { format!("{}.{}", self.catalog_name, self.schema_name) }.to_string()
        );
    }

    /// Nested resource whose builder takes separate path params (count > 1) → each path param maps
    /// to its same-named captured component by reference (no join).
    #[test]
    fn nested_separate_path_params_map_by_name() {
        let components = idents(&["catalog_name", "schema_name"]);
        let catalog = path_arg_expr(&path("catalog_name"), &components, &join_format(2), 2);
        let schema = path_arg_expr(&path("schema_name"), &components, &join_format(2), 2);
        assert_eq!(
            catalog.to_string(),
            quote! { &self.catalog_name }.to_string()
        );
        assert_eq!(schema.to_string(), quote! { &self.schema_name }.to_string());
    }

    /// A non-path param is never routed through `path_arg_expr`; this guards the classifier we rely
    /// on (`is_path_param`) so the instance-method split stays correct.
    #[test]
    fn query_param_is_not_a_path_param() {
        let q = RequestParam::Query(QueryParam {
            name: "page_token".to_string(),
            field_type: string_type(),
            documentation: None,
            resource_reference: None,
        });
        assert!(!q.is_path_param());
        assert!(path("name").is_path_param());
    }

    // ── child-create arg classification ──────────────────────────────────────────────────────

    /// CreateSchema builder `new(client, name, catalog_name)`: `name` is the child's own (method
    /// arg), `catalog_name` is the parent component (filled from `&self.catalog_name`).
    #[test]
    fn create_schema_args_split_by_parent_components() {
        let required = vec![path("name"), path("catalog_name")];
        let refs: Vec<&RequestParam> = required.iter().collect();
        let parents = vec!["catalog_name".to_string()];
        assert_eq!(
            classify_create_args(&refs, &parents),
            vec![CreateArg::MethodArg, CreateArg::ParentComponent]
        );
    }

    /// CreateTable builder `new(client, name, schema_name, catalog_name)`: only `name` is a method
    /// arg; both `schema_name` and `catalog_name` are parent components. Order is preserved.
    #[test]
    fn create_table_args_split_preserves_builder_order() {
        let required = vec![path("name"), path("schema_name"), path("catalog_name")];
        let refs: Vec<&RequestParam> = required.iter().collect();
        let parents = vec!["catalog_name".to_string(), "schema_name".to_string()];
        assert_eq!(
            classify_create_args(&refs, &parents),
            vec![
                CreateArg::MethodArg,
                CreateArg::ParentComponent,
                CreateArg::ParentComponent,
            ]
        );
    }

    /// Extra required non-name fields (e.g. `table_type`) stay method args.
    #[test]
    fn create_args_extra_required_fields_are_method_args() {
        let required = vec![path("name"), path("catalog_name"), path("table_type")];
        let refs: Vec<&RequestParam> = required.iter().collect();
        let parents = vec!["catalog_name".to_string()];
        assert_eq!(
            classify_create_args(&refs, &parents),
            vec![
                CreateArg::MethodArg,
                CreateArg::ParentComponent,
                CreateArg::MethodArg,
            ]
        );
    }

    // ── from_full_name constructor ───────────────────────────────────────────────────────────

    /// A two-component resource splits the full name with `splitn(2, '.')` and forwards the parts to
    /// `new` in order.
    #[test]
    fn from_full_name_splits_into_components() {
        let components = idents(&["catalog_name", "schema_name"]);
        let low = format_ident!("SchemaServiceClient");
        let out = from_full_name_constructor(&components, &low, "schema").to_string();

        assert!(out.contains("splitn (2usize"), "expected splitn(2): {out}");
        assert!(out.contains("let catalog_name = parts . next ()"), "{out}");
        assert!(out.contains("let schema_name = parts . next ()"), "{out}");
        assert!(
            out.contains("Self :: new (catalog_name , schema_name , client)"),
            "forwards parts in order to new: {out}"
        );
    }

    /// A three-component resource splits with `splitn(3, '.')`.
    #[test]
    fn from_full_name_three_components() {
        let components = idents(&["catalog_name", "schema_name", "table_name"]);
        let low = format_ident!("TableServiceClient");
        let out = from_full_name_constructor(&components, &low, "table").to_string();
        assert!(out.contains("splitn (3usize"), "expected splitn(3): {out}");
        assert!(
            out.contains("Self :: new (catalog_name , schema_name , table_name , client)"),
            "{out}"
        );
    }

    // ── child model imports ──────────────────────────────────────────────────────────────────

    use crate::analysis::{ChildLink, GenerationPlan, ServicePlan};
    use crate::codegen::{CodeGenConfig, CodeGenOutput, ServiceHandler};
    use crate::parsing::CodeGenMetadata;

    fn test_config() -> CodeGenConfig {
        CodeGenConfig {
            context_type_path: "crate::Context".into(),
            result_type_path: "crate::Result".into(),
            models_path_template: "common::models::{service}::v1".into(),
            models_path_crate_template: "crate::models::{service}::v1".into(),
            resource_store_crate_name: "olai_store".into(),
            output: CodeGenOutput {
                common: "/tmp/c".into(),
                models: None,
                models_subdir: "_gen".into(),
                server: None,
                client: None,
                python: None,
                node: None,
                node_ts: None,
                python_typings_filename: "client.pyi".into(),
                generate_resource_clients: true,
            },
            generate_resource_enum: false,
            generate_store_integration: false,
            error_type_path: None,
            generate_object_conversions: false,
            bindings: None,
            models_gen_dir: None,
        }
    }

    /// A resource-scoped `ServicePlan` with the given service name (→ base_path/package) and accessor
    /// params, plus one direct child described by `children`.
    fn svc(service_name: &str, package: &str, children: Vec<ChildLink>) -> ServicePlan {
        use crate::analysis::ManagedResource;
        use crate::google::api::ResourceDescriptor;
        let base_path = crate::utils::strings::service_to_base_path(service_name);
        ServicePlan {
            service_name: service_name.to_string(),
            handler_name: crate::utils::strings::service_to_handler_name(service_name),
            base_path,
            package: package.to_string(),
            methods: vec![],
            managed_resources: vec![ManagedResource {
                type_name: "X".to_string(),
                descriptor: ResourceDescriptor::default(),
            }],
            documentation: None,
            hierarchy: vec![],
            resource_accessor_params: Some(vec!["name".to_string()]),
            direct_children: children,
        }
    }

    /// The child models module is imported, the parent's own path is not re-imported, and the
    /// import list is deduped.
    #[test]
    fn child_model_imports_pull_in_child_models() {
        let child_link = ChildLink {
            child_singular: "schema".to_string(),
            child_base_path: "schema".to_string(),
            child_accessor_params: vec!["name".to_string(), "schema_name".to_string()],
        };
        let plan = GenerationPlan {
            services: vec![
                svc("CatalogService", "example.catalog.v1", vec![child_link]),
                svc("SchemaService", "example.schema.v1", vec![]),
            ],
            skipped_methods: vec![],
        };
        let metadata = CodeGenMetadata::default();
        let config = test_config();
        let parent = ServiceHandler {
            plan: &plan.services[0],
            metadata: &metadata,
            config: &config,
        };

        let imports: Vec<String> = child_model_import_paths(&parent, &plan)
            .iter()
            .map(|t| t.to_string())
            .collect();

        // Exactly one import: the child's (schema) models module. The parent's own (catalog) path
        // is excluded.
        assert_eq!(
            imports.len(),
            1,
            "expected one child import, got {imports:?}"
        );
        assert!(
            imports[0].contains("schema"),
            "child schema models import missing: {imports:?}"
        );
        assert!(
            !imports[0].contains("catalog"),
            "parent's own models path should not be re-imported: {imports:?}"
        );
    }
}
