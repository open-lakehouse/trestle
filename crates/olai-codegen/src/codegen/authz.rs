//! Authorization-vocabulary generation (`authz.rs`, `<service>/authz.rs`, `authz.cedarschema`).
//!
//! This is part of the **Generation** stage (see [`super`]). From the [`AuthzOperation`]s of
//! [`plan_authz`](crate::analysis::plan_authz) it emits, into the server output:
//!
//! - the root `authz.rs`: the `Operation` enum (one variant per RPC), the `Securable` struct and
//!   the `Authorizable` trait;
//! - one `<service>/authz.rs` per service with `impl Authorizable` for its requests;
//! - `authz.cedarschema`: the Cedar actions, one per operation.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::{
    AuthzConfig, CodeGenConfig, Runtime, ServiceHandler, doc_tokens, extract_type_ident,
    format_tokens,
};
use crate::Result;
use crate::analysis::{
    AuthzOperation, GenerationPlan, NameField, NameFieldKind, RequestType, SecurableKind,
};
use crate::parsing::CodeGenMetadata;

/// The root module's file name, also its module name in the server `mod.rs`.
pub(crate) const AUTHZ_MODULE: &str = "authz";
/// The file of the Cedar actions, in the server output.
pub(crate) const CEDAR_ACTIONS_FILE: &str = "authz.cedarschema";

/// The files of the authorization vocabulary, keyed by path relative to the server output, and
/// the base paths of the services that got a `<service>/authz.rs`.
pub(super) fn generate(
    plan: &GenerationPlan,
    metadata: &CodeGenMetadata,
    config: &CodeGenConfig,
    authz: &AuthzConfig,
) -> Result<(HashMap<String, String>, Vec<String>)> {
    let operations = crate::analysis::plan_authz(plan, metadata)?;
    let label_path: syn::Path = syn::parse_str(
        &config
            .authz_label_path()
            .expect("validated: authz has an ObjectLabel path"),
    )
    .expect("validated: a Rust path");
    let module_path: syn::Path =
        syn::parse_str(&authz.module_path).expect("validated: a Rust path");

    let mut files = HashMap::new();
    files.insert(
        format!("{AUTHZ_MODULE}.rs"),
        root_module(plan, metadata, &operations, &label_path)?,
    );

    let mut services_with_impls = Vec::new();
    for handler in ServiceHandler::for_plan(plan, metadata, config) {
        let service = handler.plan;
        let ops: Vec<&AuthzOperation> = operations
            .iter()
            .filter(|op| op.service_base_path == service.base_path && op.emit_impl)
            .collect();
        if ops.is_empty() {
            continue;
        }
        files.insert(
            format!("{}/{AUTHZ_MODULE}.rs", service.base_path),
            service_module(&handler, &ops, &module_path, &label_path, metadata)?,
        );
        services_with_impls.push(service.base_path.clone());
    }

    files.insert(
        CEDAR_ACTIONS_FILE.to_string(),
        cedar_actions(&operations, plan, metadata, authz),
    );
    Ok((files, services_with_impls))
}

