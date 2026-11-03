use std::str::FromStr;

use async_trait::async_trait;
use cedar_policy::{
    Context, Entities, EntityId, EntityTypeName, EntityUid, PolicyId, PolicySet, Request, Schema,
    ValidationMode, Validator,
};
use serde_json::Value;

use super::Authorizer;
use super::wire::{AuthorizeRequest, AuthorizeResponse, Decision, EntityRef};
use crate::error::BoxError;

/// Supplies the Cedar entities a [`CedarAuthorizer`] evaluates a request against,
/// implemented by the host against wherever its users, groups and grants live.
#[async_trait]
pub trait EntityLoader: Send + Sync + 'static {
    /// The entities `request` needs, as a Cedar JSON entities array: at least the
    /// principal with its attributes and its ancestors (groups, roles), and the
    /// resource's ancestors. A caller-supplied
    /// [`resource_entity`](AuthorizeRequest::resource_entity) is added afterwards and
    /// replaces a loaded entity with the same uid.
    ///
    /// An error is a deny.
    async fn load(&self, request: &AuthorizeRequest) -> Result<Value, BoxError>;
}

/// Policies or a schema [`CedarAuthorizer::new`] refuses.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CedarConfigError {
    /// The schema doesn't parse.
    #[error("invalid Cedar schema: {0}")]
    Schema(String),
    /// The policies don't parse.
    #[error("invalid Cedar policies: {0}")]
    Policies(String),
    /// The policies contain templates, which are not supported.
    #[error("Cedar policy templates are not supported")]
    Templates,
    /// Two policies have the same `@id`.
    #[error("more than one Cedar policy with id {0}")]
    DuplicateId(String),
    /// The policies don't validate against the schema in strict mode.
    #[error("Cedar policies don't validate against the schema: {0}")]
    Validation(String),
}

/// An [`Authorizer`] that evaluates Cedar policies in-process.
///
/// For standalone deployments, and as a fallback where a round trip to a remote policy
/// decision point costs too much. The policies are validated against the schema in
/// strict mode once, at construction; the request and its entities and context are
/// validated against the schema on every request.
///
/// A policy's id is its `@id("…")` annotation, else Cedar's positional `policyN`, so
/// [`policies`](AuthorizeResponse::policies) names the determining policies stably.
///
/// It fails closed: if the entities can't be loaded, or the request, context or
/// entities don't match the schema, the answer is a deny with the reason in
/// [`errors`](AuthorizeResponse::errors).
pub struct CedarAuthorizer {
    policies: PolicySet,
    schema: Schema,
    loader: Box<dyn EntityLoader>,
    engine: cedar_policy::Authorizer,
}

impl CedarAuthorizer {
    /// Evaluates `policies` (Cedar policy syntax) against the entities `loader`
    /// supplies, validating everything against `schema` (Cedar schema syntax).
    ///
    /// # Errors
    ///
    /// When the schema or the policies don't parse, contain templates or duplicate
    /// `@id`s, or the policies don't validate against the schema.
    pub fn new(
        schema: &str,
        policies: &str,
        loader: impl EntityLoader,
    ) -> Result<Self, CedarConfigError> {
        let (schema, warnings) = Schema::from_cedarschema_str(schema)
            .map_err(|e| CedarConfigError::Schema(e.to_string()))?;
        for warning in warnings {
            tracing::debug!(%warning, "Cedar schema warning");
        }
        let parsed =
            PolicySet::from_str(policies).map_err(|e| CedarConfigError::Policies(e.to_string()))?;
        if parsed.templates().next().is_some() {
            return Err(CedarConfigError::Templates);
        }
        let mut policies = PolicySet::new();
        for policy in parsed.policies() {
            let id = policy
                .annotation("id")
                .map_or_else(|| policy.id().to_string(), str::to_owned);
            policies
                .add(policy.new_id(PolicyId::new(&id)))
                .map_err(|_| CedarConfigError::DuplicateId(id))?;
        }
        let validation = Validator::new(schema.clone()).validate(&policies, ValidationMode::Strict);
        if !validation.validation_passed() {
            let errors: Vec<String> = validation
                .validation_errors()
                .map(ToString::to_string)
                .collect();
            return Err(CedarConfigError::Validation(errors.join("; ")));
        }
        Ok(Self {
            policies,
            schema,
            loader: Box::new(loader),
            engine: cedar_policy::Authorizer::new(),
        })
    }

    async fn decide(&self, request: &AuthorizeRequest) -> Result<AuthorizeResponse, String> {
        let principal = uid(&request.principal)?;
        let action = uid(&request.action)?;
        let resource = uid(&request.resource)?;

        let mut entities = match self.loader.load(request).await {
            Ok(Value::Array(entities)) => entities,
            Ok(_) => return Err("the entity loader did not return an array".into()),
            Err(e) => return Err(format!("loading entities: {e}")),
        };
        if let Some(entity) = &request.resource_entity {
            let resource_uid = serde_json::to_value(&request.resource)
                .map_err(|e| format!("encoding the resource: {e}"))?;
            if entity.get("uid") != Some(&resource_uid) {
                return Err("resource_entity is not the request's resource".into());
            }
            entities.retain(|loaded| loaded.get("uid") != Some(&resource_uid));
            entities.push(entity.clone());
        }
        let entities = Entities::from_json_value(Value::Array(entities), Some(&self.schema))
            .map_err(|e| format!("invalid entities: {e}"))?;
        let context = Context::from_json_value(
            Value::Object(request.context.clone()),
            Some((&self.schema, &action)),
        )
        .map_err(|e| format!("invalid context: {e}"))?;
        let cedar_request = Request::new(principal, action, resource, context, Some(&self.schema))
            .map_err(|e| format!("invalid request: {e}"))?;

        let response = self
            .engine
            .is_authorized(&cedar_request, &self.policies, &entities);
        let mut policies: Vec<String> = response
            .diagnostics()
            .reason()
            .map(ToString::to_string)
            .collect();
        policies.sort();
        Ok(AuthorizeResponse {
            decision: match response.decision() {
                cedar_policy::Decision::Allow => Decision::Allow,
                cedar_policy::Decision::Deny => Decision::Deny,
            },
            policies,
            errors: response
                .diagnostics()
                .errors()
                .map(ToString::to_string)
                .collect(),
        })
    }
}

fn uid(entity: &EntityRef) -> Result<EntityUid, String> {
    let entity_type = EntityTypeName::from_str(&entity.entity_type)
        .map_err(|e| format!("invalid entity type {:?}: {e}", entity.entity_type))?;
    Ok(EntityUid::from_type_name_and_id(
        entity_type,
        EntityId::new(&entity.id),
    ))
}

#[async_trait]
impl Authorizer for CedarAuthorizer {
    async fn authorize(&self, request: &AuthorizeRequest) -> AuthorizeResponse {
        self.decide(request).await.unwrap_or_else(|error| {
            tracing::warn!(%error, "authorization failed closed");
            AuthorizeResponse::deny(error)
        })
    }
}
