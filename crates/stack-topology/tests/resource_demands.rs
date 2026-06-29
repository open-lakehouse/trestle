//! Tests for the resource-demand layer: a module declares a `(kind, name)` it needs,
//! and the planner auto-deploys a provider, provisions the named resource, and binds the
//! provider's typed [`Connection`] back into the consumer.

use std::collections::BTreeMap;

use olai_stack_topology::{
    Catalog, Connection, ConnectionBinding, ConnectionField, ConnectionTemplate, Module, ModuleId,
    ObjectStoreCredential, Placement, PlanCtx, PlanError, Provides, RenderSpec, ResourceDemand,
    Role, Selection, ServiceSpec, baseline_catalog, plan,
};

/// Build a minimal provider module that provisions `kind` and vends the given typed
/// connection template.
fn provider(id: &str, kind: &str, template: ConnectionTemplate) -> Module {
    let mut provides = Provides::default();
    provides.resource_kinds.insert(kind.into(), template);
    Module {
        id: ModuleId::from(id),
        display_name: None,
        summary: None,
        category: None,
        provider_of: None,
        requires: vec![],
        conflicts_with: vec![],
        needs: vec![],
        services: vec![ServiceSpec {
            name: id.to_string(),
            role: Role::new("provider"),
            placement: Placement::Container {
                service: id.to_string(),
            },
            endpoints: vec![],
            depends_on: vec![],
        }],
        provides,
        knobs: vec![],
        render: RenderSpec::default(),
    }
}

/// A relational-db connection template at `url`-with-`{name}`.
fn relational(url: &str) -> ConnectionTemplate {
    ConnectionTemplate(Connection::RelationalDb { url: url.into() })
}

/// A minimal S3 object-store connection template (only the addressing fields matter for
/// these tests; credentials are filler).
fn s3_store(uri: &str) -> ConnectionTemplate {
    ConnectionTemplate(Connection::ObjectStore {
        uri: uri.into(),
        bucket: "{name}".into(),
        endpoint: "http://store:1".into(),
        credential: ObjectStoreCredential::S3 {
            access_key_id: "k".into(),
            secret_access_key: "s".into(),
            region: "r".into(),
        },
    })
}

/// A binding of a single field to an env-var key.
fn bind1(field: ConnectionField, key: &str) -> ConnectionBinding {
    ConnectionBinding {
        bind: vec![(field, key.into())],
    }
}

/// Build a consumer module with the given demands.
fn consumer(id: &str, needs: Vec<ResourceDemand>) -> Module {
    Module {
        id: ModuleId::from(id),
        display_name: None,
        summary: None,
        category: None,
        provider_of: None,
        requires: vec![],
        conflicts_with: vec![],
        needs,
        services: vec![ServiceSpec {
            name: id.to_string(),
            role: Role::new("consumer"),
            placement: Placement::Container {
                service: id.to_string(),
            },
            endpoints: vec![],
            depends_on: vec![],
        }],
        provides: Provides::default(),
        knobs: vec![],
        render: RenderSpec::default(),
    }
}

#[test]
fn selecting_only_unity_catalog_auto_provisions_its_providers() {
    // UC declares it needs a relational_db + object_store; selecting *only* UC must
    // pull in the relational store and object store (plus its envoy `requires`).
    let p = plan(
        &Selection::modules(["unity-catalog"]),
        &baseline_catalog(),
        &PlanCtx::default(),
    )
    .expect("UC alone should plan, auto-provisioning its providers");

    for id in ["unity-catalog", "postgres", "seaweedfs", "envoy"] {
        assert!(
            p.graph.module(&ModuleId::from(id)).is_some(),
            "expected {id} in the auto-provisioned graph"
        );
    }

    // The named resources are provisioned.
    assert!(p.postgres_databases.contains(&"unitycatalog".to_string()));
    assert!(p.s3_buckets.contains(&"unity".to_string()));

    // Providers are ordered before the consumer (compose startup order).
    let order: Vec<&str> = p.head.includes.iter().map(|i| i.module.as_str()).collect();
    let pos = |id: &str| order.iter().position(|x| *x == id);
    assert!(pos("postgres") < pos("unity-catalog"));
    assert!(pos("seaweedfs") < pos("unity-catalog"));
}

