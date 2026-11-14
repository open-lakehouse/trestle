use std::collections::HashMap;
use std::str::FromStr;

use async_trait::async_trait;
use cedar_policy::{
    Context, Entities, EntityId, EntityTypeName, EntityUid, PolicyId, PolicySet, Request, Schema,
    ValidationMode, Validator,
};
use serde_json::{Map, Value};

use super::Authorizer;
use super::wire::{AuthorizeRequest, AuthorizeResponse, Decision, EntityRef, entity_ref};
use crate::error::BoxError;

/// Supplies the Cedar entities a [`CedarAuthorizer`] evaluates a request against,
/// implemented by the host against wherever its users, groups and grants live.
#[async_trait]
pub trait EntityLoader: Send + Sync + 'static {
    /// The entities `request` needs, as a Cedar JSON entities array: at least the
    /// principal with its attributes and its ancestors (groups, roles), and whatever the
    /// caller doesn't supply in [`entities`](AuthorizeRequest::entities).
    ///
    /// The caller-supplied entities are merged in afterwards: an entity both loaded and
    /// supplied gets the attributes of both (the supplied value wins for an attribute in
    /// both) and the parents of both. So a loader may return just the attributes it owns
    /// for a supplied entity, such as the grants the principal holds on it, with no
    /// parents.
    ///
    /// An error is a deny.
    async fn load(&self, request: &AuthorizeRequest) -> Result<Value, BoxError>;

    /// The entities of several requests, one result per request in the same order. By
    /// default one at a time; override it to share lookups across a batch.
    async fn load_batch(&self, requests: &[AuthorizeRequest]) -> Vec<Result<Value, BoxError>> {
        let mut loaded = Vec::with_capacity(requests.len());
        for request in requests {
            loaded.push(self.load(request).await);
        }
        loaded
    }
}

