//! The JSON a policy decision point (PDP) speaks: `POST /v1/authorize` takes an
//! [`AuthorizeRequest`] and answers an [`AuthorizeResponse`]; `POST /v1/authorize/batch`
//! does the same for many at once.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The path of a PDP's single-request endpoint.
pub const AUTHORIZE_PATH: &str = "/v1/authorize";
/// The path of a PDP's batch endpoint.
pub const AUTHORIZE_BATCH_PATH: &str = "/v1/authorize/batch";

/// A Cedar entity uid: an entity type such as `Ns::User` and an id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EntityRef {
    /// The fully qualified entity type, e.g. `Ns::Resource`.
    #[serde(rename = "type")]
    pub entity_type: String,
    /// The entity id.
    pub id: String,
}

impl std::fmt::Display for EntityRef {
    /// Cedar's syntax, `Ns::User::"alice"`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}::{:?}", self.entity_type, self.id)
    }
}

impl EntityRef {
    /// The entity `id` of type `entity_type`.
    pub fn new(entity_type: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            entity_type: entity_type.into(),
            id: id.into(),
        }
    }
}

/// May `principal` perform `action` on `resource`?
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuthorizeRequest {
    /// Who is asking, e.g. from [`EntityTypes::principal`](crate::EntityTypes::principal).
    pub principal: EntityRef,
    /// The action, e.g. `Ns::Action::"read"`.
    pub action: EntityRef,
    /// What the action is on.
    pub resource: EntityRef,
    /// The Cedar request context, e.g. from [`AuthContext`](crate::AuthContext).
    #[serde(default)]
    pub context: Map<String, Value>,
    /// The entities the caller owns the data of, as Cedar JSON entities (`uid`, `attrs`,
    /// `parents`): typically the resource and its ancestors, so the PDP needs no access
    /// to the caller's tables. The PDP merges them with what its `EntityLoader` loads:
    /// their attributes win over loaded ones, and their parents are added to the loaded
    /// parents. They must not include the principal, nor carry an attribute the loader
    /// owns (see `CedarAuthorizer::with_loader_attrs`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<Value>,
}

impl AuthorizeRequest {
    /// A request with an empty context and no entities.
    pub fn new(principal: EntityRef, action: EntityRef, resource: EntityRef) -> Self {
        Self {
            principal,
            action,
            resource,
            context: Map::new(),
            entities: Vec::new(),
        }
    }

    /// Sets the request context.
    pub fn with_context(mut self, context: impl Into<Map<String, Value>>) -> Self {
        self.context = context.into();
        self
    }

    /// Adds a Cedar JSON entity the caller supplies, such as the resource.
    pub fn with_entity(mut self, entity: Value) -> Self {
        self.entities.push(entity);
        self
    }

    /// Adds the Cedar JSON entities the caller supplies.
    pub fn with_entities(mut self, entities: impl IntoIterator<Item = Value>) -> Self {
        self.entities.extend(entities);
        self
    }

    /// The uids of the caller-supplied entities and of the parents they name, the
    /// resource first, then in order of appearance, without duplicates. What a loader
    /// resolves grants on.
    pub fn entity_uids(&self) -> Vec<EntityRef> {
        let mut uids = vec![self.resource.clone()];
        let mut push = |value: &Value| {
            if let Some(uid) = entity_ref(value)
                && !uids.contains(&uid)
            {
                uids.push(uid);
            }
        };
        for entity in &self.entities {
            if let Some(uid) = entity.get("uid") {
                push(uid);
            }
            for parent in entity
                .get("parents")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                push(parent);
            }
        }
        uids
    }
}

/// The uid in Cedar JSON, `{"type", "id"}` or the escaped `{"__entity": {"type", "id"}}`.
pub(crate) fn entity_ref(value: &Value) -> Option<EntityRef> {
    let value = value.get("__entity").unwrap_or(value);
    Some(EntityRef::new(
        value.get("type")?.as_str()?,
        value.get("id")?.as_str()?,
    ))
}

/// Whether a request is allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    /// At least one `permit` policy applies and no `forbid` does.
    Allow,
    /// Anything else, including any failure to decide.
    Deny,
}

/// The answer to an [`AuthorizeRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizeResponse {
    /// The decision.
    pub decision: Decision,
    /// The ids of the policies that determined the decision.
    #[serde(default)]
    pub policies: Vec<String>,
    /// Errors met while deciding. With any error that kept the request from being
    /// evaluated, the decision is [`Decision::Deny`].
    #[serde(default)]
    pub errors: Vec<String>,
}

