//! Tests for the resource-demand layer: a module declares a `(kind, name)` it needs,
//! and the planner auto-deploys a provider, provisions the named resource, and injects
//! the provider's coordinates back into the consumer.

use std::collections::BTreeMap;

use olai_stack_topology::{
    Catalog, Injection, Module, ModuleId, Placement, PlanCtx, PlanError, Provides, RenderSpec,
    ResourceDemand, ResourceProvider, Role, RoleContract, Selection, ServiceSpec, baseline_catalog,
    plan,
};

/// Build a minimal provider module that provisions `kind` and vends the given
/// coordinate templates.
fn provider(id: &str, kind: &str, coordinates: &[(&str, &str)]) -> Module {
    let mut rp = ResourceProvider::default();
    for (name, tmpl) in coordinates {
        rp.coordinates.insert((*name).into(), (*tmpl).into());
    }
    let mut provides = Provides::default();
    provides.resource_kinds.insert(kind.into(), rp);
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
        &Selection::modules(["local-stack-unity-catalog"]),
        &baseline_catalog(),
        &PlanCtx::default(),
    )
    .expect("UC alone should plan, auto-provisioning its providers");

    for id in [
        "local-stack-unity-catalog",
        "local-stack-postgres",
        "local-stack-seaweedfs",
        "local-stack-envoy",
    ] {
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
    assert!(pos("local-stack-postgres") < pos("local-stack-unity-catalog"));
    assert!(pos("local-stack-seaweedfs") < pos("local-stack-unity-catalog"));
}

#[test]
fn provider_coordinate_is_injected_into_the_consumer() {
    let p = plan(
        &Selection::modules(["local-stack-unity-catalog"]),
        &baseline_catalog(),
        &PlanCtx::default(),
    )
    .unwrap();

    // UC's demand injects the connection-string coordinate as UC_DATABASE_URL, with
    // {name} resolved to the demanded database name.
    let uc_env = p
        .injected
        .get(&ModuleId::from("local-stack-unity-catalog"))
        .unwrap();
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
        "injected coordinate must also land in the stack .env"
    );
}

