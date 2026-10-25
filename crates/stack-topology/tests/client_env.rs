//! The client environment (`Plan::client_env`, written to `client/*.env`): what an app,
//! notebook, or engine loads to reach the stack. Addresses must follow the plan's routes and
//! the caller's vantage, and the file must never carry a credential.

use olai_stack_topology::{
    CLIENT_CONTAINER_ENV_PATH, CLIENT_HOST_ENV_PATH, Connection, ObjectStoreCredential, Plan,
    PlanCtx, Selection, Vantage, baseline_catalog,
};

fn plan(selection: Selection, ctx: PlanCtx) -> Plan {
    baseline_catalog()
        .plan(&selection, &ctx)
        .expect("plan should succeed")
}

fn full_stack() -> Selection {
    Selection::modules([
        "envoy",
        "rustfs",
        "postgres",
        "unity-catalog",
        "mlflow",
        "jaeger",
        "headwaters",
        "omnigent",
    ])
}

#[test]
fn host_addresses_follow_the_gateway_port_and_routes() {
    let p = plan(
        full_stack(),
        PlanCtx {
            gateway_host_port: 9090,
            ..PlanCtx::default()
        },
    );
    let env = p.client_env(Vantage::Host);
    let get = |k: &str| env.get(k).unwrap_or_else(|| panic!("missing {k}"));
    assert_eq!(get("MLFLOW_TRACKING_URI"), "http://localhost:9090");
    // Artifacts go through MLflow's proxy, never presigned in-network URLs.
    assert_eq!(get("MLFLOW_ENABLE_PROXY_MULTIPART_DOWNLOAD"), "false");
    assert_eq!(get("MLFLOW_ENABLE_PROXY_MULTIPART_UPLOAD"), "false");
    assert_eq!(
        get("UNITY_CATALOG_URL"),
        "http://localhost:9090/api/2.1/unity-catalog"
    );
    assert_eq!(get("OPENLINEAGE_URL"), "http://localhost:9090");
    assert_eq!(get("OPENLINEAGE_ENDPOINT"), "/api/v1/lineage");
    assert_eq!(
        get("MLFLOW_AI_GATEWAY_URL"),
        "http://localhost:9090/ai-gateway"
    );
    assert_eq!(get("OMNIGENT_URL"), "http://localhost:9090/omnigent");
    assert_eq!(
        get("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT"),
        "http://localhost:9090/v1/traces"
    );
    // The object store is reached through its dedicated gateway listener.
    assert!(
        get("AWS_ENDPOINT_URL_S3").starts_with("http://localhost:"),
        "{}",
        get("AWS_ENDPOINT_URL_S3")
    );
}

#[test]
fn auth_moves_gateway_addresses_to_the_secure_origin() {
    let mut selection = full_stack();
    selection.knob_overrides.insert(
        "envoy".into(),
        [("auth".to_string(), "true".to_string())].into(),
    );
    let p = plan(
        selection,
        PlanCtx {
            env_name: "demo".into(),
            ..PlanCtx::default()
        },
    );
    let host = p.client_env(Vantage::Host);
    let get = |k: &str| host.get(k).unwrap_or_else(|| panic!("missing {k}"));
    assert_eq!(get("MLFLOW_TRACKING_URI"), "https://demo.localhost:9080");
    assert_eq!(
        get("MLFLOW_AI_GATEWAY_URL"),
        "https://demo.localhost:9080/ai-gateway"
    );
    // Object stores keep their own, ungated listeners.
    assert!(get("AWS_ENDPOINT_URL_S3").starts_with("http://localhost:"));

    let container = p.client_env(Vantage::Container);
    let get = |k: &str| container.get(k).unwrap_or_else(|| panic!("missing {k}"));
    assert_eq!(get("MLFLOW_TRACKING_URI"), "http://demo.internal:10000");
    assert_eq!(
        get("MLFLOW_AI_GATEWAY_URL"),
        "http://demo.internal:10000/ai-gateway"
    );
    assert!(get("AWS_ENDPOINT_URL_S3").starts_with("http://envoy:"));
}

#[test]
fn container_addresses_stay_in_network() {
    let p = plan(full_stack(), PlanCtx::default());
    let env = p.client_env(Vantage::Container);
    let get = |k: &str| env.get(k).unwrap_or_else(|| panic!("missing {k}"));
    assert_eq!(get("MLFLOW_TRACKING_URI"), "http://envoy:10000");
    assert_eq!(get("OPENLINEAGE_URL"), "http://envoy:10000");
    assert_eq!(
        get("MLFLOW_AI_GATEWAY_URL"),
        "http://envoy:10000/ai-gateway"
    );
    assert_eq!(get("OMNIGENT_URL"), "http://envoy:10000/omnigent");
    assert_eq!(
        get("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT"),
        "http://envoy:10000/v1/traces"
    );
    assert!(get("AWS_ENDPOINT_URL_S3").starts_with("http://envoy:"));
}

#[test]
fn lineage_ingest_path_is_stable_across_the_ui_knob() {
    // With the UI on, the gateway rewrites `/api/v1/lineage` under the `/lineage` base path;
    // clients still post to the static prefix either way.
    for serve_ui in ["true", "false"] {
        let mut selection = full_stack();
        selection
            .knob_overrides
            .entry("headwaters".into())
            .or_default()
            .insert("serve_ui".into(), serve_ui.into());
        let p = plan(selection, PlanCtx::default());
        let env = p.client_env(Vantage::Host);
        assert_eq!(env.get("OPENLINEAGE_URL"), Some("http://localhost:9080"));
        assert_eq!(env.get("OPENLINEAGE_ENDPOINT"), Some("/api/v1/lineage"));
    }
}

#[test]
fn client_env_carries_no_credentials() {
    let prefer_azurite = PlanCtx {
        provider_preference: [("object_store".to_string(), vec!["azurite".into()])].into(),
        ..PlanCtx::default()
    };
    for (selection, ctx) in [
        (full_stack(), PlanCtx::default()),
        (
            Selection::modules(["envoy", "azurite", "postgres", "mlflow"]),
            prefer_azurite,
        ),
    ] {
        let p = plan(selection, ctx);
        let mut secrets: Vec<String> = Vec::new();
        for c in p.connections.values() {
            match c {
                Connection::ObjectStore { credential, .. } => match credential {
                    ObjectStoreCredential::S3 {
                        access_key_id,
                        secret_access_key,
                        ..
                    } => secrets.extend([access_key_id.clone(), secret_access_key.clone()]),
                    ObjectStoreCredential::AzureBlob { connection_string } => {
                        secrets.push(connection_string.clone())
                    }
                    _ => {}
                },
                Connection::RelationalDb { password, url, .. } => {
                    secrets.extend([password.clone(), url.clone()])
                }
                _ => {}
            }
        }
        assert!(
            !secrets.is_empty(),
            "the fixture should resolve credentials"
        );
        for vantage in [Vantage::Host, Vantage::Container] {
            for (k, v) in p.client_env(vantage).iter() {
                for s in &secrets {
                    assert!(!v.contains(s.as_str()), "`{k}` leaks a credential");
                }
                assert!(!k.contains("SECRET") && !k.contains("PASSWORD") && !k.contains("TOKEN"));
            }
        }

        let out = p.materialize().unwrap();
        for path in [CLIENT_HOST_ENV_PATH, CLIENT_CONTAINER_ENV_PATH] {
            let f = out
                .files
                .iter()
                .find(|f| f.path == path)
                .unwrap_or_else(|| panic!("{path} not materialized"));
            assert!(!f.sensitive, "{path} is shareable, not sensitive");
        }
    }
}
