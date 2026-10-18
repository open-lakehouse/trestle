//! Deployment steps: every module's steps and the target's, rendered in deployment order into
//! the environment's README and `deploy/` scripts.

use std::collections::BTreeMap;

use olai_stack_topology::{
    DEPLOY_SHELL_SETUP, DeployPhase, HostedConnection, HostedKind, HostedResource, HostedVendor,
    MaterializedOutput, ModuleId, Plan, PlanCtx, PlanError, Secrets, Selection, baseline_catalog,
};

/// Neon Postgres and S3 behind UC and MLflow.
fn selection(vendor: HostedVendor) -> Selection {
    let mut selection = Selection::modules([
        "envoy",
        "hosted-postgres",
        "aws-s3",
        "unity-catalog",
        "mlflow",
    ]);
    let HostedConnection::Postgres {
        host,
        port,
        database,
        username,
        password,
        ssl_mode,
    } = HostedKind::Postgres.default_connection()
    else {
        unreachable!("a postgres default");
    };
    selection.hosted.insert(
        ModuleId::from("hosted-postgres"),
        HostedResource {
            cloud: None,
            vendor: Some(vendor),
            connection: HostedConnection::Postgres {
                host,
                port,
                database,
                username,
                password,
                ssl_mode,
            },
        },
    );
    selection
}

fn secrets() -> Secrets {
    [
        ("PGHOST", "ep-cool-fog-123456.us-east-2.aws.neon.tech"),
        ("PGUSER", "neondb_owner"),
        ("PGPASSWORD", "pg-secret-value"),
        ("AWS_ACCESS_KEY_ID", "AKIAEXAMPLE"),
        ("AWS_SECRET_ACCESS_KEY", "aws-secret-value"),
    ]
    .into_iter()
    .collect()
}

fn plan(vendor: HostedVendor, secrets: Option<Secrets>) -> Plan {
    baseline_catalog()
        .plan(
            &selection(vendor),
            &PlanCtx {
                env_name: "lake".into(),
                secrets,
                ..PlanCtx::default()
            },
        )
        .expect("plans")
}

fn file<'a>(out: &'a MaterializedOutput, path: &str) -> &'a str {
    out.files
        .iter()
        .find(|f| f.path == path)
        .map(|f| f.contents.as_str())
        .unwrap_or_else(|| panic!("expected {path}"))
}

#[test]
fn steps_run_in_phase_then_dependency_order_with_the_targets_last() {
    let out = plan(HostedVendor::Neon, Some(secrets()))
        .materialize()
        .unwrap();
    let order: Vec<(DeployPhase, String)> = out
        .deploy
        .iter()
        .map(|s| {
            let owner = s.module.as_ref().map_or("compose", |m| m.as_str());
            (s.phase, format!("{owner}/{}", s.id))
        })
        .collect();
    let names: Vec<&str> = order.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "aws-s3/buckets",
            "hosted-postgres/server",
            "hosted-postgres/databases",
            "compose/secrets",
            "compose/up",
        ],
        "{order:?}"
    );
    assert!(order.windows(2).all(|w| w[0].0 <= w[1].0));
    assert_eq!(
        out.deploy.iter().map(|s| s.order).collect::<Vec<_>>(),
        [1, 2, 3, 4, 5]
    );

    let readme = file(&out, "README.md");
    let at = |needle: &str| {
        readme
            .find(needle)
            .unwrap_or_else(|| panic!("{needle} in\n{readme}"))
    };
    assert!(at("## Provision") < at("### 1. Create the buckets (`aws-s3`)"));
    assert!(
        at("### 2. Create the Neon project (`hosted-postgres`)")
            < at("### 3. Create the databases")
    );
    assert!(at("## Configure") < at("### 4. Supply the hosted resources' secrets"));
    assert!(at("## Start") < at("docker compose up -d --wait"));
    assert!(
        readme.contains("bash deploy/hosted-postgres-server.sh"),
        "{readme}"
    );
    assert!(readme.contains("`PGPASSWORD`"), "{readme}");
}