/// The attributes only a [`CedarAuthorizer`]'s [`EntityLoader`] may set unless configured
/// otherwise: the grants a principal holds on an entity, as
/// [`GrantsEntityLoader`](crate::GrantsEntityLoader) loads them.
pub const DEFAULT_LOADER_ATTRS: &[&str] = &["privileges", "inherited_privileges"];

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
/// The entities a request is evaluated against are the loaded ones merged with the
/// caller-supplied [`entities`](AuthorizeRequest::entities) (see [`EntityLoader::load`]).
/// The caller may not supply the principal, nor set an attribute the loader owns
/// ([`with_loader_attrs`](Self::with_loader_attrs)), so it can't forge grants.
///
/// It fails closed: if the entities can't be loaded, the caller-supplied ones break the
/// rules above, the resource is neither loaded nor supplied, or the request, context or
/// entities don't match the schema, the answer is a deny with the reason in
/// [`errors`](AuthorizeResponse::errors).
pub struct CedarAuthorizer {
    policies: PolicySet,
    schema: Schema,
    loader: Box<dyn EntityLoader>,
    loader_attrs: Vec<String>,
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
            loader_attrs: DEFAULT_LOADER_ATTRS.iter().map(|&a| a.to_owned()).collect(),
            engine: cedar_policy::Authorizer::new(),
        })
    }

    /// The attributes only the loader may set, instead of [`DEFAULT_LOADER_ATTRS`]. A
    /// request whose caller-supplied entities set one is denied.
    pub fn with_loader_attrs<I, S>(mut self, attrs: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.loader_attrs = attrs.into_iter().map(Into::into).collect();
        self
    }

    async fn decide(
        &self,
        request: &AuthorizeRequest,
        loaded: Result<Value, BoxError>,
    ) -> Result<AuthorizeResponse, String> {
        let principal = uid(&request.principal)?;
        let action = uid(&request.action)?;
        let resource = uid(&request.resource)?;

        let loaded = match loaded {
            Ok(Value::Array(entities)) => entities,
            Ok(_) => return Err("the entity loader did not return an array".into()),
            Err(e) => return Err(format!("loading entities: {e}")),
        };
        let entities = merge(loaded, request, &self.loader_attrs)?;
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

/// The loaded entities with the caller-supplied ones merged in (see
/// [`EntityLoader::load`]), checking the rules [`CedarAuthorizer`] enforces on them.
fn merge(
    loaded: Vec<Value>,
    request: &AuthorizeRequest,
    loader_attrs: &[String],
) -> Result<Vec<Value>, String> {
    let mut entities = Vec::with_capacity(loaded.len() + request.entities.len());
    let mut index: HashMap<EntityRef, usize> = HashMap::new();
    for entity in loaded {
        let uid = entity
            .get("uid")
            .and_then(entity_ref)
            .ok_or("the entity loader returned an entity without a uid")?;
        match index.get(&uid) {
            Some(&at) => absorb(&mut entities[at], &entity),
            None => {
                index.insert(uid, entities.len());
                entities.push(entity);
            }
        }
    }
    for entity in &request.entities {
        let uid = entity
            .get("uid")
            .and_then(entity_ref)
            .ok_or("a supplied entity has no uid")?;
        if uid == request.principal {
            return Err(format!("the caller may not supply the principal {uid}"));
        }
        if let Some(attrs) = entity.get("attrs").and_then(Value::as_object)
            && let Some(attr) = loader_attrs.iter().find(|a| attrs.contains_key(a.as_str()))
        {
            return Err(format!(
                "the supplied entity {uid} sets {attr:?}, which only the entity loader may set"
            ));
        }
        match index.get(&uid) {
            Some(&at) => absorb(&mut entities[at], entity),
            None => {
                index.insert(uid, entities.len());
                entities.push(entity.clone());
            }
        }
    }
    if !index.contains_key(&request.resource) {
        return Err(format!(
            "the resource {} is neither loaded nor supplied",
            request.resource
        ));
    }
    Ok(entities)
}

/// Merges `other` into `entity`: its attributes win, and its parents are added.
fn absorb(entity: &mut Value, other: &Value) {
    let Some(entity) = entity.as_object_mut() else {
        return;
    };
    if let Some(attrs) = other.get("attrs").and_then(Value::as_object) {
        let target = entity
            .entry("attrs")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Some(target) = target.as_object_mut() {
            target.extend(attrs.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
    }
    if let Some(parents) = other.get("parents").and_then(Value::as_array) {
        let target = entity
            .entry("parents")
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Some(target) = target.as_array_mut() {
            for parent in parents {
                let known = entity_ref(parent);
                if !target.iter().any(|p| entity_ref(p) == known) {
                    target.push(parent.clone());
                }
            }
        }
    }
}

fn fail_closed(result: Result<AuthorizeResponse, String>) -> AuthorizeResponse {
    result.unwrap_or_else(|error| {
        tracing::warn!(%error, "authorization failed closed");
        AuthorizeResponse::deny(error)
    })
}

#[async_trait]
impl Authorizer for CedarAuthorizer {
    async fn authorize(&self, request: &AuthorizeRequest) -> AuthorizeResponse {
        let loaded = self.loader.load(request).await;
        fail_closed(self.decide(request, loaded).await)
    }

    /// Loads the entities of the whole batch at once ([`EntityLoader::load_batch`]).
    async fn authorize_batch(&self, requests: &[AuthorizeRequest]) -> Vec<AuthorizeResponse> {
        let mut loaded = self.loader.load_batch(requests).await.into_iter();
        let mut responses = Vec::with_capacity(requests.len());
        for request in requests {
            let entities = loaded
                .next()
                .unwrap_or_else(|| Err("the entity loader answered fewer requests".into()));
            responses.push(fail_closed(self.decide(request, entities).await));
        }
        responses
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn request() -> AuthorizeRequest {
        AuthorizeRequest::new(
            EntityRef::new("Ns::User", "u1"),
            EntityRef::new("Ns::Action", "read"),
            EntityRef::new("Ns::Doc", "d1"),
        )
    }

    fn attrs() -> Vec<String> {
        DEFAULT_LOADER_ATTRS.iter().map(|&a| a.to_owned()).collect()
    }

    #[test]
    fn merges_supplied_entities_into_loaded_ones() {
        let loaded = vec![
            json!({ "uid": { "type": "Ns::User", "id": "u1" }, "attrs": {}, "parents": [] }),
            json!({
                "uid": { "type": "Ns::Doc", "id": "d1" },
                "attrs": { "privileges": ["READ"], "title": "loaded" },
                "parents": [],
            }),
        ];
        let request = request().with_entities([
            json!({
                "uid": { "__entity": { "type": "Ns::Doc", "id": "d1" } },
                "attrs": { "title": "supplied" },
                "parents": [{ "type": "Ns::Folder", "id": "f1" }],
            }),
            json!({ "uid": { "type": "Ns::Folder", "id": "f1" }, "attrs": {}, "parents": [] }),
        ]);
        let merged = merge(loaded, &request, &attrs()).unwrap();
        assert_eq!(merged.len(), 3);
        assert_eq!(
            merged[1],
            json!({
                "uid": { "type": "Ns::Doc", "id": "d1" },
                "attrs": { "privileges": ["READ"], "title": "supplied" },
                "parents": [{ "type": "Ns::Folder", "id": "f1" }],
            })
        );
    }

    #[test]
    fn supplied_entities_may_not_forge_grants_or_the_principal() {
        let forged = request().with_entity(json!({
            "uid": { "type": "Ns::Doc", "id": "d1" },
            "attrs": { "inherited_privileges": ["ALL"] },
            "parents": [],
        }));
        let error = merge(vec![], &forged, &attrs()).unwrap_err();
        assert!(error.contains("only the entity loader may set"), "{error}");

        let principal = request().with_entity(json!({
            "uid": { "type": "Ns::User", "id": "u1" }, "attrs": { "admin": true }, "parents": [],
        }));
        let error = merge(vec![], &principal, &attrs()).unwrap_err();
        assert!(error.contains("may not supply the principal"), "{error}");

        // Other attribute names are the caller's to set when so configured.
        let custom = vec!["owner".to_string()];
        let ok = request().with_entity(json!({
            "uid": { "type": "Ns::Doc", "id": "d1" }, "attrs": { "privileges": [] }, "parents": [],
        }));
        assert!(merge(vec![], &ok, &custom).is_ok());
    }

    #[test]
    fn the_resource_must_be_loaded_or_supplied() {
        let error = merge(vec![], &request(), &attrs()).unwrap_err();
        assert!(error.contains("neither loaded nor supplied"), "{error}");
        let error = merge(vec![json!({ "attrs": {} })], &request(), &attrs()).unwrap_err();
        assert!(error.contains("without a uid"), "{error}");
    }
}