#[test]
fn provider_connection_is_bound_into_the_consumer() {
    let p = plan(
        &Selection::modules(["unity-catalog"]),
        &baseline_catalog(),
        &PlanCtx::default(),
    )
    .unwrap();

    // UC's demand binds the relational URL as UC_DATABASE_URL, with {name} resolved to the
    // demanded database name.
    let uc_env = p.injected.get(&ModuleId::from("unity-catalog")).unwrap();
    assert_eq!(
        uc_env.get("UC_DATABASE_URL"),
        Some(
            "postgresql://${POSTGRES_USER:-postgres}:${POSTGRES_PASSWORD:-postgres}@db:5432/unitycatalog"
        )
    );

    // It also reaches the stack `.env` so compose can resolve the fragment's
    // ${UC_DATABASE_URL} at run time.
    assert_eq!(
        p.env.get("UC_DATABASE_URL"),
        uc_env.get("UC_DATABASE_URL"),
        "bound connection field must also land in the stack .env"
    );

    // The typed connection is exposed on the plan for downstream consumers.
    let conn = p
        .connections
        .get(&(ModuleId::from("unity-catalog"), 0))
        .expect("UC's first demand (relational_db) resolves a connection");
    assert!(matches!(conn, Connection::RelationalDb { .. }));
}

#[test]
fn an_app_module_demanding_postgres_gets_a_provider_and_its_url() {
    // A templated app declares it needs a Postgres database — same mechanism as the
    // infra modules. The planner auto-provisions Postgres and binds the URL.
    let app = consumer(
        "my-app",
        vec![ResourceDemand {
            resource: "relational_db".into(),
            name: "appdb".into(),
            provider: None,
            bind: bind1(ConnectionField::Url, "APP_DATABASE_URL"),
        }],
    );
    let catalog = baseline_catalog().merge(Catalog::from_modules([app]));

    let p = plan(
        &Selection::modules(["my-app"]),
        &catalog,
        &PlanCtx::default(),
    )
    .unwrap();

    assert!(p.graph.module(&ModuleId::from("postgres")).is_some());
    assert!(p.postgres_databases.contains(&"appdb".to_string()));
    let app_env = p.injected.get(&ModuleId::from("my-app")).unwrap();
    assert_eq!(
        app_env.get("APP_DATABASE_URL"),
        Some(
            "postgresql://${POSTGRES_USER:-postgres}:${POSTGRES_PASSWORD:-postgres}@db:5432/appdb"
        )
    );
}

#[test]
fn two_consumers_share_one_provider_and_each_get_their_db() {
    let a = consumer(
        "svc-a",
        vec![ResourceDemand {
            resource: "relational_db".into(),
            name: "a_db".into(),
            provider: None,
            bind: ConnectionBinding::default(),
        }],
    );
    let b = consumer(
        "svc-b",
        vec![ResourceDemand {
            resource: "relational_db".into(),
            name: "b_db".into(),
            provider: None,
            bind: ConnectionBinding::default(),
        }],
    );
    let catalog = baseline_catalog().merge(Catalog::from_modules([a, b]));
    let p = plan(
        &Selection::modules(["svc-a", "svc-b"]),
        &catalog,
        &PlanCtx::default(),
    )
    .unwrap();

    // One Postgres provider, both databases provisioned.
    assert_eq!(
        p.graph
            .nodes
            .iter()
            .filter(|m| m.id.as_str() == "postgres")
            .count(),
        1
    );
    assert!(p.postgres_databases.contains(&"a_db".to_string()));
    assert!(p.postgres_databases.contains(&"b_db".to_string()));
}

#[test]
fn unsatisfied_demand_errors_when_no_provider_exists() {
    // A consumer needs a kind nothing in the catalog provisions.
    let lonely = consumer(
        "lonely",
        vec![ResourceDemand {
            resource: "message_queue".into(),
            name: "events".into(),
            provider: None,
            bind: ConnectionBinding::default(),
        }],
    );
    let catalog = Catalog::from_modules([lonely]);
    let err = plan(
        &Selection::modules(["lonely"]),
        &catalog,
        &PlanCtx::default(),
    )
    .unwrap_err();
    assert_eq!(
        err,
        PlanError::UnsatisfiedDemand {
            module: "lonely".into(),
            resource: "message_queue".into(),
        }
    );
}