impl AuthorizeResponse {
    /// Whether the decision is [`Decision::Allow`].
    pub fn is_allowed(&self) -> bool {
        self.decision == Decision::Allow
    }

    /// A [`Decision::Deny`] because the request could not be decided.
    pub fn deny(error: impl Into<String>) -> Self {
        Self {
            decision: Decision::Deny,
            policies: Vec::new(),
            errors: vec![error.into()],
        }
    }
}

/// Several [`AuthorizeRequest`]s, answered in order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchAuthorizeRequest {
    /// The requests.
    pub requests: Vec<AuthorizeRequest>,
}

/// The answers to a [`BatchAuthorizeRequest`], one per request in the same order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchAuthorizeResponse {
    /// The answers.
    pub responses: Vec<AuthorizeResponse>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn request_json_is_stable() {
        let request = AuthorizeRequest::new(
            EntityRef::new("Ns::User", "u1"),
            EntityRef::new("Ns::Action", "read"),
            EntityRef::new("Ns::Doc", "d1"),
        )
        .with_context(json!({ "auth_method": "jwt" }).as_object().unwrap().clone())
        .with_entity(json!({
            "uid": { "type": "Ns::Doc", "id": "d1" }, "attrs": {}, "parents": [],
        }));
        let wire = json!({
            "principal": { "type": "Ns::User", "id": "u1" },
            "action": { "type": "Ns::Action", "id": "read" },
            "resource": { "type": "Ns::Doc", "id": "d1" },
            "context": { "auth_method": "jwt" },
            "entities": [{
                "uid": { "type": "Ns::Doc", "id": "d1" }, "attrs": {}, "parents": [],
            }],
        });
        assert_eq!(serde_json::to_value(&request).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<AuthorizeRequest>(wire).unwrap(),
            request
        );
    }

    #[test]
    fn optional_request_fields_may_be_left_out() {
        let request: AuthorizeRequest = serde_json::from_value(json!({
            "principal": { "type": "Ns::User", "id": "u1" },
            "action": { "type": "Ns::Action", "id": "read" },
            "resource": { "type": "Ns::Doc", "id": "d1" },
        }))
        .unwrap();
        assert!(request.context.is_empty());
        assert!(request.entities.is_empty());
        let back = serde_json::to_value(&request).unwrap();
        assert!(back.get("entities").is_none());
    }

    #[test]
    fn lists_the_uids_of_the_entities_and_their_parents() {
        let doc = EntityRef::new("Ns::Doc", "d1");
        let folder = json!({ "type": "Ns::Folder", "id": "f1" });
        let request = AuthorizeRequest::new(
            EntityRef::new("Ns::User", "u1"),
            EntityRef::new("Ns::Action", "read"),
            doc.clone(),
        )
        .with_entities([
            json!({ "uid": doc, "attrs": {}, "parents": [folder] }),
            // Escaped uids count too, and a uid seen before is not repeated.
            json!({ "uid": { "__entity": folder }, "attrs": {}, "parents": [
                { "__entity": { "type": "Ns::Root", "id": "r" } }
            ] }),
        ]);
        assert_eq!(
            request.entity_uids(),
            [
                doc,
                EntityRef::new("Ns::Folder", "f1"),
                EntityRef::new("Ns::Root", "r")
            ]
        );
    }

    #[test]
    fn response_json_is_stable() {
        let response = AuthorizeResponse {
            decision: Decision::Allow,
            policies: vec!["admin-permit".into()],
            errors: vec![],
        };
        let wire = json!({ "decision": "allow", "policies": ["admin-permit"], "errors": [] });
        assert_eq!(serde_json::to_value(&response).unwrap(), wire);
        let parsed: AuthorizeResponse =
            serde_json::from_value(json!({ "decision": "deny" })).unwrap();
        assert_eq!(parsed.decision, Decision::Deny);
        assert!(parsed.policies.is_empty() && parsed.errors.is_empty());
        assert!(
            serde_json::from_value::<AuthorizeResponse>(json!({ "decision": "maybe" })).is_err()
        );
    }

    #[test]
    fn batch_json_is_stable() {
        let batch = BatchAuthorizeResponse {
            responses: vec![AuthorizeResponse::deny("boom")],
        };
        assert_eq!(
            serde_json::to_value(&batch).unwrap(),
            json!({ "responses": [{ "decision": "deny", "policies": [], "errors": ["boom"] }] })
        );
    }
}
