//! `MemoryGrantStore` against the `GrantStore` contract, and `GrantsEntityLoader` under a
//! `CedarAuthorizer` deciding with grants per securable (`tests/fixtures/grants/`).

mod support;

use olai_auth::{
    AuthorizeRequest, Authorizer, CedarAuthorizer, Decision, EntityRef, GrantChange, GrantStore,
    GrantsEntityLoader, MemoryGrantStore, Subject, SubjectTypes,
};
use serde_json::{Value, json};
use support::grant_store::{GROUP_ENG, GROUP_STAFF, conformance};

const SCHEMA: &str = include_str!("fixtures/grants/schema.cedarschema");
const POLICIES: &str = include_str!("fixtures/grants/policies.cedar");

fn groups() -> MemoryGrantStore {
    let store = MemoryGrantStore::new();
    store.add_group(GROUP_STAFF, "staff");
    store.add_group(GROUP_ENG, "eng");
    store.add_member(GROUP_STAFF, Subject::group(GROUP_ENG));
    store.add_member(GROUP_ENG, Subject::user("alice"));
    store.add_member(GROUP_STAFF, Subject::user("bob"));
    store
}

#[tokio::test]
async fn memory_store_meets_the_contract() {
    conformance(&groups()).await;
}

fn uid(kind: &str, id: &str) -> Value {
    json!({ "type": format!("Cat::{kind}"), "id": id })
}

/// What a server supplies for `main.sales.orders`: the table, schema and catalog, with
/// their hierarchy and the typed parent attributes the policies navigate.
fn get_table(user: &str) -> AuthorizeRequest {
    let (catalog, schema, table) = (
        uid("Catalog", "main"),
        uid("Schema", "main.sales"),
        uid("Table", "main.sales.orders"),
    );
    AuthorizeRequest::new(
        EntityRef::new("Cat::User", user),
        EntityRef::new("Cat::Action", "GetTable"),
        EntityRef::new("Cat::Table", "main.sales.orders"),
    )
    .with_entities([
        json!({
            "uid": table,
            "attrs": {
                "catalog": { "__entity": catalog },
                "schema": { "__entity": schema },
            },
            "parents": [schema, catalog],
        }),
        json!({
            "uid": schema,
            "attrs": { "catalog": { "__entity": catalog } },
            "parents": [catalog],
        }),
        json!({ "uid": catalog, "attrs": {}, "parents": [] }),
    ])
}

async fn grant(
    store: &MemoryGrantStore,
    subject: Subject,
    privileges: &[&str],
    kind: &str,
    id: &str,
) {
    store
        .apply(
            &EntityRef::new(format!("Cat::{kind}"), id),
            &[GrantChange {
                subject,
                add: privileges.iter().map(|p| p.to_string()).collect(),
                remove: vec![],
            }],
        )
        .await
        .unwrap();
}

async fn authorizer() -> CedarAuthorizer {
    let store = groups();
    // alice: everything in place, SELECT on the table.
    grant(
        &store,
        Subject::user("alice"),
        &["USE_CATALOG"],
        "Catalog",
        "main",
    )
    .await;
    grant(
        &store,
        Subject::user("alice"),
        &["USE_SCHEMA"],
        "Schema",
        "main.sales",
    )
    .await;
    grant(
        &store,
        Subject::user("alice"),
        &["SELECT"],
        "Table",
        "main.sales.orders",
    )
    .await;
    // carol: USE_SCHEMA on the table instead of on its schema.
    grant(
        &store,
        Subject::user("carol"),
        &["USE_CATALOG"],
        "Catalog",
        "main",
    )
    .await;
    grant(
        &store,
        Subject::user("carol"),
        &["USE_SCHEMA", "SELECT"],
        "Table",
        "main.sales.orders",
    )
    .await;
    // staff (bob directly): SELECT on the whole catalog, inherited by its tables.
    grant(
        &store,
        Subject::group(GROUP_STAFF),
        &["USE_CATALOG", "SELECT"],
        "Catalog",
        "main",
    )
    .await;
    grant(
        &store,
        Subject::group(GROUP_STAFF),
        &["USE_SCHEMA"],
        "Schema",
        "main.sales",
    )
    .await;
    // dave owns the table, and nothing else.
    grant(
        &store,
        Subject::user("dave"),
        &["OWNER"],
        "Table",
        "main.sales.orders",
    )
    .await;
    // erin: USE_SCHEMA on the catalog, which is not the schema.
    grant(
        &store,
        Subject::user("erin"),
        &["USE_CATALOG", "USE_SCHEMA", "SELECT"],
        "Catalog",
        "main",
    )
    .await;

    let loader = GrantsEntityLoader::new(
        store,
        SubjectTypes::new("Cat::User", "Cat::ServiceAccount", "Cat::Group"),
    );
    CedarAuthorizer::new(SCHEMA, POLICIES, loader).unwrap()
}

#[tokio::test]
async fn decides_on_the_grants_of_each_securable() {
    let authorizer = authorizer().await;
    let cases = [
        ("alice", Decision::Allow, vec!["get-table"]),
        ("carol", Decision::Deny, vec![]),
        ("bob", Decision::Allow, vec!["get-table"]),
        ("dave", Decision::Allow, vec!["table-owner"]),
        ("erin", Decision::Deny, vec![]),
        ("nobody", Decision::Deny, vec![]),
    ];
    for (user, decision, policies) in &cases {
        let response = authorizer.authorize(&get_table(user)).await;
        assert!(response.errors.is_empty(), "{user}: {:?}", response.errors);
        assert_eq!(response.decision, *decision, "{user}");
        assert_eq!(&response.policies, policies, "{user}");
    }

    // The same through one batch.
    let requests: Vec<AuthorizeRequest> = cases.iter().map(|(u, ..)| get_table(u)).collect();
    let responses = authorizer.authorize_batch(&requests).await;
    let decisions: Vec<Decision> = responses.iter().map(|r| r.decision).collect();
    let expected: Vec<Decision> = cases.iter().map(|(_, d, _)| *d).collect();
    assert_eq!(decisions, expected);
}

#[tokio::test]
async fn the_server_cannot_forge_grants() {
    let authorizer = authorizer().await;
    let mut forged = get_table("nobody");
    forged.entities[0]["attrs"]["privileges"] = json!(["OWNER"]);
    let response = authorizer.authorize(&forged).await;
    assert_eq!(response.decision, Decision::Deny);
    assert!(
        response
            .errors
            .iter()
            .any(|e| e.contains("only the entity loader may set")),
        "{:?}",
        response.errors
    );
}
