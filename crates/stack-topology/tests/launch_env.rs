//! Rendering for a runtime that keeps credentials off disk: every baseline module, with a
//! sentinel in every local credential and hosted secret, materializes with no sentinel in any
//! file, and the launch environment carries them all.

use olai_stack_topology::{
    HostedConnection, HostedResource, LOCAL_CREDENTIALS, LocalCredentials, MaterializedOutput,
    ModuleId, PlanCtx, Secrets, Selection, baseline_catalog_with,
};

/// A sentinel per local credential, valid for each (alphanumeric, within its length bounds).
fn sentinels() -> (LocalCredentials, Vec<String>) {
    let mut creds = LocalCredentials::new();
    let mut values = Vec::new();
    for (i, c) in LOCAL_CREDENTIALS.iter().enumerate() {
        let value = format!("SENTINEL{i}{}", "x".repeat(40))[..c.max_len.min(48)].to_owned();
        creds.set(c.key, &value).unwrap();
        values.push(value);
    }
    (creds, values)
}

fn assert_clean(out: &MaterializedOutput, sentinels: &[String]) {
    // Every rewritten env file must be interpolated: none of them is read raw any more.
    let fragments: String = out
        .files
        .iter()
        .filter(|f| f.path.ends_with("compose.yaml"))
        .map(|f| f.contents.as_str())
        .collect();
    for f in out.files.iter().filter(|f| f.path.contains("/.env/")) {
        let entry = format!("- path: ./{}\n", f.path);
        for (i, _) in fragments.match_indices(&entry) {
            let next = fragments[i + entry.len()..].lines().next().unwrap_or("");
            assert_ne!(next.trim(), "format: raw", "{} is still read raw", f.path);
        }
    }
    for f in &out.files {
        assert!(!f.sensitive, "{} still sensitive", f.path);
        for s in sentinels {
            assert!(!f.contents.contains(s.as_str()), "{s} in {}", f.path);
        }
    }
}

#[test]
fn local_credentials_leave_no_trace_in_the_tree() {
    let (creds, values) = sentinels();
    let catalog = baseline_catalog_with(&creds);
    let mut selection = Selection::modules([
        "envoy",
        "authelia",
        "postgres",
        "rustfs",
        "mlflow",
        "unity-catalog",
        "jaeger",
        "headwaters",
        "omnigent",
        "marimo",
        "puppygraph",
        "rivers",
    ]);
    selection.gateway.auth = true;
    selection.gateway.policy = true;
    selection.client.python = true;
    let out = catalog
        .plan(&selection, &PlanCtx::default())
        .unwrap()
        .materialize()
        .unwrap();
    assert!(out.files.iter().any(|f| f.sensitive));
    let (out, env) = out.into_launch_env().unwrap();
    assert_clean(&out, &values);
    for value in &values {
        assert!(
            env.vars().values().any(|v| v.contains(value.as_str())),
            "{value} lost"
        );
    }
    // Azurite instead of RustFS.
    let selection = Selection::modules(["envoy", "postgres", "azurite", "mlflow", "unity-catalog"]);
    let (out, _) = catalog
        .plan(&selection, &PlanCtx::default())
        .unwrap()
        .materialize()
        .unwrap()
        .into_launch_env()
        .unwrap();
    assert_clean(&out, &values);
}

#[test]
fn hosted_secrets_leave_no_trace_in_the_tree() {
    let mut selection = Selection::modules([
        "envoy",
        "hosted-postgres",
        "aws-s3",
        "unity-catalog",
        "mlflow",
    ]);
    selection.hosted.insert(
        ModuleId::from("hosted-postgres"),
        HostedResource {
            cloud: Some("aws".into()),
            vendor: None,
            connection: HostedConnection::Postgres {
                host: "db.example.com".into(),
                port: 5432,
                database: "{name}".into(),
                username: "app".into(),
                password: "${PG_PASSWORD}".into(),
                ssl_mode: None,
            },
        },
    );
    selection.hosted.insert(
        ModuleId::from("aws-s3"),
        HostedResource {
            cloud: Some("aws".into()),
            vendor: None,
            connection: HostedConnection::S3 {
                region: "us-east-1".into(),
                endpoint: None,
                bucket: "{name}".into(),
                access_key_id: "${AWS_ACCESS_KEY_ID}".into(),
                secret_access_key: "${AWS_SECRET_ACCESS_KEY}".into(),
                role_arn: None,
            },
        },
    );
    let values = ["HOSTEDPGSENTINEL", "HOSTEDAKSENTINEL", "HOSTEDSKSENTINEL"];
    let secrets: Secrets = ["PG_PASSWORD", "AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"]
        .into_iter()
        .zip(values)
        .collect();
    let ctx = PlanCtx {
        secrets: Some(secrets),
        ..PlanCtx::default()
    };
    let (creds, local) = sentinels();
    let out = baseline_catalog_with(&creds)
        .plan(&selection, &ctx)
        .unwrap()
        .materialize()
        .unwrap();
    let (out, env) = out.into_launch_env().unwrap();
    let all: Vec<String> = values.iter().map(|s| s.to_string()).chain(local).collect();
    assert_clean(&out, &all);
    for value in values {
        assert!(
            env.vars().values().any(|v| v.contains(value)),
            "{value} lost"
        );
    }
}