/// The root `authz.rs`.
fn root_module(
    plan: &GenerationPlan,
    metadata: &CodeGenMetadata,
    operations: &[AuthzOperation],
    label_path: &syn::Path,
) -> Result<String> {
    let docs: HashMap<&str, Option<&str>> = plan
        .services
        .iter()
        .flat_map(|s| &s.methods)
        .map(|m| {
            (
                m.metadata.method_name.as_str(),
                m.metadata.documentation.as_deref(),
            )
        })
        .collect();

    let variants: Vec<TokenStream> = operations
        .iter()
        .map(|op| {
            let ident = format_ident!("{}", op.name);
            // The RPC's summary, its first line: the rest (lists, notes on the wire) belongs
            // to the handler method, and a list cut off from its indentation would not lint.
            let summary = docs
                .get(op.name.as_str())
                .copied()
                .flatten()
                .and_then(|doc| doc.lines().map(str::trim).find(|l| !l.is_empty()));
            let doc = doc_tokens(summary);
            quote! { #doc #ident, }
        })
        .collect();
    let idents: Vec<_> = operations
        .iter()
        .map(|op| format_ident!("{}", op.name))
        .collect();
    let names: Vec<&str> = operations.iter().map(|op| op.name.as_str()).collect();
    let kinds: Vec<TokenStream> = operations
        .iter()
        .map(|op| match &op.securable.kind {
            SecurableKind::Root => quote! { SecurableKind::Root },
            SecurableKind::Resource(label) => {
                let label = format_ident!("{}", label);
                quote! { SecurableKind::Resource(ObjectLabel::#label) }
            }
            SecurableKind::Any => quote! { SecurableKind::Any },
        })
        .collect();
    let known_labels = crate::analysis::object_labels(plan, metadata);

    // The `Get` RPC of each resource, for list filtering. A label managed by several services
    // keeps the first in plan order.
    let mut gets: BTreeMap<String, String> = BTreeMap::new();
    for service in &plan.services {
        let Some(resource) = service.managed_resources.first() else {
            continue;
        };
        let Some((_, label)) = resource.descriptor.r#type.rsplit_once('/') else {
            continue;
        };
        if !known_labels.iter().any(|l| l == label) {
            continue;
        }
        if let Some(get) = service
            .methods
            .iter()
            .find(|m| m.request_type == RequestType::Get)
        {
            gets.entry(label.to_string())
                .or_insert_with(|| get.metadata.method_name.clone());
        }
    }
    let get_arms: Vec<TokenStream> = gets
        .iter()
        .map(|(label, get)| {
            let label = format_ident!("{}", label);
            let get = format_ident!("{}", get);
            quote! { ObjectLabel::#label => Some(Operation::#get), }
        })
        .collect();

    let tokens = quote! {
        //! The authorization vocabulary of the API.
        //!
        //! Every RPC is an [`Operation`], named after it, and every request an [`Authorizable`]
        //! that names its operation and the [`Securable`] it acts on. A policy decides whether a
        //! caller may perform an operation on a securable; the Cedar actions of the operations
        //! are in the sibling `authz.cedarschema`.

        use #label_path;

        /// An operation of the API: one per RPC, named after it.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum Operation {
            #(#variants)*
        }

        impl Operation {
            /// Every operation, in declaration order.
            pub const ALL: &'static [Operation] = &[#(Operation::#idents),*];

            /// The name of the operation: the RPC name, which is also its Cedar action id.
            pub const fn as_str(&self) -> &'static str {
                match self {
                    #(Operation::#idents => #names,)*
                }
            }

            /// The kind of securable the operation is asked on.
            pub const fn securable_kind(&self) -> SecurableKind {
                match self {
                    #(Operation::#idents => #kinds,)*
                }
            }

            /// The operation that reads one resource of `label` (the `Get` RPC of its service),
            /// if there is one. List results can be filtered with it.
            #[allow(unreachable_patterns)]
            pub const fn get_of(label: ObjectLabel) -> Option<Operation> {
                match label {
                    #(#get_arms)*
                    _ => None,
                }
            }
        }

        impl ::std::fmt::Display for Operation {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl ::std::str::FromStr for Operation {
            type Err = UnknownOperation;

            fn from_str(s: &str) -> ::std::result::Result<Self, Self::Err> {
                match s {
                    #(#names => Ok(Operation::#idents),)*
                    other => Err(UnknownOperation(other.to_string())),
                }
            }
        }

        /// The kind of resource an [`Operation`] acts on.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum SecurableKind {
            /// The service root, e.g. for a list of top-level resources.
            Root,
            /// A resource of this kind.
            Resource(ObjectLabel),
            /// Any resource, as the request decides at run time (e.g. a request naming the type
            /// of the resource it acts on).
            Any,
        }

        /// The error of parsing a name that is not an [`Operation`].
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct UnknownOperation(pub String);

        impl ::std::fmt::Display for UnknownOperation {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                write!(f, "unknown operation `{}`", self.0)
            }
        }

        impl ::std::error::Error for UnknownOperation {}

        /// The resource a request acts on.
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct Securable {
            /// The kind of resource, or `None` for the service root.
            pub label: Option<ObjectLabel>,
            /// The parts of the resource's name, outermost first (e.g. a parent's name, then
            /// the resource's own). Empty for the root. A list names the container of the
            /// resources it lists; a create names the new resource.
            ///
            /// A request that leaves a name field empty gives an empty part, which names no
            /// resource: deny it rather than reading the shorter name as a container.
            pub name: Vec<String>,
        }

        impl Securable {
            /// The resource of kind `label` named `name`.
            pub fn new(label: ObjectLabel, name: Vec<String>) -> Self {
                Self { label: Some(label), name }
            }

            /// The service root.
            pub fn root() -> Self {
                Self { label: None, name: Vec::new() }
            }
        }

        /// A request that names its operation and the securable it acts on.
        pub trait Authorizable: Send + Sync {
            /// The operation the request performs.
            fn operation(&self) -> Operation;

            /// The resource the request acts on.
            fn securable(&self) -> Securable;
        }
    };
    format_tokens(tokens)
}

