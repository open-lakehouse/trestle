//! Authorization planning: the operation and securable of every RPC.
//!
//! Each RPC becomes an operation, the action a policy decides on, named after the RPC. Its
//! request carries a securable, the resource the operation acts on, which a policy checks the
//! caller against. This module works out, for every method, which kind of resource that is and
//! which request fields make up its name, so the emitters can generate an `Authorizable` impl and
//! a Cedar action per RPC.
//!
//! The securable of a request is, in order of precedence:
//!
//! 1. the `(olai.trestle.v1.authz)` method option, when present;
//! 2. the one request field with a `google.api.resource_reference { type }`, which names that
//!    resource;
//! 3. on a service that manages a resource:
//!    - a `List` names the container of the listed resources, from the resource's parent fields
//!      present on the request (the root when it has none);
//!    - a `Create` names the new resource, from its parent fields and its own name field;
//!    - any other method names the managed resource, from the method's path parameters.
//!
//! A method that matches none of these (a resource-less service, a collection-style custom
//! method, a create without its parent fields) is an error, so that no request is left without
//! a securable by accident. [`plan_authz`] reports every such method at once.

use std::collections::BTreeMap;

use crate::analysis::{GenerationPlan, MethodPlan, MethodShape, RequestType, ServicePlan};
use crate::error::{Error, Result};
use crate::parsing::types::BaseType;
use crate::parsing::{AuthzRule, CodeGenMetadata, MessageField};

/// The operation and securable of one RPC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthzOperation {
    /// The operation's name, which is the RPC name (e.g. `GetTable`). It is unique across the
    /// plan and doubles as the Cedar action id.
    pub name: String,
    /// The base path of the service the RPC belongs to (e.g. `tables`).
    pub service_base_path: String,
    /// The proto package of that service (e.g. `example.tables.v1`).
    pub package: String,
    /// The fully qualified request message (e.g. `.example.tables.v1.GetTableRequest`).
    pub input_type: String,
    /// The securable the request acts on.
    pub securable: SecurablePlan,
    /// Whether to emit the request's `Authorizable` impl (`false` when the method opts out with
    /// `skip`).
    pub emit_impl: bool,
}

/// The securable of a request: the kind of resource and where its name comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurablePlan {
    /// The kind of resource.
    pub kind: SecurableKind,
    /// The request fields that, in order and each split on `.`, make up the name. Empty for the
    /// root, and for a method that opts out with `skip`.
    pub name_fields: Vec<NameField>,
}

/// The kind of resource an operation acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecurableKind {
    /// The service root (e.g. a list of top-level resources).
    Root,
    /// A resource, by its label: the last segment of its `google.api.resource` type (e.g.
    /// `Table` for `example.io/Table`), which is its `ObjectLabel` variant.
    Resource(String),
    /// Any resource, decided by the request at run time. Only a method that opts out with
    /// `skip` and names no `securable_type` has it.
    Any,
}

impl SecurablePlan {
    fn root() -> Self {
        Self {
            kind: SecurableKind::Root,
            name_fields: vec![],
        }
    }

    fn resource(label: String, name_fields: Vec<NameField>) -> Self {
        Self {
            kind: SecurableKind::Resource(label),
            name_fields,
        }
    }
}

/// A request field that contributes to a securable's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameField {
    /// The field, as the path of field names from the request (e.g. `["full_name"]`, or
    /// `["info", "catalog_name"]` for a field of a singular message field `info`).
    pub path: Vec<String>,
    /// How the (last) field is read.
    pub kind: NameFieldKind,
}

/// The Rust shape of a [`NameField`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameFieldKind {
    /// A `String`, split on `.`.
    String,
    /// An `Option<String>`, split on `.` when set.
    OptionalString,
    /// An integer, as one part.
    Integer,
    /// An `Option` of an integer, as one part when set.
    OptionalInteger,
}