#[test]
fn ambiguous_provider_errors_when_two_modules_provide_the_kind() {
    // Two providers for the same kind, no tie-break → the planner refuses to guess.
    let p1 = provider("pg-a", "relational_db", relational("a://{name}"));
    let p2 = provider("pg-b", "relational_db", relational("b://{name}"));
    let c = consumer(
        "needs-db",
        vec![ResourceDemand {
            resource: "relational_db".into(),
            name: "db".into(),
            provider: None,
            bind: ConnectionBinding::default(),
        }],
    );
    let catalog = Catalog::from_modules([p1, p2, c]);
    let err = plan(
        &Selection::modules(["needs-db"]),
        &catalog,
        &PlanCtx::default(),
    )
    .unwrap_err();
    match err {
        PlanError::AmbiguousProvider {
            resource,
            providers,
        } => {
            assert_eq!(resource, "relational_db");
            assert_eq!(
                providers,
                vec![ModuleId::from("pg-a"), ModuleId::from("pg-b")]
            );
        }
        other => panic!("expected AmbiguousProvider, got {other:?}"),
    }
}

#[test]
fn demand_chain_resolves_to_a_fixed_point() {
    // A consumer needs kind X; the X-provider itself needs kind Y. Both providers must
    // be auto-pulled in (the fixed point, not a single pass).
    let mut x_provider = provider("x-prov", "x", relational("x://{name}"));
    x_provider.needs = vec![ResourceDemand {
        resource: "y".into(),
        name: "y-res".into(),
        provider: None,
        bind: ConnectionBinding::default(),
    }];
    let y_provider = provider("y-prov", "y", relational("y://{name}"));
    let c = consumer(
        "top",
        vec![ResourceDemand {
            resource: "x".into(),
            name: "x-res".into(),
            provider: None,
            bind: ConnectionBinding::default(),
        }],
    );
    let catalog = Catalog::from_modules([x_provider, y_provider, c]);
    let p = plan(&Selection::modules(["top"]), &catalog, &PlanCtx::default()).unwrap();
    for id in ["top", "x-prov", "y-prov"] {
        assert!(
            p.graph.module(&ModuleId::from(id)).is_some(),
            "fixed point should pull in {id}"
        );
    }
}

// ---- Abstract `object_store` role: planner picks the provider --------------------

/// A `PlanCtx` that prefers Azurite over SeaweedFS for the object store.
fn azurite_preferred() -> PlanCtx {
    let mut preference = BTreeMap::new();
    preference.insert(
        "object_store".to_string(),
        vec![ModuleId::from("azurite"), ModuleId::from("seaweedfs")],
    );
    PlanCtx {
        provider_preference: preference,
        ..Default::default()
    }
}

#[test]
fn default_object_store_is_seaweedfs() {
    // No preference → the catalog default (SeaweedFS) satisfies MLflow's object_store
    // demand; Azurite is not deployed and no Azure vars appear.
    let p = plan(
        &Selection::modules(["mlflow"]),
        &baseline_catalog(),
        &PlanCtx::default(),
    )
    .unwrap();
    assert!(p.graph.module(&ModuleId::from("seaweedfs")).is_some());
    assert!(p.graph.module(&ModuleId::from("azurite")).is_none());
    assert!(p.s3_buckets.contains(&"mlflow".to_string()));
    assert!(p.azure_containers.is_empty());
    assert_eq!(p.env.get("AWS_ACCESS_KEY_ID"), Some("seaweedfs"));
    assert_eq!(p.env.get("AZURE_STORAGE_CONNECTION_STRING"), None);
}

#[test]
fn preference_selects_azurite_and_drops_aws() {
    // An Azurite-preferred environment: MLflow's object_store demand resolves to Azurite.
    let p = plan(
        &Selection::modules(["mlflow"]),
        &baseline_catalog(),
        &azurite_preferred(),
    )
    .unwrap();

    // Azurite is deployed, SeaweedFS is not.
    assert!(p.graph.module(&ModuleId::from("azurite")).is_some());
    assert!(p.graph.module(&ModuleId::from("seaweedfs")).is_none());
    // The container (not an S3 bucket) is provisioned on Azurite.
    assert!(p.azure_containers.contains(&"mlflow".to_string()));
    assert!(p.s3_buckets.is_empty());
    // Azure creds present; no AWS_* leak (SeaweedFS isn't in the graph).
    assert!(p.env.get("AZURE_STORAGE_CONNECTION_STRING").is_some());
    assert_eq!(p.env.get("AWS_ACCESS_KEY_ID"), None);
}