/// The `<service>/authz.rs` of one service.
fn service_module(
    handler: &ServiceHandler<'_>,
    operations: &[&AuthzOperation],
    module_path: &syn::Path,
    label_path: &syn::Path,
    metadata: &CodeGenMetadata,
) -> Result<String> {
    let models_path = handler.models_path();
    let foreign = handler.foreign_models_imports();
    let runtime = handler.config.runtime;
    let mut uses_label = false;
    let impls: Vec<TokenStream> = operations
        .iter()
        .map(|op| {
            let message = metadata.messages.get(&op.input_type).ok_or_else(|| {
                crate::Error::Build(format!(
                    "the request `{}` of `{}` is not in the descriptors",
                    op.input_type, op.name
                ))
            })?;
            let request = extract_type_ident(&message.name);
            let operation = format_ident!("{}", op.name);
            let parts: Vec<TokenStream> = op
                .securable
                .name_fields
                .iter()
                .map(|field| name_part(field, runtime))
                .collect();
            let label = match &op.securable.kind {
                SecurableKind::Resource(label) => {
                    uses_label = true;
                    let label = format_ident!("{}", label);
                    quote! { Some(ObjectLabel::#label) }
                }
                SecurableKind::Root => quote! { None },
                SecurableKind::Any => {
                    return Err(crate::Error::Build(format!(
                        "`{}` acts on any resource, so it has no generated impl",
                        op.name
                    )));
                }
            };
            let body = if parts.is_empty() {
                quote! { Securable { label: #label, name: Vec::new() } }
            } else {
                quote! {
                    let mut name = Vec::new();
                    #(#parts)*
                    Securable { label: #label, name }
                }
            };
            Ok(quote! {
                impl Authorizable for #request {
                    fn operation(&self) -> Operation {
                        Operation::#operation
                    }

                    fn securable(&self) -> Securable {
                        #body
                    }
                }
            })
        })
        .collect::<Result<_>>()?;

    let label_use = if uses_label {
        quote! { use #label_path; }
    } else {
        quote! {}
    };
    let tokens = quote! {
        //! [`Authorizable`] for the requests of this service.

        #![allow(unused_imports)]

        #label_use
        use #module_path::{Authorizable, Operation, Securable};
        use #models_path::*;
        #(#foreign)*

        #(#impls)*
    };
    format_tokens(tokens)
}

/// The statements that push the parts of `field` onto `name`.
///
/// A nested field is read through its singular message fields: directly under buffa (a
/// `MessageField` derefs to the default message when unset), and through the `Option` under
/// prost, where an unset message reads as the default one too.
fn name_part(field: &NameField, runtime: Runtime) -> TokenStream {
    let idents: Vec<syn::Ident> = field
        .path
        .iter()
        .map(|segment| crate::utils::field_ident(segment))
        .collect();
    let (leaf, through) = idents.split_last().expect("a name field has a path");
    let push = |value: TokenStream| match field.kind {
        NameFieldKind::String => quote! {
            name.extend(#value.split('.').map(str::to_string));
        },
        NameFieldKind::OptionalString => quote! {
            if let Some(value) = &#value {
                name.extend(value.split('.').map(str::to_string));
            }
        },
        NameFieldKind::Integer => quote! {
            name.push(#value.to_string());
        },
        NameFieldKind::OptionalInteger => quote! {
            if let Some(value) = #value {
                name.push(value.to_string());
            }
        },
    };
    if through.is_empty() || runtime == Runtime::Buffa {
        return push(quote! { self.#(#through.)*#leaf });
    }
    // prost: a singular message field is an `Option`. Read an unset one as the default message,
    // as buffa does, so a request missing it gets the same (empty) name parts.
    let first = &through[0];
    let mut chain = quote! { self.#first.as_ref() };
    for segment in &through[1..] {
        chain = quote! { #chain.and_then(|message| message.#segment.as_ref()) };
    }
    match field.kind {
        NameFieldKind::String => {
            push(quote! { #chain.map_or("", |message| message.#leaf.as_str()) })
        }
        NameFieldKind::OptionalString => {
            push(quote! { #chain.and_then(|message| message.#leaf.as_deref()) })
        }
        NameFieldKind::Integer => push(quote! { #chain.map_or(0, |message| message.#leaf) }),
        NameFieldKind::OptionalInteger => push(quote! { #chain.and_then(|message| message.#leaf) }),
    }
}

/// The Cedar schema fragment of the actions.
fn cedar_actions(
    operations: &[AuthzOperation],
    plan: &GenerationPlan,
    metadata: &CodeGenMetadata,
    authz: &AuthzConfig,
) -> String {
    let known_labels = crate::analysis::object_labels(plan, metadata);
    let docs: HashMap<&str, Option<&str>> = plan
        .services
        .iter()
        .flat_map(|s| &s.methods)
        .map(|m| {
            (
                m.metadata.method_name.as_str(),
                m.metadata.documentation.as_deref(),
            )
        })
        .collect();
    let principals = authz.principal_types.join(", ");

    let mut out = String::new();
    out.push_str(
        "// The actions of the API: one per operation (RPC), on the entity type of the resource\n\
         // its request acts on. The entity types are named after the resource kinds and are\n\
         // declared alongside, in this namespace.\n",
    );
    let _ = writeln!(out, "namespace {} {{", authz.cedar_namespace);
    for (i, op) in operations.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if let Some(doc) = docs.get(op.name.as_str()).copied().flatten() {
            for line in doc.lines().map(str::trim).take_while(|l| !l.is_empty()) {
                let _ = writeln!(out, "  // {line}");
            }
        }
        let resource = match &op.securable.kind {
            SecurableKind::Root => authz.root_entity.clone(),
            SecurableKind::Resource(label) => label.clone(),
            SecurableKind::Any => std::iter::once(authz.root_entity.as_str())
                .chain(known_labels.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(", "),
        };
        let _ = writeln!(out, "  action \"{}\" appliesTo {{", op.name);
        let _ = writeln!(out, "    principal: [{principals}],");
        let _ = writeln!(out, "    resource: [{resource}],");
        if let Some(context) = &authz.context_type {
            let _ = writeln!(out, "    context: {context},");
        }
        out.push_str("  };\n");
    }
    out.push_str("}\n");
    out
}