/// Plan the operation and securable of every method of `plan`.
///
/// Operations come in service order, then method order.
///
/// # Errors
///
/// Returns [`Error::Build`] listing every RPC whose name is not unique across the plan, and
/// every method whose securable can't be found (see the [module docs](self)).
pub fn plan_authz(
    plan: &GenerationPlan,
    metadata: &CodeGenMetadata,
) -> Result<Vec<AuthzOperation>> {
    let prefix = label_prefix(plan);
    let mut operations = Vec::new();
    let mut failures = Vec::new();
    for service in &plan.services {
        for method in &service.methods {
            let name = method.metadata.method_name.clone();
            match plan_method(service, method, &prefix, metadata) {
                Ok((securable, emit_impl)) => operations.push(AuthzOperation {
                    name,
                    service_base_path: service.base_path.clone(),
                    package: service.package.clone(),
                    input_type: method.metadata.input_type.clone(),
                    securable,
                    emit_impl,
                }),
                Err(reason) => failures.push(format!("{}.{name}: {reason}", service.service_name)),
            }
        }
    }

    let mut by_name: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for op in &operations {
        by_name.entry(&op.name).or_default().push(&op.package);
    }
    let duplicates: Vec<String> = by_name
        .iter()
        .filter(|(_, packages)| packages.len() > 1)
        .map(|(name, packages)| format!("`{name}` (in {})", packages.join(", ")))
        .collect();
    if !duplicates.is_empty() {
        return Err(Error::Build(format!(
            "authz operations are named after their RPC and must be unique; rename the RPCs \
             that share a name: {}",
            duplicates.join("; ")
        )));
    }

    // One request type, one impl: two RPCs taking the same message can't both get one.
    let mut by_input: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for op in operations.iter().filter(|op| op.emit_impl) {
        by_input.entry(&op.input_type).or_default().push(&op.name);
    }
    for (input, names) in by_input.iter().filter(|(_, names)| names.len() > 1) {
        failures.push(format!(
            "{}: the request `{input}` is shared, so only one of them can name its securable; \
             set `(olai.trestle.v1.authz) = {{skip: true}}` on the others",
            names.join(", ")
        ));
    }

    if !failures.is_empty() {
        return Err(Error::Build(format!(
            "can't find the securable of {} request(s); annotate the field naming the resource \
             with `google.api.resource_reference`, or set `(olai.trestle.v1.authz)` on the \
             method:\n  {}",
            failures.len(),
            failures.join("\n  ")
        )));
    }
    Ok(operations)
}

/// The securable of one method and whether to emit its impl, or why it can't be found.
fn plan_method(
    service: &ServicePlan,
    method: &MethodPlan,
    prefix: &str,
    metadata: &CodeGenMetadata,
) -> std::result::Result<(SecurablePlan, bool), String> {
    let fields = metadata.get_message_fields(&method.metadata.input_type);
    let skipped = method.metadata.authz.as_ref().is_some_and(|r| r.skip);
    if !skipped && !metadata.messages.contains_key(&method.metadata.input_type) {
        return Err(format!(
            "the request `{}` is not a message of the API (set `skip`)",
            method.metadata.input_type
        ));
    }
    if let Some(rule) = &method.metadata.authz {
        return from_rule(rule, &fields, prefix, metadata).map(|s| (s, !rule.skip));
    }

    // The service's managed resource decides first; a field referencing a resource type is the
    // fallback, for resource-less services and methods the resource can't explain.
    let plan = match from_service(service, method, &fields, prefix, metadata) {
        Ok(plan) => plan,
        Err(reason) => from_reference(&fields, prefix, metadata)?.ok_or(reason)?,
    };
    check_parent_fields(service, &fields, &plan)?;
    Ok((plan, true))
}

/// The securable of a request of a service that manages a resource.
fn from_service(
    service: &ServicePlan,
    method: &MethodPlan,
    fields: &[MessageField],
    prefix: &str,
    metadata: &CodeGenMetadata,
) -> std::result::Result<SecurablePlan, String> {
    let Some(resource) = service.managed_resources.first() else {
        return Err("the service manages no resource".to_string());
    };
    let resource_type = &resource.descriptor.r#type;
    let parents = crate::analysis::derive_accessor_parent_fields(service);
    let field = |name: &str| fields.iter().find(|f| f.name == name);

    // A collection-style custom method (a `List*` GET, a factory POST) acts on the collection,
    // like a list.
    let on_collection = method.request_type == RequestType::List
        || (matches!(method.request_type, RequestType::Custom(_))
            && method.shape == MethodShape::CollectionScoped);
    if on_collection {
        let present: Vec<&MessageField> = parents.iter().map_while(|p| field(p)).collect();
        let Some(last) = present.last() else {
            if parents.is_empty() {
                return Ok(SecurablePlan::root());
            }
            return Err(format!(
                "a list of a nested resource without its parent field `{}`",
                parents[0]
            ));
        };
        let container = parent_type(service, &last.name, metadata)
            .ok_or_else(|| format!("can't tell the resource type of `{}`", last.name))?;
        let name_fields = present
            .into_iter()
            .map(name_field)
            .collect::<std::result::Result<_, _>>()?;
        return Ok(SecurablePlan::resource(
            known_label(&container, prefix, metadata)?,
            name_fields,
        ));
    }

    let label = known_label(resource_type, prefix, metadata)?;
    if method.request_type == RequestType::Create {
        let mut name_fields = Vec::with_capacity(parents.len() + 1);
        for parent in &parents {
            let f = field(parent)
                .ok_or_else(|| format!("a create without the parent field `{parent}`"))?;
            name_fields.push(name_field(f)?);
        }
        let leaf = format!("{}_name", resource.descriptor.singular);
        let own = field("name")
            .or_else(|| field(&leaf))
            .ok_or_else(|| format!("a create without a `name` or `{leaf}` field"))?;
        name_fields.push(name_field(own)?);
        return Ok(SecurablePlan::resource(label, name_fields));
    }

    let path: Vec<&MessageField> = method
        .path_parameters()
        .filter_map(|p| field(&p.name))
        .collect();
    if path.is_empty() {
        return Err("no path parameter names the resource".to_string());
    }
    let name_fields = path
        .into_iter()
        .map(name_field)
        .collect::<std::result::Result<_, _>>()?;
    Ok(SecurablePlan::resource(label, name_fields))
}

