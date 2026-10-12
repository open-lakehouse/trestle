//! The `omnigent` agent-platform module: what it pulls into a stack, how it is routed, and
//! how its in-stack agent host follows its knob and the gateway's authentication.

use std::collections::BTreeMap;

use olai_stack_topology::{ModuleId, Plan, PlanCtx, Selection, baseline_catalog};

fn plan_with(knobs: &[(&str, &str, &str)]) -> Plan {
    let mut knob_overrides: BTreeMap<ModuleId, BTreeMap<String, String>> = BTreeMap::new();
    for (module, key, value) in knobs {
        knob_overrides
            .entry(ModuleId::from(*module))
            .or_default()
            .insert((*key).to_string(), (*value).to_string());
    }
    let selection = Selection {
        modules: vec!["envoy".into(), "unity-catalog".into(), "omnigent".into()],
        knob_overrides,
        ..Default::default()
    };
    baseline_catalog()
        .plan(&selection, &PlanCtx::default())
        .expect("plan should succeed")
}

fn services(p: &Plan) -> Vec<String> {
    p.services[&ModuleId::from("omnigent")]
        .iter()
        .map(|s| s.name.clone())
        .collect()
}

fn fragment(p: &Plan, id: &str) -> String {
    p.renders
        .iter()
        .find(|(module, _)| module.as_str() == id)
        .map(|(_, out)| out.fragment.clone())
        .unwrap_or_else(|| panic!("{id} renders"))
}

#[test]
fn selecting_omnigent_brings_its_models_and_store() {
    let p = plan_with(&[]);
    // MLflow (for its AI Gateway) is a hard requirement; it and Omnigent demand a relational
    // store, MLflow an object store, so the default providers are planned in too.
    for id in ["mlflow", "postgres", "rustfs"] {
        assert!(
            p.services.contains_key(&ModuleId::from(id)),
            "{id} not planned"
        );
    }

    // The whole Omnigent surface is fronted at its base path, streaming-safe; the AI Gateway
    // it gets its models from is fronted at `/ai-gateway`.
    let shared = &p.gateway.listeners[0].routes;
    let route = |prefix: &str| {
        shared
            .iter()
            .find(|r| r.prefix == prefix)
            .unwrap_or_else(|| panic!("no route for {prefix}"))
    };
    let omnigent = route("/omnigent");
    assert_eq!(omnigent.cluster, "omnigent");
    assert_eq!(omnigent.rewrite, None);
    assert!(omnigent.options.streaming);
    assert_eq!(
        route("/ai-gateway").rewrite.as_deref(),
        Some("/mlflow/gateway")
    );
}

#[test]
fn the_host_runs_agents_with_the_stacks_client_env_and_gateway_models() {
    let p = plan_with(&[]);
    assert_eq!(services(&p), ["omnigent", "omnigent-host"]);

    let f = fragment(&p, "omnigent");
    // Single-user while the gateway is open.
    assert!(f.contains(r#"OMNIGENT_AUTH_ENABLED: "0""#), "{f}");
    assert!(f.contains(r#"OMNIGENT_WEB_BASE_PATH: "/omnigent""#), "{f}");
    // The host loads the stack's in-network client env, forwards it to agents, and points the
    // harnesses at the AI Gateway rather than carrying a provider key.
    assert!(f.contains("path: ./client/container.env"), "{f}");
    let passthrough = f
        .lines()
        .find(|l| l.contains("OMNIGENT_RUNNER_ENV_PASSTHROUGH:"))
        .expect("passthrough line");
    for key in [
        "UNITY_CATALOG_URL",
        "MLFLOW_AI_GATEWAY_URL",
        "AWS_ENDPOINT_URL_S3",
    ] {
        assert!(passthrough.contains(key), "{passthrough}");
    }
    assert!(f.contains(r#"ANTHROPIC_BASE_URL="$$MLFLOW_AI_GATEWAY_URL/anthropic""#));
    assert!(f.contains(r#"ANTHROPIC_MODEL: "claude-opus-5-5""#), "{f}");
    assert!(!f.contains("sk-"), "no provider key in the fragment");

    // The fragment is valid YAML.
    let _: serde_yaml::Value = serde_yaml::from_str(&f).expect("valid YAML");
}

#[test]
fn the_host_knob_drops_the_host() {
    let p = plan_with(&[("omnigent", "host", "false")]);
    assert_eq!(services(&p), ["omnigent"]);
    assert!(!fragment(&p, "omnigent").contains("omnigent-host:"));
}

#[test]
fn under_gateway_auth_the_server_trusts_the_asserted_identity_and_runs_no_host() {
    let p = plan_with(&[("envoy", "auth", "true")]);
    // A host can't present the gateway-asserted identity, so it is dropped from the topology
    // and the rendered compose alike.
    assert_eq!(services(&p), ["omnigent"]);
    let f = fragment(&p, "omnigent");
    assert!(!f.contains("omnigent-host:"), "{f}");
    assert!(f.contains("OMNIGENT_AUTH_PROVIDER: header"), "{f}");
    assert!(f.contains("OMNIGENT_AUTH_HEADER: Remote-Email"), "{f}");
    assert!(!f.contains("OMNIGENT_AUTH_ENABLED"), "{f}");
}

#[test]
fn the_ai_gateway_is_seeded_with_the_configured_models() {
    let p = plan_with(&[(
        "mlflow",
        "ai_gateway_models",
        "anthropic:claude-sonnet-5-5,openai:gpt-test",
    )]);
    let f = fragment(&p, "mlflow");
    assert!(
        f.contains(r#"MLFLOW_GATEWAY_MODELS: "anthropic:claude-sonnet-5-5,openai:gpt-test""#),
        "{f}"
    );
}