#[test]
fn an_app_module_demanding_postgres_gets_a_provider_and_its_url() {
    // A templated app declares it needs a Postgres database — same mechanism as the
    // infra modules. The planner auto-provisions Postgres and injects the URL.
    let app = consumer(
        "my-app",
        vec![ResourceDemand {
            resource: "relational_db".into(),
            name: "appdb".into(),
            provider: None,
            inject: vec![Injection {
                key: "APP_DATABASE_URL".into(),
                coordinate: "url".into(),
            }],
        }],
    );
    let catalog = baseline_catalog().merge(Catalog::from_modules([app]));

    let p = plan(
        &Selection::modules(["my-app"]),
        &catalog,
        &PlanCtx::default(),
    )
    .unwrap();

    assert!(
        p.graph
            .module(&ModuleId::from("local-stack-postgres"))
            .is_some()
    );
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
            inject: vec![],
        }],
    );
    let b = consumer(
        "svc-b",
        vec![ResourceDemand {
            resource: "relational_db".into(),
            name: "b_db".into(),
            provider: None,
            inject: vec![],
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
            .filter(|m| m.id.as_str() == "local-stack-postgres")
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
            inject: vec![],
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
    let p1 = provider("pg-a", "relational_db", &[("url", "a://{name}")]);
    let p2 = provider("pg-b", "relational_db", &[("url", "b://{name}")]);
    let c = consumer(
        "needs-db",
        vec![ResourceDemand {
            resource: "relational_db".into(),
            name: "db".into(),
            provider: None,
            inject: vec![],
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
    let mut x_provider = provider("x-prov", "x", &[("v", "{name}")]);
    x_provider.needs = vec![ResourceDemand {
        resource: "y".into(),
        name: "y-res".into(),
        provider: None,
        inject: vec![],
    }];
    let y_provider = provider("y-prov", "y", &[("v", "{name}")]);
    let c = consumer(
        "top",
        vec![ResourceDemand {
            resource: "x".into(),
            name: "x-res".into(),
            provider: None,
            inject: vec![],
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
        vec![
            ModuleId::from("local-stack-azurite"),
            ModuleId::from("local-stack-seaweedfs"),
        ],
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
        &Selection::modules(["local-stack-mlflow"]),
        &baseline_catalog(),
        &PlanCtx::default(),
    )
    .unwrap();
    assert!(
        p.graph
            .module(&ModuleId::from("local-stack-seaweedfs"))
            .is_some()
    );
    assert!(
        p.graph
            .module(&ModuleId::from("local-stack-azurite"))
            .is_none()
    );
    assert!(p.s3_buckets.contains(&"mlflow".to_string()));
    assert!(p.azure_containers.is_empty());
    assert_eq!(p.env.get("AWS_ACCESS_KEY_ID"), Some("seaweedfs"));
    assert_eq!(p.env.get("AZURE_STORAGE_CONNECTION_STRING"), None);
}

#[test]
fn preference_selects_azurite_and_drops_aws() {
    // An Azurite-preferred environment: MLflow's object_store demand resolves to Azurite.
    let p = plan(
        &Selection::modules(["local-stack-mlflow"]),
        &baseline_catalog(),
        &azurite_preferred(),
    )
    .unwrap();

    // Azurite is deployed, SeaweedFS is not.
    assert!(
        p.graph
            .module(&ModuleId::from("local-stack-azurite"))
            .is_some()
    );
    assert!(
        p.graph
            .module(&ModuleId::from("local-stack-seaweedfs"))
            .is_none()
    );
    // The container (not an S3 bucket) is provisioned on Azurite.
    assert!(p.azure_containers.contains(&"mlflow".to_string()));
    assert!(p.s3_buckets.is_empty());
    // Azure creds present; no AWS_* leak (SeaweedFS isn't in the graph).
    assert!(p.env.get("AZURE_STORAGE_CONNECTION_STRING").is_some());
    assert_eq!(p.env.get("AWS_ACCESS_KEY_ID"), None);
}

#[test]
fn consumer_uri_follows_the_chosen_provider() {
    // A consumer that injects the object_store `uri` coordinate gets the S3-shaped value
    // by default and the Azure-shaped value under an Azurite preference.
    let app = consumer(
        "store-app",
        vec![ResourceDemand {
            resource: "object_store".into(),
            name: "artifacts".into(),
            provider: None,
            inject: vec![Injection {
                key: "STORE_URI".into(),
                coordinate: "uri".into(),
            }],
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
            provider: Some(ModuleId::from("local-stack-seaweedfs")),
            inject: vec![Injection {
                key: "STORE_URI".into(),
                coordinate: "uri".into(),
            }],
        }],
    );
    let catalog = baseline_catalog().merge(Catalog::from_modules([app]));
    let p = plan(
        &Selection::modules(["pinned-app"]),
        &catalog,
        &azurite_preferred(),
    )
    .unwrap();
    assert!(
        p.graph
            .module(&ModuleId::from("local-stack-seaweedfs"))
            .is_some()
    );
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
    let mut s3 = ResourceProvider {
        provider_kind: Some("s3".into()),
        ..Default::default()
    };
    s3.coordinates.insert("uri".into(), "s3://{name}".into());
    let mut a = provider("prov-s3", "object_store", &[]);
    a.provides
        .resource_kinds
        .insert("object_store".into(), s3.clone());
    let mut b = provider("prov-azure", "object_store", &[]);
    b.provides.resource_kinds.insert("object_store".into(), s3);
    let c = consumer(
        "needs-store",
        vec![ResourceDemand {
            resource: "object_store".into(),
            name: "x".into(),
            provider: None,
            inject: vec![],
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
        {
            let mut a = provider("prov-s3", "object_store", &[]);
            a.provides.resource_kinds.insert(
                "object_store".into(),
                ResourceProvider {
                    provider_kind: Some("s3".into()),
                    ..Default::default()
                },
            );
            a
        },
        provider("prov-azure", "object_store", &[]),
        consumer(
            "needs-store",
            vec![ResourceDemand {
                resource: "object_store".into(),
                name: "x".into(),
                provider: None,
                inject: vec![],
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
    // for an external location. Same role, distinct names, distinct inject keys.
    let uc = consumer(
        "catalog",
        vec![
            ResourceDemand {
                resource: "object_store".into(),
                name: "uc-managed".into(),
                provider: None,
                inject: vec![Injection {
                    key: "UC_MANAGED_URI".into(),
                    coordinate: "uri".into(),
                }],
            },
            ResourceDemand {
                resource: "object_store".into(),
                name: "uc-external".into(),
                provider: None,
                inject: vec![Injection {
                    key: "UC_EXTERNAL_URI".into(),
                    coordinate: "uri".into(),
                }],
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

    // Each demand injects its own coordinate under its own key — no collision.
    let env = p.injected.get(&ModuleId::from("catalog")).unwrap();
    assert_eq!(env.get("UC_MANAGED_URI"), Some("s3://uc-managed"));
    assert_eq!(env.get("UC_EXTERNAL_URI"), Some("s3://uc-external"));

    // One object-store provider serves both demands.
    assert_eq!(
        p.graph
            .nodes
            .iter()
            .filter(|m| m.id.as_str() == "local-stack-seaweedfs")
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
                provider: Some(ModuleId::from("local-stack-seaweedfs")),
                inject: vec![Injection {
                    key: "UC_MANAGED_URI".into(),
                    coordinate: "uri".into(),
                }],
            },
            ResourceDemand {
                resource: "object_store".into(),
                name: "uc-external".into(),
                provider: Some(ModuleId::from("local-stack-azurite")),
                inject: vec![Injection {
                    key: "UC_EXTERNAL_URI".into(),
                    coordinate: "uri".into(),
                }],
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
fn provider_missing_a_required_coordinate_fails_the_contract() {
    // A registered role contract requires `endpoint`, but this provider only renders
    // `uri` — planning must fail at the contract check, naming the missing coordinate.
    let store = provider("half-store", "object_store", &[("uri", "x://{name}")]);
    let app = consumer(
        "needs-store",
        vec![ResourceDemand {
            resource: "object_store".into(),
            name: "data".into(),
            provider: None,
            inject: vec![],
        }],
    );
    let catalog = Catalog::from_modules([store, app]).with_role_contract(RoleContract::new(
        Role::object_store(),
        [
            ResourceProvider::URI_COORDINATE,
            ResourceProvider::ENDPOINT_COORDINATE,
        ],
    ));
    let err = plan(
        &Selection::modules(["needs-store"]),
        &catalog,
        &PlanCtx::default(),
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            PlanError::IncompleteProviderContract { ref role, ref coordinate, .. }
                if role == "object_store" && coordinate == "endpoint"
        ),
        "expected IncompleteProviderContract for the missing endpoint, got {err:?}"
    );
}

#[test]
fn baseline_object_store_providers_satisfy_their_contract() {
    // Both default and Azurite-preferred plans pass the baseline's object_store and
    // relational_db contracts (no IncompleteProviderContract).
    assert!(
        plan(
            &Selection::modules(["local-stack-unity-catalog", "local-stack-mlflow"]),
            &baseline_catalog(),
            &PlanCtx::default(),
        )
        .is_ok()
    );
    assert!(
        plan(
            &Selection::modules(["local-stack-unity-catalog", "local-stack-mlflow"]),
            &baseline_catalog(),
            &azurite_preferred(),
        )
        .is_ok()
    );
}

#[test]
fn two_unpinned_object_store_providers_in_one_env_is_rejected() {
    // Selecting both object_store providers directly, with no demand pin to sanction the
    // pair, is an unpinned same-role clash: the planner refuses to silently pick one.
    let err = plan(
        &Selection::modules([
            "local-stack-seaweedfs",
            "local-stack-azurite",
            "local-stack-mlflow", // demands an object_store, but pins nothing
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
                    && providers.contains(&ModuleId::from("local-stack-seaweedfs"))
                    && providers.contains(&ModuleId::from("local-stack-azurite"))
        ),
        "expected ConflictingRoleProviders for the unpinned object_store pair, got {err:?}"
    );
}