/// The securable named by the one request field with a `resource_reference { type }`, if there
/// is one.
fn from_reference(
    fields: &[MessageField],
    prefix: &str,
    metadata: &CodeGenMetadata,
) -> std::result::Result<Option<SecurablePlan>, String> {
    let typed: Vec<&MessageField> = fields
        .iter()
        .filter(|f| {
            f.resource_reference
                .as_ref()
                .is_some_and(|r| !r.r#type.is_empty())
        })
        .collect();
    match typed.as_slice() {
        [] => Ok(None),
        [field] => {
            let resource_type = &field.resource_reference.as_ref().expect("filtered").r#type;
            let label = known_label(resource_type, prefix, metadata)?;
            Ok(Some(SecurablePlan::resource(
                label,
                vec![name_field(field)?],
            )))
        }
        many => Err(format!(
            "several fields reference a resource type ({}); pick one",
            many.iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Refuse an inferred securable that leaves out a field annotated as a parent of the service's
/// resource (`resource_reference { child_type }`): the name would be missing its container.
fn check_parent_fields(
    service: &ServicePlan,
    fields: &[MessageField],
    plan: &SecurablePlan,
) -> std::result::Result<(), String> {
    let Some(resource) = service.managed_resources.first() else {
        return Ok(());
    };
    let named = |name: &str| plan.name_fields.iter().any(|f| f.path == [name]);
    let missing: Vec<&str> = fields
        .iter()
        .filter(|f| {
            f.resource_reference
                .as_ref()
                .is_some_and(|r| r.child_type == resource.descriptor.r#type)
        })
        .map(|f| f.name.as_str())
        .filter(|name| !named(name))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "the parent field(s) `{}` would not be part of the resource's name",
            missing.join("`, `")
        ))
    }
}

/// The securable an `(olai.trestle.v1.authz)` rule names.
fn from_rule(
    rule: &AuthzRule,
    fields: &[MessageField],
    prefix: &str,
    metadata: &CodeGenMetadata,
) -> std::result::Result<SecurablePlan, String> {
    let label = (!rule.securable_type.is_empty())
        .then(|| known_label(&rule.securable_type, prefix, metadata))
        .transpose()?;
    if rule.skip {
        // The server builds the securable itself; the fields are not read.
        if !rule.securable_fields.is_empty() {
            return Err("`securable_fields` with `skip`, which reads no field".to_string());
        }
        return Ok(SecurablePlan {
            kind: label.map_or(SecurableKind::Any, SecurableKind::Resource),
            name_fields: vec![],
        });
    }
    let name_fields: Vec<NameField> = rule
        .securable_fields
        .iter()
        .map(|path| nested_name_field(path, fields, metadata))
        .collect::<std::result::Result<_, _>>()?;
    match label {
        None if name_fields.is_empty() => Ok(SecurablePlan::root()),
        None => Err("`securable_fields` without a `securable_type`".to_string()),
        Some(_) if name_fields.is_empty() => {
            Err("a `securable_type` without `securable_fields`".to_string())
        }
        Some(label) => Ok(SecurablePlan::resource(label, name_fields)),
    }
}

/// The name field at the dotted `path` from the request's `fields`, through singular message
/// fields (e.g. `info.catalog_name`).
fn nested_name_field(
    path: &str,
    fields: &[MessageField],
    metadata: &CodeGenMetadata,
) -> std::result::Result<NameField, String> {
    let segments: Vec<&str> = path.split('.').collect();
    let (last, through) = segments.split_last().expect("split yields one segment");
    let mut fields = fields.to_vec();
    for segment in through {
        let field = fields
            .iter()
            .find(|f| f.name == *segment)
            .ok_or_else(|| format!("no field `{segment}` on the way to `{path}`"))?;
        let BaseType::Message(message) = &field.unified_type.base_type else {
            return Err(format!("`{segment}` of `{path}` is not a message field"));
        };
        if field.unified_type.is_repeated {
            return Err(format!("`{segment}` of `{path}` is repeated"));
        }
        fields = (metadata.messages.get(&format!(".{message}")))
            .or_else(|| metadata.messages.get(message))
            .map(|m| m.fields.clone())
            .ok_or_else(|| format!("the message `{message}` of `{path}` is unknown"))?;
    }
    let field = fields
        .iter()
        .find(|f| f.name == *last)
        .ok_or_else(|| format!("the request has no field `{path}`"))?;
    let mut name = name_field(field)?;
    name.path = segments.iter().map(|s| s.to_string()).collect();
    Ok(name)
}

/// The package prefix of the messages that get an `ObjectLabel` variant (e.g. `.example.`),
/// as the `labels.rs` generator infers it: the top-level segment of the first service package.
fn label_prefix(plan: &GenerationPlan) -> String {
    plan.services
        .first()
        .map(|s| format!(".{}.", s.package.split('.').next().unwrap_or_default()))
        .unwrap_or_default()
}

/// Every `ObjectLabel` variant, sorted: the label of each `google.api.resource` type the API
/// declares.
pub fn object_labels(plan: &GenerationPlan, metadata: &CodeGenMetadata) -> Vec<String> {
    let prefix = label_prefix(plan);
    let labels: std::collections::BTreeSet<String> = metadata
        .messages
        .iter()
        .filter(|(name, _)| name.starts_with(&prefix))
        .filter_map(|(_, m)| m.resource_descriptor.as_ref())
        .filter_map(|d| d.r#type.rsplit_once('/').map(|(_, l)| l.to_string()))
        .filter(|l| !l.is_empty())
        .collect();
    labels.into_iter().collect()
}

/// The label of a `google.api.resource` type the descriptors declare.
fn known_label(
    resource_type: &str,
    prefix: &str,
    metadata: &CodeGenMetadata,
) -> std::result::Result<String, String> {
    let declared = metadata.messages.iter().any(|(name, m)| {
        name.starts_with(prefix)
            && m.resource_descriptor
                .as_ref()
                .is_some_and(|d| d.r#type == resource_type)
    });
    if !declared {
        return Err(format!(
            "no message declares the resource type `{resource_type}`"
        ));
    }
    match resource_type.rsplit_once('/') {
        Some((_, label)) if !label.is_empty() => Ok(label.to_string()),
        _ => Err(format!(
            "the resource type `{resource_type}` has no `/`-separated name"
        )),
    }
}

/// The resource type of the parent field `name` of `service`'s resource.
fn parent_type(service: &ServicePlan, name: &str, metadata: &CodeGenMetadata) -> Option<String> {
    service
        .hierarchy
        .iter()
        .find(|h| h.parent_field_name == name)
        .map(|h| h.parent_resource_type.clone())
        .or_else(|| {
            name.strip_suffix("_name")
                .and_then(|singular| metadata.resource_from_singular(singular))
                .map(|d| d.r#type.clone())
        })
}

/// `field` as a name part, if it is a string or an integer.
fn name_field(field: &MessageField) -> std::result::Result<NameField, String> {
    let ty = &field.unified_type;
    if ty.is_repeated {
        return Err(format!("the name field `{}` is repeated", field.name));
    }
    let kind = match (&ty.base_type, ty.is_optional) {
        (BaseType::String, false) => NameFieldKind::String,
        (BaseType::String, true) => NameFieldKind::OptionalString,
        (BaseType::Int32 | BaseType::Int64, false) => NameFieldKind::Integer,
        (BaseType::Int32 | BaseType::Int64, true) => NameFieldKind::OptionalInteger,
        _ => {
            return Err(format!(
                "the name field `{}` is neither a string nor an integer",
                field.name
            ));
        }
    };
    Ok(NameField {
        path: vec![field.name.clone()],
        kind,
    })
}
