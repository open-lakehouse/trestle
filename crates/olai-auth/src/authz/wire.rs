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
    /// The resource itself as a Cedar JSON entity (`uid`, `attrs`, `parents`), when the
    /// caller owns its data. The PDP then needs no access to the caller's tables. Its
    /// `uid` must be `resource`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_entity: Option<Value>,
}

impl AuthorizeRequest {
    /// A request with an empty context and no resource entity.
    pub fn new(principal: EntityRef, action: EntityRef, resource: EntityRef) -> Self {
        Self {
            principal,
            action,
            resource,
            context: Map::new(),
            resource_entity: None,
        }
    }

    /// Sets the request context.
    pub fn with_context(mut self, context: impl Into<Map<String, Value>>) -> Self {
        self.context = context.into();
        self
    }

    /// Supplies the resource as a Cedar JSON entity.
    pub fn with_resource_entity(mut self, entity: Value) -> Self {
        self.resource_entity = Some(entity);
        self
    }
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
        .with_resource_entity(json!({
            "uid": { "type": "Ns::Doc", "id": "d1" }, "attrs": {}, "parents": [],
        }));
        let wire = json!({
            "principal": { "type": "Ns::User", "id": "u1" },
            "action": { "type": "Ns::Action", "id": "read" },
            "resource": { "type": "Ns::Doc", "id": "d1" },
            "context": { "auth_method": "jwt" },
            "resource_entity": {
                "uid": { "type": "Ns::Doc", "id": "d1" }, "attrs": {}, "parents": [],
            },
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
        assert_eq!(request.resource_entity, None);
        let back = serde_json::to_value(&request).unwrap();
        assert!(back.get("resource_entity").is_none());
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