#[test]
fn consumer_uri_follows_the_chosen_provider() {
    // A consumer that binds the object_store `uri` field gets the S3-shaped value by
    // default and the Azure-shaped value under an Azurite preference.
    let app = consumer(
        "store-app",
        vec![ResourceDemand {
            resource: "object_store".into(),
            name: "artifacts".into(),
            provider: None,
            bind: bind1(ConnectionField::Uri, "STORE_URI"),
        }],
    );
    let catalog = baseline_catalog().merge(Catalog::from_modules([app]));

    let s3 = plan(
        &Selection::modules(["store-app"]),
        &catalog,
        &PlanCtx::default(),
    )
    .unwrap();
    assert_eq!(
        s3.injected
            .get(&ModuleId::from("store-app"))
            .and_then(|e| e.get("STORE_URI")),
        Some("s3://artifacts")
    );

    let azure = plan(
        &Selection::modules(["store-app"]),
        &catalog,
        &azurite_preferred(),
    )
    .unwrap();
    assert_eq!(
        azure
            .injected
            .get(&ModuleId::from("store-app"))
            .and_then(|e| e.get("STORE_URI")),
        Some("wasbs://artifacts@devstoreaccount1.blob.core.windows.net")
    );
}

#[test]
fn a_demand_pin_overrides_preference() {
    // Even under an Azurite-preferred ctx, a demand pinning SeaweedFS uses S3.
    let app = consumer(
        "pinned-app",
        vec![ResourceDemand {
            resource: "object_store".into(),
            name: "artifacts".into(),
            provider: Some(ModuleId::from("seaweedfs")),
            bind: bind1(ConnectionField::Uri, "STORE_URI"),
        }],
    );
    let catalog = baseline_catalog().merge(Catalog::from_modules([app]));
    let p = plan(
        &Selection::modules(["pinned-app"]),
        &catalog,
        &azurite_preferred(),
    )
    .unwrap();
    assert!(p.graph.module(&ModuleId::from("seaweedfs")).is_some());
    assert_eq!(
        p.injected
            .get(&ModuleId::from("pinned-app"))
            .and_then(|e| e.get("STORE_URI")),
        Some("s3://artifacts")
    );
}

#[test]
fn ambiguous_object_store_without_default_or_preference_errors() {
    // Two object_store providers, no catalog default, no preference, no pin → ambiguous.
    // (Build a catalog without the baseline's default to exercise the error.)
    let a = provider("prov-s3", "object_store", s3_store("s3://{name}"));
    let b = provider("prov-azure", "object_store", s3_store("s3://{name}"));
    let c = consumer(
        "needs-store",
        vec![ResourceDemand {
            resource: "object_store".into(),
            name: "x".into(),
            provider: None,
            bind: ConnectionBinding::default(),
        }],
    );
    let catalog = Catalog::from_modules([a, b, c]); // no with_default_provider
    let err = plan(
        &Selection::modules(["needs-store"]),
        &catalog,
        &PlanCtx::default(),
    )
    .unwrap_err();
    assert!(
        matches!(err, PlanError::AmbiguousProvider { ref resource, .. } if resource == "object_store"),
        "expected AmbiguousProvider, got {err:?}"
    );

    // A catalog default resolves it.
    let catalog = Catalog::from_modules([
        provider("prov-s3", "object_store", s3_store("s3://{name}")),
        provider("prov-azure", "object_store", s3_store("s3://{name}")),
        consumer(
            "needs-store",
            vec![ResourceDemand {
                resource: "object_store".into(),
                name: "x".into(),
                provider: None,
                bind: ConnectionBinding::default(),
            }],
        ),
    ])
    .with_default_provider("object_store", "prov-s3");
    let ok = plan(
        &Selection::modules(["needs-store"]),
        &catalog,
        &PlanCtx::default(),
    );
    assert!(ok.is_ok(), "catalog default should resolve the ambiguity");
}

