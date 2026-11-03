//! Neon-hosted placement: service modules run as Neon Functions, and Neon's Postgres and object
//! storage stand in for the in-stack backends, while MLflow and Omnigent run locally.

use olai_stack_topology::{
    FunctionRequirement, HostedConnection, HostedResource, HostedVendor, ModuleId, Plan, PlanCtx,
    PlanError, Secrets, Selection, Vantage, baseline_catalog,
};

fn function() -> HostedResource {
    HostedResource {
        cloud: None,
        vendor: Some(HostedVendor::NeonFunction),
        connection: HostedConnection::Function { url: None },
    }
}

/// MLflow and Omnigent locally; Unity Catalog and Headwaters as functions; Postgres and
/// storage from the branch.
fn hybrid() -> Selection {
    let mut selection = Selection::modules([
        "mlflow",
        "omnigent",
        "unity-catalog",
        "headwaters",
        "hosted-postgres",
        "neon-storage",
    ]);
    selection
        .hosted
        .insert(ModuleId::from("unity-catalog"), function());
    selection
        .hosted
        .insert(ModuleId::from("headwaters"), function());
    selection.hosted.insert(
        ModuleId::from("hosted-postgres"),
        HostedResource {
            cloud: None,
            vendor: Some(HostedVendor::Neon),
            connection: HostedConnection::Postgres {
                host: "${PGHOST}".into(),
                port: 5432,
                database: "{name}".into(),
                username: "${PGUSER}".into(),
                password: "${PGPASSWORD}".into(),
                ssl_mode: None,
            },
        },
    );
    selection
}

/// What binding a branch supplies.
fn branch() -> Secrets {
    [
        ("PGHOST", "ep-x-pooler.example.neon.tech"),
        ("PGUSER", "owner"),
        ("PGPASSWORD", "pg-secret"),
        ("NEON_S3_ENDPOINT", "https://br-x.storage.example.neon.tech"),
        ("NEON_S3_REGION", "eu-central-1"),
        ("NEON_S3_ACCESS_KEY_ID", "key-id"),
        ("NEON_S3_SECRET_ACCESS_KEY", "s3-secret"),
        (
            "UNITYCATALOG_FUNCTION_URL",
            "https://br-x-unitycatalog.compute.example.neon.tech/",
        ),
        (
            "HEADWATERS_FUNCTION_URL",
            "https://br-x-headwaters.compute.example.neon.tech",
        ),
    ]
    .into_iter()
    .collect()
}

fn plan(selection: &Selection, secrets: Option<Secrets>) -> Result<Plan, PlanError> {
    let ctx = PlanCtx {
        env_name: "neon".into(),
        secrets,
        ..PlanCtx::default()
    };
    baseline_catalog().plan(selection, &ctx)
}

fn modules(plan: &Plan) -> Vec<&str> {
    plan.graph.nodes.iter().map(|m| m.id().as_str()).collect()
}

#[test]
fn function_hosted_modules_run_nothing_and_demand_nothing() {
    let plan = plan(&hybrid(), Some(branch())).unwrap();
    let ids = modules(&plan);
    // Unity Catalog's Postgres and object-store demands are the function's business: no
    // in-stack Postgres or RustFS joins for them, and neither does anything for Headwaters.
    for absent in ["postgres", "rustfs"] {
        assert!(!ids.contains(&absent), "{absent} in {ids:?}");
    }
    for id in ["unity-catalog", "headwaters"] {
        assert!(ids.contains(&id), "{id} in {ids:?}");
        assert!(plan.stack_facts.functions.contains_key(id));
        assert!(!plan.stack_facts.runs(id));
    }
    assert!(plan.stack_facts.runs("mlflow"));

    let out = plan.materialize().unwrap();
    let compose: Vec<&str> = out
        .files
        .iter()
        .map(|f| f.path.as_str())
        .filter(|p| p.ends_with("compose.yaml"))
        .collect();
    assert!(
        compose
            .iter()
            .all(|p| !p.contains("unity-catalog") && !p.contains("headwaters")),
        "{compose:?}"
    );
    // MLflow's registry is the function's, and it doesn't wait for a local UC.
    let mlflow = out
        .files
        .iter()
        .find(|f| f.path == "modules/mlflow/compose.yaml")
        .unwrap();
    assert!(
        mlflow
            .contents
            .contains("uc:https://br-x-unitycatalog.compute.example.neon.tech"),
        "{}",
        mlflow.contents
    );
    assert!(
        !mlflow.contents.contains("unitycatalog:"),
        "{}",
        mlflow.contents
    );
    // A function's URL is an address, not a credential: the fragment holding it can still move
    // its credentials into the launch env.
    let (_, launch) = plan.materialize().unwrap().into_launch_env().unwrap();
    assert!(
        !launch
            .into_vars()
            .values()
            .any(|v| v.contains("compute.example"))
    );
}

#[test]
fn dependants_reach_the_functions_at_their_urls() {
    let plan = plan(&hybrid(), Some(branch())).unwrap();
    for from in [Vantage::Host, Vantage::Container] {
        let env = plan.client_env(from);
        assert_eq!(
            env.get("UNITY_CATALOG_URL"),
            Some("https://br-x-unitycatalog.compute.example.neon.tech/api/2.1/unity-catalog")
        );
        assert_eq!(
            env.get("MLFLOW_REGISTRY_URI"),
            Some("uc:https://br-x-unitycatalog.compute.example.neon.tech")
        );
        assert_eq!(
            env.get("OPENLINEAGE_URL"),
            Some("https://br-x-headwaters.compute.example.neon.tech")
        );
        assert_eq!(env.get("OPENLINEAGE_ENDPOINT"), Some("/api/v1/lineage"));
    }
}