#[test]
fn each_script_is_written_where_the_readme_says() {
    let out = plan(HostedVendor::Neon, Some(secrets()))
        .materialize()
        .unwrap();
    for step in &out.deploy {
        if let Some(script) = &step.script {
            assert_eq!(file(&out, &script.path), script.contents);
            assert!(script.contents.starts_with("#!/usr/bin/env bash\n"));
            assert!(script.contents.contains("set -euo pipefail"));
        }
    }
    let databases = file(&out, "deploy/hosted-postgres-databases.sh");
    // Neon's databases are created through its API, one per demanded name.
    assert!(
        databases.contains("neonctl databases create"),
        "{databases}"
    );
    for name in ["'unitycatalog'", "'mlflow'"] {
        assert!(databases.contains(name), "{name} in\n{databases}");
    }
}

#[test]
fn a_scripts_commands_are_its_body_after_the_shell_setup() {
    let out = plan(HostedVendor::Neon, Some(secrets()))
        .materialize()
        .unwrap();
    let mut seen = 0;
    for script in out.deploy.iter().filter_map(|s| s.script.as_ref()) {
        // Pasted into an interactive shell, neither strict mode nor `$0` belongs.
        for wrapper in ["set -euo pipefail", "$0", "#!/usr/bin/env bash"] {
            assert!(!script.body.contains(wrapper), "{}: {wrapper}", script.path);
        }
        assert!(!script.body.contains("save_secret() {"), "{}", script.path);
        let setup_then_body = format!("{DEPLOY_SHELL_SETUP}\n{}", script.body);
        assert!(
            script.contents.ends_with(&setup_then_body),
            "{}:\n{}",
            script.path,
            script.contents
        );
        seen += 1;
    }
    assert!(seen >= 3, "saw {seen} scripts");
}

#[test]
fn steps_never_contain_a_secret_value() {
    let out = plan(HostedVendor::AwsRds, Some(secrets()))
        .materialize()
        .unwrap();
    let values = [
        "pg-secret-value",
        "AKIAEXAMPLE",
        "aws-secret-value",
        "ep-cool-fog",
    ];
    for step in &out.deploy {
        let script = step.script.as_ref().map_or("", |s| s.contents.as_str());
        for value in values {
            assert!(
                !step.instructions.contains(value) && !script.contains(value),
                "step {} leaks {value}",
                step.id
            );
        }
    }
    for path in ["README.md", "deploy/hosted-postgres-server.sh"] {
        let text = file(&out, path);
        assert!(values.iter().all(|v| !text.contains(v)), "{path}:\n{text}");
    }
}

