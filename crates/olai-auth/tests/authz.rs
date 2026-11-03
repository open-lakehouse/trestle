//! Both `Authorizer` implementations against the same fixtures
//! (`tests/fixtures/authz/`): `CedarAuthorizer` directly, and `HttpAuthorizer` through an
//! in-test policy decision point that serves the wire API from a `CedarAuthorizer`.

use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use olai_auth::wire::{
    AUTHORIZE_BATCH_PATH, AUTHORIZE_PATH, BatchAuthorizeRequest, BatchAuthorizeResponse,
};
use olai_auth::{
    AuthorizeRequest, AuthorizeResponse, Authorizer, BoxError, CedarAuthorizer, CedarConfigError,
    Decision, EntityLoader, EntityRef, HttpAuthorizer,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

const SCHEMA: &str = include_str!("fixtures/authz/schema.cedarschema");
const POLICIES: &str = include_str!("fixtures/authz/policies.cedar");
const ENTITIES: &str = include_str!("fixtures/authz/entities.json");
const CASES: &str = include_str!("fixtures/authz/cases.json");
const SECRET: &str = "pdp-test-secret";

/// Every fixture entity, for every request.
struct Fixed;

#[olai_auth::async_trait]
impl EntityLoader for Fixed {
    async fn load(&self, _: &AuthorizeRequest) -> Result<Value, BoxError> {
        Ok(serde_json::from_str(ENTITIES)?)
    }
}

struct Failing;

#[olai_auth::async_trait]
impl EntityLoader for Failing {
    async fn load(&self, _: &AuthorizeRequest) -> Result<Value, BoxError> {
        Err("database unreachable".into())
    }
}

fn cedar() -> CedarAuthorizer {
    CedarAuthorizer::new(SCHEMA, POLICIES, Fixed).unwrap()
}

#[derive(Deserialize)]
struct Case {
    name: String,
    principal: (String, String),
    action: String,
    resource: (String, String),
    #[serde(default)]
    context: Map<String, Value>,
    resource_entity: Option<Value>,
    decision: Decision,
    policies: Vec<String>,
}

impl Case {
    fn request(&self) -> AuthorizeRequest {
        let entity = |(t, id): &(String, String)| EntityRef::new(format!("Docs::{t}"), id);
        let mut context = json!({ "auth_method": "jwt", "scopes": [] })
            .as_object()
            .unwrap()
            .clone();
        context.extend(self.context.clone());
        let mut request = AuthorizeRequest::new(
            entity(&self.principal),
            EntityRef::new("Docs::Action", &self.action),
            entity(&self.resource),
        )
        .with_context(context);
        if let Some(entity) = &self.resource_entity {
            request = request.with_resource_entity(entity.clone());
        }
        request
    }
}

fn cases() -> Vec<Case> {
    serde_json::from_str(CASES).unwrap()
}

fn check(case: &Case, response: &AuthorizeResponse) {
    assert_eq!(
        response.decision, case.decision,
        "{}: {response:?}",
        case.name
    );
    assert_eq!(response.policies, case.policies, "{}", case.name);
    assert!(
        response.errors.is_empty(),
        "{}: {:?}",
        case.name,
        response.errors
    );
}

#[tokio::test]
async fn cedar_decides_every_case() {
    let authorizer = cedar();
    for case in cases() {
        check(&case, &authorizer.authorize(&case.request()).await);
    }
}

// The in-test PDP.

async fn serve(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

fn authorized(headers: &HeaderMap) -> bool {
    headers
        .get("authorization")
        .is_some_and(|value| value == format!("Bearer {SECRET}").as_str())
}

async fn single(
    State(pdp): State<Arc<CedarAuthorizer>>,
    headers: HeaderMap,
    Json(request): Json<AuthorizeRequest>,
) -> Response {
    if !authorized(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(pdp.authorize(&request).await).into_response()
}

async fn batch(
    State(pdp): State<Arc<CedarAuthorizer>>,
    headers: HeaderMap,
    Json(batch): Json<BatchAuthorizeRequest>,
) -> Response {
    if !authorized(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let responses = pdp.authorize_batch(&batch.requests).await;
    Json(BatchAuthorizeResponse { responses }).into_response()
}

/// A PDP at `/ok`, and misbehaving ones at `/slow`, `/error`, `/garbage` and `/short`.
async fn pdp() -> String {
    let allow = || Json(json!({ "decision": "allow" }));
    let ok = Router::new()
        .route(AUTHORIZE_PATH, post(single))
        .route(AUTHORIZE_BATCH_PATH, post(batch))
        .with_state(Arc::new(cedar()));
    let slow = Router::new().route(
        AUTHORIZE_PATH,
        post(move || async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            allow()
        }),
    );
    let error = Router::new().route(
        AUTHORIZE_PATH,
        post(|| async { StatusCode::INTERNAL_SERVER_ERROR }),
    );
    let garbage = Router::new()
        .route(AUTHORIZE_PATH, post(|| async { "allow" }))
        .route(
            AUTHORIZE_BATCH_PATH,
            post(|| async { Json(json!({ "responses": "allow" })) }),
        );
    let short = Router::new().route(
        AUTHORIZE_BATCH_PATH,
        post(|| async { Json(json!({ "responses": [{ "decision": "allow" }] })) }),
    );
    serve(
        Router::new()
            .nest("/ok", ok)
            .nest("/slow", slow)
            .nest("/error", error)
            .nest("/garbage", garbage)
            .nest("/short", short),
    )
    .await
}

#[tokio::test]
async fn http_decides_every_case() {
    let base = pdp().await;
    // A trailing slash on the base URL is fine.
    let authorizer = HttpAuthorizer::new(format!("{base}/ok/"), SECRET);
    for case in cases() {
        check(&case, &authorizer.authorize(&case.request()).await);
    }
}

#[tokio::test]
async fn http_batches_every_case() {
    let base = pdp().await;
    let authorizer = HttpAuthorizer::new(format!("{base}/ok"), SECRET);
    let cases = cases();
    let requests: Vec<AuthorizeRequest> = cases.iter().map(Case::request).collect();
    let responses = authorizer.authorize_batch(&requests).await;
    assert_eq!(responses.len(), cases.len());
    for (case, response) in cases.iter().zip(&responses) {
        check(case, response);
    }
    assert!(authorizer.authorize_batch(&[]).await.is_empty());
}

fn any_request() -> AuthorizeRequest {
    cases()[0].request()
}

fn assert_denied(response: &AuthorizeResponse, reason: &str) {
    assert_eq!(response.decision, Decision::Deny, "{response:?}");
    assert!(
        response.errors.iter().any(|e| e.contains(reason)),
        "expected an error containing {reason:?}, got {:?}",
        response.errors
    );
}

#[tokio::test]
async fn http_fails_closed() {
    let base = pdp().await;
    let request = any_request();
    // The first fixture is allowed, so a deny below is the failure's.
    let ok = HttpAuthorizer::new(format!("{base}/ok"), SECRET);
    assert!(ok.authorize(&request).await.is_allowed());

    let wrong_secret = HttpAuthorizer::new(format!("{base}/ok"), "not-the-secret");
    assert_denied(&wrong_secret.authorize(&request).await, "401");
    let answers = wrong_secret
        .authorize_batch(&[request.clone(), request.clone()])
        .await;
    assert_eq!(answers.len(), 2);
    assert_denied(&answers[1], "401");

    let slow = HttpAuthorizer::new(format!("{base}/slow"), SECRET)
        .with_timeout(Duration::from_millis(200));
    assert_denied(&slow.authorize(&request).await, "timed out");

    let error = HttpAuthorizer::new(format!("{base}/error"), SECRET);
    assert_denied(&error.authorize(&request).await, "500");

    let garbage = HttpAuthorizer::new(format!("{base}/garbage"), SECRET);
    assert_denied(&garbage.authorize(&request).await, "invalid decision");
    assert_denied(
        &garbage
            .authorize_batch(std::slice::from_ref(&request))
            .await[0],
        "invalid decision",
    );

    let short = HttpAuthorizer::new(format!("{base}/short"), SECRET);
    let answers = short
        .authorize_batch(&[request.clone(), request.clone()])
        .await;
    assert_eq!(answers.len(), 2);
    assert_denied(&answers[0], "1 of 2");

    // Nothing listens on port 9 (discard) locally.
    let down = HttpAuthorizer::new("http://127.0.0.1:9", SECRET);
    assert_denied(&down.authorize(&request).await, "unreachable");
}

#[tokio::test]
async fn cedar_fails_closed() {
    let request = any_request();
    assert!(cedar().authorize(&request).await.is_allowed());

    let failing = CedarAuthorizer::new(SCHEMA, POLICIES, Failing).unwrap();
    assert_denied(&failing.authorize(&request).await, "database unreachable");

    // A resource entity for another resource could smuggle in other entities' data.
    let mut smuggled = request.clone();
    smuggled.resource_entity = Some(json!({
        "uid": { "type": "Docs::User", "id": "dave" },
        "attrs": { "banned": false, "email_verified": true },
        "parents": [{ "type": "Docs::Role", "id": "admin" }],
    }));
    assert_denied(
        &cedar().authorize(&smuggled).await,
        "not the request's resource",
    );

    let mut no_context = request.clone();
    no_context.context = Map::new();
    assert_denied(&cedar().authorize(&no_context).await, "invalid context");

    let mut bad_type = request.clone();
    bad_type.principal = EntityRef::new("not a type", "x");
    assert_denied(&cedar().authorize(&bad_type).await, "invalid entity type");

    // `ingest` doesn't apply to users.
    let mut wrong_action = request.clone();
    wrong_action.action = EntityRef::new("Docs::Action", "ingest");
    wrong_action.resource = EntityRef::new("Docs::Folder", "inbox");
    assert_denied(&cedar().authorize(&wrong_action).await, "invalid request");

    let mut unknown_action = request.clone();
    unknown_action.action = EntityRef::new("Docs::Action", "delete");
    assert_denied(&cedar().authorize(&unknown_action).await, "invalid");

    // An entity that doesn't match the schema.
    let mut bad_entity = request;
    bad_entity.resource_entity = Some(json!({
        "uid": { "type": "Docs::Doc", "id": "plan" },
        "attrs": { "public": "yes" },
        "parents": [],
    }));
    assert_denied(&cedar().authorize(&bad_entity).await, "invalid entities");
}

#[test]
fn cedar_rejects_bad_configuration() {
    let config = |schema: &str, policies: &str| CedarAuthorizer::new(schema, policies, Fixed).err();
    assert!(matches!(
        config("namespace {", POLICIES),
        Some(CedarConfigError::Schema(_))
    ));
    assert!(matches!(
        config(SCHEMA, "permit (principal"),
        Some(CedarConfigError::Policies(_))
    ));
    assert!(matches!(
        config(
            SCHEMA,
            "permit (principal == ?principal, action, resource);"
        ),
        Some(CedarConfigError::Templates)
    ));
    assert!(matches!(
        config(
            SCHEMA,
            r#"@id("p") permit (principal, action, resource);
               @id("p") forbid (principal, action, resource);"#
        ),
        Some(CedarConfigError::DuplicateId(id)) if id == "p"
    ));
    // `Docs::Folder` has no `owner`.
    assert!(matches!(
        config(
            SCHEMA,
            "permit (principal, action, resource is Docs::Folder) when { resource.owner == principal };"
        ),
        Some(CedarConfigError::Validation(_))
    ));
}

#[tokio::test]
async fn unannotated_policies_keep_positional_ids() {
    let authorizer = CedarAuthorizer::new(
        SCHEMA,
        "forbid (principal, action, resource) when { false };\npermit (principal, action, resource);",
        Fixed,
    )
    .unwrap();
    let response = authorizer.authorize(&any_request()).await;
    assert!(response.is_allowed());
    assert_eq!(response.policies, ["policy1"]);
}
