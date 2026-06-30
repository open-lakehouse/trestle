//! Tests for the resource-demand layer: a module declares a `(kind, name)` it needs,
//! and the planner auto-deploys a provider, provisions the named resource, and binds the
//! provider's typed [`Connection`] back into the consumer.

use std::collections::BTreeMap;
use std::sync::Arc;

use olai_stack_topology::{
    Catalog, Connection, ConnectionBinding, ConnectionField, ConnectionTemplate, DataModule,
    ExtraResource, Module, ModuleId, ObjectStoreCredential, Placement, PlanCtx, PlanError,
    Provides, RenderSpec, ResourceDemand, Role, Selection, ServiceSpec, baseline_catalog,
};

/// Build a minimal provider module that provisions `kind` and vends the given typed
/// connection template.
fn provider(id: &str, kind: &str, template: ConnectionTemplate) -> Arc<dyn Module> {
    provider_with_needs(id, kind, template, vec![])
}

/// A provider that itself demands `needs` (for the fixed-point chain test).
fn provider_with_needs(
    id: &str,
    kind: &str,
    template: ConnectionTemplate,
    needs: Vec<ResourceDemand>,
) -> Arc<dyn Module> {
    let mut provides = Provides::default();
    provides.resource_kinds.insert(kind.into(), template);
    Arc::new(DataModule {
        id: ModuleId::from(id),
        display_name: None,
        summary: None,
        category: None,
        provider_of: None,
        requires: vec![],
        conflicts_with: vec![],
        needs,
        service_specs: vec![ServiceSpec {
            name: id.to_string(),
            role: Role::new("provider"),
            placement: Placement::Container {
                service: id.to_string(),
            },
            endpoints: vec![],
            depends_on: vec![],
            base_path: String::new(),
        }],
        provides,
        knobs: vec![],
        render: RenderSpec::default(),
    })
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
fn consumer(id: &str, needs: Vec<ResourceDemand>) -> Arc<dyn Module> {
    Arc::new(DataModule {
        id: ModuleId::from(id),
        display_name: None,
        summary: None,
        category: None,
        provider_of: None,
        requires: vec![],
        conflicts_with: vec![],
        needs,
        service_specs: vec![ServiceSpec {
            name: id.to_string(),
            role: Role::new("consumer"),
            placement: Placement::Container {
                service: id.to_string(),
            },
            endpoints: vec![],
            depends_on: vec![],
            base_path: String::new(),
        }],
        provides: Provides::default(),
        knobs: vec![],
        render: RenderSpec::default(),
    })
}

#[test]
fn selecting_only_unity_catalog_auto_provisions_its_providers() {
    // UC declares it needs a relational_db + object_store; selecting *only* UC must
    // pull in the relational store and object store (plus its envoy `requires`).
    let p = baseline_catalog()
        .plan(&Selection::modules(["unity-catalog"]), &PlanCtx::default())
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
fn provider_connection_is_resolved_for_the_consumer_without_binding() {
    // UC declares no `ConnectionBinding`: it reads its resolved connections straight from the
    // render context rather than round-tripping coordinates through `.env`. The planner must
    // still resolve the demand, expose the typed connection on the plan, and inject nothing.
    let p = baseline_catalog()
        .plan(&Selection::modules(["unity-catalog"]), &PlanCtx::default())
        .unwrap();

    // No coordinate is injected into UC's env — the old `UC_DATABASE_URL` round-trip is gone.
    let uc_env = p.injected.get(&ModuleId::from("unity-catalog")).unwrap();
    assert_eq!(uc_env.get("UC_DATABASE_URL"), None);
    assert_eq!(p.env.get("UC_DATABASE_URL"), None);

    // The typed connection is still exposed on the plan, with `{name}` resolved to UC's
    // demanded database — this is what the fragment reads as `connections.relational_db.0.url`.
    let conn = p
        .connections
        .get(&(ModuleId::from("unity-catalog"), 0))
        .expect("UC's first demand (relational_db) resolves a connection");
    match conn {
        Connection::RelationalDb { url } => assert!(
            url.contains("@db:5432/unitycatalog"),
            "URL resolves UC's database name: {url}"
        ),
        other => panic!("expected a relational_db connection, got {other:?}"),
    }
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

    let p = catalog
        .plan(&Selection::modules(["my-app"]), &PlanCtx::default())
        .unwrap();

    assert!(p.graph.module(&ModuleId::from("postgres")).is_some());
    assert!(p.postgres_databases.contains(&"appdb".to_string()));
    let app_env = p.injected.get(&ModuleId::from("my-app")).unwrap();
    assert_eq!(
        app_env.get("APP_DATABASE_URL"),
        Some("postgresql://postgres:postgres@db:5432/appdb")
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
    let p = catalog
        .plan(&Selection::modules(["svc-a", "svc-b"]), &PlanCtx::default())
        .unwrap();

    // One Postgres provider, both databases provisioned.
    assert_eq!(
        p.graph
            .nodes
            .iter()
            .filter(|m| m.id().as_str() == "postgres")
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
    let err = catalog
        .plan(&Selection::modules(["lonely"]), &PlanCtx::default())
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
    let err = catalog
        .plan(&Selection::modules(["needs-db"]), &PlanCtx::default())
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
    let x_provider = provider_with_needs(
        "x-prov",
        "x",
        relational("x://{name}"),
        vec![ResourceDemand {
            resource: "y".into(),
            name: "y-res".into(),
            provider: None,
            bind: ConnectionBinding::default(),
        }],
    );
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
    let p = catalog
        .plan(&Selection::modules(["top"]), &PlanCtx::default())
        .unwrap();
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
    let p = baseline_catalog()
        .plan(&Selection::modules(["mlflow"]), &PlanCtx::default())
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
    let p = baseline_catalog()
        .plan(&Selection::modules(["mlflow"]), &azurite_preferred())
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

    let s3 = catalog
        .plan(&Selection::modules(["store-app"]), &PlanCtx::default())
        .unwrap();
    assert_eq!(
        s3.injected
            .get(&ModuleId::from("store-app"))
            .and_then(|e| e.get("STORE_URI")),
        Some("s3://artifacts")
    );

    let azure = catalog
        .plan(&Selection::modules(["store-app"]), &azurite_preferred())
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
    let p = catalog
        .plan(&Selection::modules(["pinned-app"]), &azurite_preferred())
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
    let err = catalog
        .plan(&Selection::modules(["needs-store"]), &PlanCtx::default())
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
    let ok = catalog.plan(&Selection::modules(["needs-store"]), &PlanCtx::default());
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
    let p = catalog
        .plan(&Selection::modules(["catalog"]), &PlanCtx::default())
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
            .filter(|m| m.id().as_str() == "seaweedfs")
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
    let p = catalog
        .plan(&Selection::modules(["catalog"]), &PlanCtx::default())
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
    let err = catalog
        .plan(&Selection::modules(["needs-store"]), &PlanCtx::default())
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
    let err = baseline_catalog()
        .plan(
            &Selection::modules([
                "seaweedfs",
                "azurite",
                "mlflow", // demands an object_store, but pins nothing
            ]),
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

// --- environment-level extra resources (provisioned, never bound) ---

/// An extra bucket with no object_store-demanding module still pulls in a provider and is
/// provisioned: nothing in the selection needs an object store, yet seaweedfs joins the graph
/// and the bucket lands in `s3_buckets`.
#[test]
fn extra_resource_pulls_in_a_provider_with_no_consumer() {
    let mut sel = Selection::modules(["envoy"]);
    sel.extra_resources = vec![ExtraResource {
        resource: Role::OBJECT_STORE.into(),
        name: "exports".into(),
        provider: None,
    }];
    let p = baseline_catalog().plan(&sel, &PlanCtx::default()).unwrap();
    assert!(
        p.services.contains_key(&ModuleId::from("seaweedfs")),
        "the object_store provider is pulled into the graph"
    );
    assert_eq!(p.s3_buckets, vec!["exports".to_string()]);
}

/// An extra database is provisioned on postgres and surfaces in `postgres_databases`.
#[test]
fn extra_database_is_provisioned_on_postgres() {
    let mut sel = Selection::modules(["envoy", "postgres"]);
    sel.extra_resources = vec![ExtraResource {
        resource: Role::RELATIONAL_DB.into(),
        name: "analytics".into(),
        provider: None,
    }];
    let p = baseline_catalog().plan(&sel, &PlanCtx::default()).unwrap();
    assert!(p.postgres_databases.contains(&"analytics".to_string()));
}

/// An extra naming a resource a module already demands is deduped — the name appears once.
#[test]
fn extra_resource_dedups_against_a_module_demand() {
    // mlflow demands the `mlflow` database on postgres; an extra naming the same is a no-op.
    let mut sel = Selection::modules(["envoy", "postgres", "mlflow", "seaweedfs"]);
    sel.extra_resources = vec![ExtraResource {
        resource: Role::RELATIONAL_DB.into(),
        name: "mlflow".into(),
        provider: None,
    }];
    let p = baseline_catalog().plan(&sel, &PlanCtx::default()).unwrap();
    let count = p
        .postgres_databases
        .iter()
        .filter(|n| *n == "mlflow")
        .count();
    assert_eq!(count, 1, "deduped: {:?}", p.postgres_databases);
}

/// A pinned extra resource lands on the pinned provider, and a pin introducing a *second*
/// same-role provider is sanctioned (not a ConflictingRoleProviders clash) — mirroring a
/// demand pin.
#[test]
fn pinned_extra_resource_sanctions_a_second_provider() {
    // mlflow demands an object_store → seaweedfs (the default). An extra bucket pins azurite,
    // a deliberate second object_store provider.
    let mut sel = Selection::modules(["envoy", "postgres", "mlflow", "seaweedfs"]);
    sel.extra_resources = vec![ExtraResource {
        resource: Role::OBJECT_STORE.into(),
        name: "exports".into(),
        provider: Some(ModuleId::from("azurite")),
    }];
    let p = baseline_catalog()
        .plan(&sel, &PlanCtx::default())
        .expect("a pinned extra sanctions the second provider");
    assert!(
        p.services.contains_key(&ModuleId::from("azurite")),
        "the pinned provider is deployed"
    );
    assert!(
        p.azure_containers.contains(&"exports".to_string()),
        "the extra bucket is provisioned on the pinned azurite: {:?}",
        p.azure_containers
    );
}
