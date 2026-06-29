//! Tests for the resource-demand layer: a module declares a `(kind, name)` it needs,
//! and the planner auto-deploys a provider, provisions the named resource, and injects
//! the provider's coordinates back into the consumer.

use olai_stack_topology::{
    Catalog, Injection, Module, ModuleId, Placement, PlanCtx, PlanError, Provides, RenderSpec,
    ResourceDemand, ResourceProvider, Role, Selection, ServiceSpec, baseline_catalog, plan,
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
    // UC declares it needs a postgres_database + s3_bucket; selecting *only* UC must
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
            resource: "postgres_database".into(),
            name: "appdb".into(),
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
            resource: "postgres_database".into(),
            name: "a_db".into(),
            inject: vec![],
        }],
    );
    let b = consumer(
        "svc-b",
        vec![ResourceDemand {
            resource: "postgres_database".into(),
            name: "b_db".into(),
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
    let p1 = provider("pg-a", "postgres_database", &[("url", "a://{name}")]);
    let p2 = provider("pg-b", "postgres_database", &[("url", "b://{name}")]);
    let c = consumer(
        "needs-db",
        vec![ResourceDemand {
            resource: "postgres_database".into(),
            name: "db".into(),
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
            assert_eq!(resource, "postgres_database");
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
        inject: vec![],
    }];
    let y_provider = provider("y-prov", "y", &[("v", "{name}")]);
    let c = consumer(
        "top",
        vec![ResourceDemand {
            resource: "x".into(),
            name: "x-res".into(),
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