#[test]
fn a_service_can_demand_two_object_stores_of_the_same_role() {
    // A Unity-Catalog-shaped consumer: one object store for managed storage, a second
    // for an external location. Same role, distinct names, distinct bind keys.
    let uc = consumer(
        "catalog",
        vec![
            ResourceDemand {
                resource: "object_store".into(),
                name: "uc-managed".into(),
                provider: None,
                bind: bind1(ConnectionField::Uri, "UC_MANAGED_URI"),
            },
            ResourceDemand {
                resource: "object_store".into(),
                name: "uc-external".into(),
                provider: None,
                bind: bind1(ConnectionField::Uri, "UC_EXTERNAL_URI"),
            },
        ],
    );
    let catalog = baseline_catalog().merge(Catalog::from_modules([uc]));
    let p = plan(
        &Selection::modules(["catalog"]),
        &catalog,
        &PlanCtx::default(),
    )
    .unwrap();

    // Both stores are provisioned in the one chosen provider (SeaweedFS by default).
    assert!(p.s3_buckets.contains(&"uc-managed".to_string()));
    assert!(p.s3_buckets.contains(&"uc-external".to_string()));

    // Each demand binds its own field under its own key — no collision.
    let env = p.injected.get(&ModuleId::from("catalog")).unwrap();
    assert_eq!(env.get("UC_MANAGED_URI"), Some("s3://uc-managed"));
    assert_eq!(env.get("UC_EXTERNAL_URI"), Some("s3://uc-external"));

    // One object-store provider serves both demands.
    assert_eq!(
        p.graph
            .nodes
            .iter()
            .filter(|m| m.id.as_str() == "seaweedfs")
            .count(),
        1
    );
}

#[test]
fn same_role_demands_can_pin_different_providers() {
    // The escape hatch: a service's two object-store demands can deliberately land on
    // different providers (e.g. managed on SeaweedFS, external on Azurite for
    // credential vending) by pinning each.
    let uc = consumer(
        "catalog",
        vec![
            ResourceDemand {
                resource: "object_store".into(),
                name: "uc-managed".into(),
                provider: Some(ModuleId::from("seaweedfs")),
                bind: bind1(ConnectionField::Uri, "UC_MANAGED_URI"),
            },
            ResourceDemand {
                resource: "object_store".into(),
                name: "uc-external".into(),
                provider: Some(ModuleId::from("azurite")),
                bind: bind1(ConnectionField::Uri, "UC_EXTERNAL_URI"),
            },
        ],
    );
    // Both object_store providers run in one environment, which is allowed *because* each
    // demand pins its provider — role-exclusivity (`check_role_exclusivity`) only rejects
    // unpinned same-role clashes, so no hand-listed `conflicts_with` is involved.
    let catalog = baseline_catalog().merge(Catalog::from_modules([uc]));
    let p = plan(
        &Selection::modules(["catalog"]),
        &catalog,
        &PlanCtx::default(),
    )
    .unwrap();

    // Managed went to SeaweedFS (s3://), external to Azurite (wasbs://).
    assert!(p.s3_buckets.contains(&"uc-managed".to_string()));
    assert!(p.azure_containers.contains(&"uc-external".to_string()));
    let env = p.injected.get(&ModuleId::from("catalog")).unwrap();
    assert_eq!(env.get("UC_MANAGED_URI"), Some("s3://uc-managed"));
    assert_eq!(
        env.get("UC_EXTERNAL_URI"),
        Some("wasbs://uc-external@devstoreaccount1.blob.core.windows.net")
    );
}

#[test]
fn binding_a_field_the_connection_lacks_errors() {
    // An object_store connection has no `url` field; binding it must fail at plan time,
    // naming the unbound field — the typed replacement for the old contract/coordinate
    // errors.
    let app = consumer(
        "needs-store",
        vec![ResourceDemand {
            resource: "object_store".into(),
            name: "data".into(),
            provider: None,
            bind: bind1(ConnectionField::Url, "BOGUS_URL"),
        }],
    );
    let catalog = baseline_catalog().merge(Catalog::from_modules([app]));
    let err = plan(
        &Selection::modules(["needs-store"]),
        &catalog,
        &PlanCtx::default(),
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            PlanError::UnboundConnectionField { ref resource, field, .. }
                if resource == "object_store" && field == ConnectionField::Url
        ),
        "expected UnboundConnectionField for url on an object store, got {err:?}"
    );
}

#[test]
fn two_unpinned_object_store_providers_in_one_env_is_rejected() {
    // Selecting both object_store providers directly, with no demand pin to sanction the
    // pair, is an unpinned same-role clash: the planner refuses to silently pick one.
    let err = plan(
        &Selection::modules([
            "seaweedfs",
            "azurite",
            "mlflow", // demands an object_store, but pins nothing
        ]),
        &baseline_catalog(),
        &PlanCtx::default(),
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            PlanError::ConflictingRoleProviders { ref role, ref providers }
                if role == "object_store"
                    && providers.contains(&ModuleId::from("seaweedfs"))
                    && providers.contains(&ModuleId::from("azurite"))
        ),
        "expected ConflictingRoleProviders for the unpinned object_store pair, got {err:?}"
    );
}