#[test]
fn a_preview_keeps_the_function_urls_as_references() {
    let plan = plan(&hybrid(), None).unwrap();
    let env = plan.client_env(Vantage::Host);
    assert_eq!(
        env.get("UNITY_CATALOG_URL"),
        Some("${UNITYCATALOG_FUNCTION_URL}/api/2.1/unity-catalog")
    );
    assert_eq!(
        env.get("MLFLOW_REGISTRY_URI"),
        Some("uc:${UNITYCATALOG_FUNCTION_URL}")
    );
    assert_eq!(
        env.get("OPENLINEAGE_URL"),
        Some("${HEADWATERS_FUNCTION_URL}")
    );
    // A branch the plan needs but nobody bound is reported, every variable at once.
    let Err(PlanError::UnresolvedSecrets { vars }) = plan_with_partial_branch() else {
        panic!("expected unresolved secrets");
    };
    assert!(
        vars.contains(&"UNITYCATALOG_FUNCTION_URL".to_string()),
        "{vars:?}"
    );
    assert!(vars.contains(&"NEON_S3_ENDPOINT".to_string()), "{vars:?}");
}

fn plan_with_partial_branch() -> Result<Plan, PlanError> {
    let secrets: Secrets = [("PGHOST", "h"), ("PGUSER", "u"), ("PGPASSWORD", "p")]
        .into_iter()
        .collect();
    plan(&hybrid(), Some(secrets))
}

#[test]
fn the_plan_lists_what_the_branch_must_supply() {
    let req = plan(&hybrid(), None).unwrap().neon_requirements();
    assert_eq!(req.databases, ["mlflow", "omnigent"]);
    assert_eq!(req.buckets, ["mlflow"]);
    assert_eq!(
        req.functions,
        [
            FunctionRequirement {
                module: ModuleId::from("headwaters"),
                slug: "headwaters".into(),
                url_var: Some("HEADWATERS_FUNCTION_URL".into()),
            },
            FunctionRequirement {
                module: ModuleId::from("unity-catalog"),
                slug: "unitycatalog".into(),
                url_var: Some("UNITYCATALOG_FUNCTION_URL".into()),
            },
        ]
    );
    for var in [
        "HEADWATERS_FUNCTION_URL",
        "NEON_S3_ACCESS_KEY_ID",
        "PGPASSWORD",
        "UNITYCATALOG_FUNCTION_URL",
    ] {
        assert!(req.vars.iter().any(|v| v == var), "{var} in {:?}", req.vars);
    }

    // A fully local environment needs no branch.
    let local = plan(&Selection::modules(["mlflow", "unity-catalog"]), None).unwrap();
    assert!(local.neon_requirements().is_empty());
}

#[test]
fn a_fully_local_environment_is_unchanged() {
    let local = plan(
        &Selection::modules(["mlflow", "unity-catalog", "headwaters"]),
        None,
    )
    .unwrap();
    let ids = modules(&local);
    assert!(
        ids.contains(&"postgres") && ids.contains(&"rustfs"),
        "{ids:?}"
    );
    assert!(local.stack_facts.functions.is_empty());
    assert_eq!(
        local.client_env(Vantage::Host).get("UNITY_CATALOG_URL"),
        Some("http://localhost:9080/api/2.1/unity-catalog")
    );
}

#[test]
fn only_a_function_capable_module_runs_as_one() {
    let mut selection = Selection::modules(["mlflow"]);
    selection
        .hosted
        .insert(ModuleId::from("mlflow"), function());
    let err = plan(&selection, None).unwrap_err();
    assert!(
        matches!(&err, PlanError::InvalidHosted { module, reason }
            if module.as_str() == "mlflow" && reason.contains("function")),
        "{err:?}"
    );

    // A function needs the function vendor, and the function vendor runs nothing else.
    let mut selection = Selection::modules(["unity-catalog"]);
    selection.hosted.insert(
        ModuleId::from("unity-catalog"),
        HostedResource {
            vendor: Some(HostedVendor::Neon),
            ..function()
        },
    );
    assert!(matches!(
        plan(&selection, None),
        Err(PlanError::InvalidHosted { .. })
    ));
    let mut selection = Selection::modules(["hosted-postgres"]);
    selection.hosted.insert(
        ModuleId::from("hosted-postgres"),
        HostedResource {
            cloud: None,
            vendor: Some(HostedVendor::NeonFunction),
            connection: HostedConnection::Postgres {
                host: "h".into(),
                port: 5432,
                database: "{name}".into(),
                username: "u".into(),
                password: "p".into(),
                ssl_mode: None,
            },
        },
    );
    assert!(matches!(
        plan(&selection, None),
        Err(PlanError::InvalidHosted { .. })
    ));
}

#[test]
fn an_explicit_function_url_is_used_as_is() {
    let mut selection = hybrid();
    selection.hosted.insert(
        ModuleId::from("headwaters"),
        HostedResource {
            connection: HostedConnection::Function {
                url: Some("https://lineage.example.com".into()),
            },
            ..function()
        },
    );
    let plan = plan(&selection, Some(branch())).unwrap();
    assert_eq!(
        plan.client_env(Vantage::Host).get("OPENLINEAGE_URL"),
        Some("https://lineage.example.com")
    );
    assert!(
        plan.neon_requirements()
            .functions
            .iter()
            .any(|f| f.slug == "headwaters" && f.url_var.is_none())
    );
}