#[test]
fn rendered_scripts_are_valid_bash() {
    let Ok(status) = std::process::Command::new("bash").arg("--version").output() else {
        eprintln!("no bash; skipping the syntax check");
        return;
    };
    assert!(status.status.success());
    let dir = std::env::temp_dir().join(format!("trestle-deploy-steps-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut checked = 0;
    let mut azure = Selection::modules(["envoy", "postgres", "azure-storage", "unity-catalog"]);
    azure.hosted.insert(
        ModuleId::from("azure-storage"),
        HostedKind::AzureBlob.default_resource(),
    );
    let mut s3_compatible = selection(HostedVendor::Generic);
    s3_compatible.hosted.insert(
        ModuleId::from("aws-s3"),
        HostedResource {
            cloud: None,
            vendor: Some(HostedVendor::Generic),
            connection: HostedConnection::S3 {
                region: "auto".into(),
                endpoint: Some("https://acct.r2.cloudflarestorage.com".into()),
                bucket: "lake-{name}".into(),
                access_key_id: "${R2_KEY}".into(),
                secret_access_key: "${R2_SECRET}".into(),
                role_arn: None,
            },
        },
    );
    let selections = HostedVendor::ALL
        .into_iter()
        .filter(|v| v.offers(HostedKind::Postgres))
        .map(selection)
        .chain([azure, s3_compatible]);
    for selection in selections {
        let out = baseline_catalog()
            .plan(&selection, &PlanCtx::default())
            .unwrap()
            .materialize()
            .unwrap();
        for script in out.deploy.iter().filter_map(|s| s.script.as_ref()) {
            let path = dir.join(script.path.replace('/', "_"));
            std::fs::write(&path, &script.contents).unwrap();
            let check = std::process::Command::new("bash")
                .arg("-n")
                .arg(&path)
                .output()
                .unwrap();
            assert!(
                check.status.success(),
                "{}: {}\n{}",
                script.path,
                String::from_utf8_lossy(&check.stderr),
                script.contents
            );
            checked += 1;
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(checked >= 7, "checked {checked} scripts");
}

#[test]
fn a_generic_postgres_server_is_described_not_scripted() {
    let out = plan(HostedVendor::Generic, None).materialize().unwrap();
    let server = out
        .deploy
        .iter()
        .find(|s| s.id == "server")
        .expect("the server step");
    assert_eq!(server.title, "Provide the Postgres server");
    assert!(server.script.is_none());
    // Databases are created the same way on any server.
    let databases = file(&out, "deploy/hosted-postgres-databases.sh");
    assert!(databases.contains("CREATE DATABASE"), "{databases}");
    assert!(
        databases.contains("export PGHOST=\"${PGHOST}\""),
        "{databases}"
    );
}

#[test]
fn managed_postgres_vendors_connect_over_tls() {
    let plan = plan(HostedVendor::Neon, Some(secrets()));
    let hosted = &plan.hosted[&ModuleId::from("hosted-postgres")];
    assert_eq!(hosted.cloud.as_deref(), Some("neon"));
    let HostedConnection::Postgres { ssl_mode, .. } = &hosted.connection else {
        panic!("postgres");
    };
    assert_eq!(ssl_mode.as_deref(), Some("require"));
    let out = plan.materialize().unwrap();
    let mlflow = file(&out, "modules/mlflow/.env/mlflow.env");
    assert!(mlflow.contains("/mlflow?sslmode=require"), "{mlflow}");
    let hibernate = file(&out, "modules/unity-catalog/secrets/hibernate.properties");
    assert!(
        hibernate.contains("/unitycatalog?sslmode=require"),
        "{hibernate}"
    );

    // An unspecified server keeps the client's default.
    let out = self::plan(HostedVendor::Generic, Some(secrets()))
        .materialize()
        .unwrap();
    assert!(!file(&out, "modules/mlflow/.env/mlflow.env").contains("sslmode"));
}

#[test]
fn a_vendor_must_offer_its_modules_kind() {
    let mut selection = selection(HostedVendor::Neon);
    selection.hosted.insert(
        ModuleId::from("aws-s3"),
        HostedResource {
            vendor: Some(HostedVendor::Neon),
            ..HostedKind::S3.default_resource()
        },
    );
    let err = baseline_catalog()
        .plan(&selection, &PlanCtx::default())
        .unwrap_err();
    assert!(
        matches!(&err, PlanError::InvalidHosted { module, .. } if module.as_str() == "aws-s3"),
        "{err:?}"
    );
}

#[test]
fn a_stack_without_hosted_resources_still_says_how_to_start() {
    let out = baseline_catalog()
        .plan(
            &Selection::modules(["postgres", "mlflow"]),
            &PlanCtx::default(),
        )
        .unwrap()
        .materialize()
        .unwrap();
    let ids: BTreeMap<&str, DeployPhase> = out
        .deploy
        .iter()
        .map(|s| (s.id.as_str(), s.phase))
        .collect();
    assert_eq!(ids, BTreeMap::from([("up", DeployPhase::Start)]));
    assert!(!out.files.iter().any(|f| f.path.starts_with("deploy/")));
    assert!(file(&out, "README.md").contains("docker compose up -d --wait"));
}
