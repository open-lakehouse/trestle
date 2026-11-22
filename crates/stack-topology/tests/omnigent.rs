//! The `omnigent` agent-platform module: what it pulls into a stack, how it is routed, how its
//! in-stack agent host follows its knob and the gateway's authentication, and where the host's
//! harnesses call models (MLflow's AI Gateway, or an upstream directly without MLflow).

use std::collections::BTreeMap;
use std::process::Command;

use olai_stack_topology::{
    GatewaySettings, MaterializedOutput, ModuleId, Plan, PlanCtx, PlanError, RuntimePlacement,
    Selection, baseline_catalog,
};

fn selection(modules: &[&str], knobs: &[(&str, &str, &str)]) -> Selection {
    let mut knob_overrides: BTreeMap<ModuleId, BTreeMap<String, String>> = BTreeMap::new();
    for (module, key, value) in knobs {
        knob_overrides
            .entry(ModuleId::from(*module))
            .or_default()
            .insert((*key).to_string(), (*value).to_string());
    }
    Selection {
        modules: modules.iter().map(|m| (*m).into()).collect(),
        knob_overrides,
        ..Default::default()
    }
}

/// Omnigent with MLflow (and Unity Catalog), as the `agents` scenario plans it.
fn plan_with(knobs: &[(&str, &str, &str)]) -> Plan {
    baseline_catalog()
        .plan(
            &selection(&["envoy", "unity-catalog-oss", "mlflow", "omnigent"], knobs),
            &PlanCtx::default(),
        )
        .expect("plan should succeed")
}

/// Omnigent without MLflow, its host on `upstream`.
fn plan_direct(upstream: &str) -> Plan {
    baseline_catalog()
        .plan(
            &selection(
                &["envoy", "unity-catalog-oss", "omnigent"],
                &[("omnigent", "upstream", upstream)],
            ),
            &PlanCtx::default(),
        )
        .expect("plan should succeed")
}

fn services(p: &Plan) -> Vec<String> {
    p.services[&ModuleId::from("omnigent")]
        .iter()
        .map(|s| s.name.clone())
        .collect()
}

fn fragment(p: &Plan, id: &str) -> String {
    olai_stack_topology::render_all(p)
        .unwrap()
        .modules
        .into_iter()
        .find(|(module, _)| module.as_str() == id)
        .map(|(_, out)| out.fragment.clone())
        .unwrap_or_else(|| panic!("{id} renders"))
}

#[test]
fn selecting_omnigent_brings_its_models_and_store() {
    let p = plan_with(&[]);
    // With MLflow (for its AI Gateway) in the stack: it and Omnigent demand a relational
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
    // The desktop's `<label>.localhost` origins pass local mode's Origin check.
    assert!(
        f.contains(r#"OMNIGENT_WS_ALLOWED_ORIGINS: "http://*.localhost:9080""#),
        "{f}"
    );
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
    // Non-local mode admits any Origin; an allowlist would make it deny-by-default.
    assert!(!f.contains("OMNIGENT_WS_ALLOWED_ORIGINS"), "{f}");
}

#[test]
fn the_trusted_origins_follow_the_slots_gateway_port() {
    let mut ctx = PlanCtx::default();
    RuntimePlacement {
        slot: 1,
        ..RuntimePlacement::standalone("dev")
    }
    .apply(&mut ctx);
    let p = baseline_catalog()
        .plan(
            &selection(&["envoy", "unity-catalog-oss", "mlflow", "omnigent"], &[]),
            &ctx,
        )
        .expect("plan should succeed");
    assert_eq!(p.gateway_host_port(), Some(10080));
    let f = fragment(&p, "omnigent");
    assert!(
        f.contains(r#"OMNIGENT_WS_ALLOWED_ORIGINS: "http://*.localhost:10080""#),
        "{f}"
    );
}

#[test]
fn without_a_gateway_no_origins_are_trusted() {
    let mut selection = selection(&["unity-catalog-oss", "mlflow", "omnigent"], &[]);
    selection.gateway = GatewaySettings {
        enabled: false,
        ..GatewaySettings::default()
    };
    let p = baseline_catalog()
        .plan(&selection, &PlanCtx::default())
        .expect("plan should succeed");
    let f = fragment(&p, "omnigent");
    assert!(f.contains(r#"OMNIGENT_AUTH_ENABLED: "0""#), "{f}");
    assert!(!f.contains("OMNIGENT_WS_ALLOWED_ORIGINS"), "{f}");
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

#[test]
fn the_gateway_seeder_stays_up_so_compose_wait_returns() {
    // Nothing depends on the seeder, so if it exited, `docker compose up --wait` would fail on
    // it whenever it finished first; it stays up and reports healthy instead.
    let f: serde_yaml::Value = serde_yaml::from_str(&fragment(&plan_with(&[]), "mlflow")).unwrap();
    let init = &f["services"]["mlflow-gateway-init"];
    assert_eq!(init["init"].as_bool(), Some(true), "{init:?}");
    assert_eq!(
        init["healthcheck"]["test"],
        serde_yaml::from_str::<serde_yaml::Value>(r#"["CMD", "test", "-f", "/tmp/seeded"]"#)
            .unwrap(),
        "{init:?}"
    );
}

fn file<'a>(out: &'a MaterializedOutput, path: &str) -> &'a str {
    &out.files
        .iter()
        .find(|f| f.path == path)
        .unwrap_or_else(|| panic!("{path} is rendered"))
        .contents
}

/// The host service of the omnigent fragment.
fn host(p: &Plan) -> serde_yaml::Value {
    let f: serde_yaml::Value = serde_yaml::from_str(&fragment(p, "omnigent")).expect("valid YAML");
    f["services"]["omnigent-host"].clone()
}

fn host_env(p: &Plan) -> BTreeMap<String, Option<String>> {
    host(p)["environment"]
        .as_mapping()
        .unwrap()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().unwrap().to_owned(),
                v.as_str().map(str::to_owned),
            )
        })
        .collect()
}

fn host_command(p: &Plan) -> String {
    host(p)["command"][2].as_str().unwrap().to_owned()
}

fn secret<'a>(p: &'a Plan, name: &str) -> Option<&'a olai_stack_topology::DeclaredSecret> {
    p.secrets
        .get(&ModuleId::from("omnigent"))?
        .iter()
        .find(|s| s.name == name)
}

#[test]
fn omnigent_no_longer_pulls_in_mlflow_and_auto_needs_it() {
    let err = baseline_catalog()
        .plan(
            &selection(&["envoy", "unity-catalog-oss", "omnigent"], &[]),
            &PlanCtx::default(),
        )
        .unwrap_err();
    let PlanError::RejectedKnob {
        module,
        key,
        reason,
    } = &err
    else {
        panic!("{err:?}");
    };
    assert_eq!((module.as_str(), key.as_str()), ("omnigent", "upstream"));
    assert!(
        reason.contains("`mlflow`") && reason.contains("`neon`"),
        "{reason}"
    );
    // `mlflow` named outright is the same.
    let err = baseline_catalog()
        .plan(
            &selection(
                &["envoy", "omnigent"],
                &[("omnigent", "upstream", "mlflow")],
            ),
            &PlanCtx::default(),
        )
        .unwrap_err();
    assert!(matches!(err, PlanError::RejectedKnob { .. }), "{err:?}");

    for upstream in ["neon", "databricks", "keys"] {
        let p = plan_direct(upstream);
        assert!(
            !p.services.contains_key(&ModuleId::from("mlflow")),
            "{upstream} pulls in no mlflow"
        );
        assert_eq!(services(&p), ["omnigent", "omnigent-host"]);
    }
    // With MLflow present, `auto` keeps using it and declares no secret of its own.
    let p = plan_with(&[]);
    assert!(host_command(&p).contains("$$MLFLOW_AI_GATEWAY_URL/anthropic"));
    assert!(!p.secrets.contains_key(&ModuleId::from("omnigent")));
}

#[test]
fn a_databricks_sandbox_host_needs_its_workspace_on_the_server() {
    let server = |p: &Plan| -> serde_yaml::Value {
        let doc: serde_yaml::Value = serde_yaml::from_str(&fragment(p, "omnigent")).unwrap();
        doc["services"]["omnigent"].clone()
    };
    // By default sessions run on the attached hosts: no workspace.
    let p = plan_with(&[]);
    assert!(secret(&p, "DATABRICKS_HOST").is_none());
    assert!(server(&p)["environment"].get("DATABRICKS_HOST").is_none());

    let p = plan_with(&[("omnigent", "sandbox_host", "databricks")]);
    assert!(secret(&p, "DATABRICKS_HOST").unwrap().required);
    assert!(server(&p)["environment"].get("DATABRICKS_HOST").is_some());

    // With the Databricks upstream too, the host and the server share the one secret.
    let p = baseline_catalog()
        .plan(
            &selection(
                &["envoy", "omnigent"],
                &[
                    ("omnigent", "upstream", "databricks"),
                    ("omnigent", "sandbox_host", "databricks"),
                ],
            ),
            &PlanCtx::default(),
        )
        .unwrap();
    let names: Vec<&str> = p.secrets[&ModuleId::from("omnigent")]
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(names.iter().filter(|n| **n == "DATABRICKS_HOST").count(), 1);
    assert!(names.contains(&"DATABRICKS_METADATA_SERVICE_URL"));
}

#[test]
fn the_neon_upstream_points_both_families_at_the_branch_gateway() {
    let p = plan_direct("neon");
    let env = host_env(&p);
    assert_eq!(env["NEON_AI_GATEWAY_BASE_URL"], None, "a bare pass-through");
    assert_eq!(env["NEON_AI_GATEWAY_TOKEN"], None);
    assert!(!env.contains_key("ANTHROPIC_API_KEY"));
    assert_eq!(env["ANTHROPIC_MODEL"].as_deref(), Some("claude-opus-5-5"));
    let command = host_command(&p);
    for export in [
        r#"ANTHROPIC_BASE_URL="$$NEON_AI_GATEWAY_BASE_URL/anthropic""#,
        r#"ANTHROPIC_API_KEY="$$NEON_AI_GATEWAY_TOKEN""#,
        r#"OPENAI_BASE_URL="$$NEON_AI_GATEWAY_BASE_URL/openai/v1""#,
        r#"OPENAI_API_KEY="$$NEON_AI_GATEWAY_TOKEN""#,
    ] {
        assert!(command.contains(export), "{export} in {command}");
    }
    for name in ["NEON_AI_GATEWAY_BASE_URL", "NEON_AI_GATEWAY_TOKEN"] {
        assert!(secret(&p, name).unwrap().required, "{name}");
    }
    // So the instance needs a branch, with its gateway.
    assert!(p.neon_requirements().ai_gateway);
}

#[test]
fn the_databricks_upstream_installs_a_provider_config_without_a_credential() {
    let p = plan_direct("databricks");
    let env = host_env(&p);
    for name in [
        "DATABRICKS_HOST",
        "DATABRICKS_METADATA_SERVICE_URL",
        "DATABRICKS_TOKEN",
    ] {
        assert_eq!(env[name], None, "{name} is a bare pass-through");
    }
    // The workspace's serving endpoints, named after the models.
    assert_eq!(
        env["ANTHROPIC_MODEL"].as_deref(),
        Some("databricks-claude-opus-5-5")
    );
    assert_eq!(
        env["ANTHROPIC_DEFAULT_HAIKU_MODEL"].as_deref(),
        Some("databricks-claude-haiku-5-5")
    );
    let targets: Vec<String> = host(&p)["configs"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|c| c["target"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        targets,
        [
            "/etc/trestle/omnigent/config.yaml",
            "/etc/trestle/omnigent/databricks_token.py"
        ]
    );
    // Installed with the workspace written in: the runners reading it lack the host's env.
    assert!(
        host_command(&p).contains(concat!(
            r#"sed "s#\$$DATABRICKS_HOST#$${DATABRICKS_HOST%/}#g" "#,
            "/etc/trestle/omnigent/config.yaml > /root/.omnigent/config.yaml"
        )),
        "{}",
        host_command(&p)
    );

    let out = p.materialize().unwrap();
    let config: serde_yaml::Value =
        serde_yaml::from_str(file(&out, "modules/omnigent/databricks/config.yaml")).unwrap();
    let provider = &config["providers"]["trestle-databricks"];
    assert_eq!(provider["kind"], "gateway");
    let anthropic = &provider["anthropic"];
    assert_eq!(
        anthropic["base_url"],
        "$DATABRICKS_HOST/serving-endpoints/anthropic"
    );
    assert_eq!(
        anthropic["auth_command"],
        "python3 /etc/trestle/omnigent/databricks_token.py"
    );
    assert_eq!(anthropic["models"]["default"], "databricks-claude-opus-5-5");
    assert!(
        file(&out, "modules/omnigent/databricks/config.yaml").starts_with("# Written by trestle")
    );
    assert!(secret(&p, "DATABRICKS_HOST").unwrap().required);
    assert!(!secret(&p, "DATABRICKS_TOKEN").unwrap().required);

    // A model already named for the workspace keeps its name.
    let p = baseline_catalog()
        .plan(
            &selection(
                &["envoy", "omnigent"],
                &[
                    ("omnigent", "upstream", "databricks"),
                    ("omnigent", "model", "databricks-gpt-5-5"),
                ],
            ),
            &PlanCtx::default(),
        )
        .unwrap();
    assert_eq!(
        host_env(&p)["ANTHROPIC_MODEL"].as_deref(),
        Some("databricks-gpt-5-5")
    );
}

#[test]
fn the_keys_upstream_passes_the_provider_keys_through() {
    let p = plan_direct("keys");
    let env = host_env(&p);
    assert_eq!(env["ANTHROPIC_API_KEY"], None);
    assert_eq!(env["OPENAI_API_KEY"], None);
    assert!(!host_command(&p).contains("ANTHROPIC_BASE_URL"));
    assert!(!secret(&p, "ANTHROPIC_API_KEY").unwrap().required);
    assert!(!p.neon_requirements().ai_gateway);
}

#[test]
fn only_the_databricks_upstream_keeps_a_trestle_provider_config() {
    // Any other upstream removes one an earlier databricks start installed (marked as
    // trestle's), so it can't keep routing Claude to the workspace; anything else is left.
    for upstream in ["neon", "keys"] {
        let command = host_command(&plan_direct(upstream));
        assert!(
            command.contains("grep -q '^# Written by trestle' && rm /root/.omnigent/config.yaml"),
            "{command}"
        );
    }
    assert!(host_command(&plan_with(&[])).contains("rm /root/.omnigent/config.yaml"));
}

#[test]
fn no_upstream_writes_a_credential_into_the_tree() {
    let sentinels = ["nt_live_sentinel", "dapi-sentinel", "sk-ant-sentinel"];
    for upstream in ["neon", "databricks", "keys"] {
        let (out, _) = plan_direct(upstream)
            .materialize()
            .unwrap()
            .into_launch_env()
            .unwrap();
        for f in &out.files {
            for s in sentinels {
                assert!(!f.contents.contains(s), "{s} in {}", f.path);
            }
        }
    }
}

#[test]
fn only_the_mlflow_upstream_draws_an_edge_to_mlflow() {
    use olai_stack_topology::projection::{TopologyEdgeDto, graph_dto};
    let uses_mlflow = |p: &Plan| {
        graph_dto(p).edges.iter().any(|e| {
            matches!(e, TopologyEdgeDto::Uses { from, to, .. } if from == "omnigent" && to == "mlflow")
        })
    };
    assert!(uses_mlflow(&plan_with(&[])));
    // With MLflow in the stack but the neon upstream, the host goes direct.
    assert!(!uses_mlflow(&plan_with(&[(
        "omnigent", "upstream", "neon"
    )])));
}

/// Run the rendered token helper with `env`, against a stub metadata endpoint serving
/// `token` (Python's own HTTP server), or `None` without `python3`.
fn token_helper(env: &[(&str, &str)], with_endpoint: bool) -> Option<(bool, String, String)> {
    let out = plan_direct("databricks").materialize().unwrap();
    let dir = std::env::temp_dir().join(format!(
        "trestle-dbx-token-{}-{with_endpoint}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let helper = dir.join("databricks_token.py");
    std::fs::write(
        &helper,
        file(&out, "modules/omnigent/databricks/databricks_token.py"),
    )
    .unwrap();
    // A stub metadata service in the same process, then the helper as `__main__`.
    let program = r#"
import http.server, json, os, runpy, sys, threading
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        ok = self.headers.get("X-Databricks-Metadata-Version") == "1" and self.headers.get("X-Databricks-Host") == "https://example.cloud.databricks.com"
        body = json.dumps({"access_token": "from-the-endpoint", "token_type": "Bearer", "expires_on": 4102444800}).encode()
        self.send_response(200 if ok else 403); self.end_headers(); self.wfile.write(body if ok else b"{}")
    def log_message(self, *a): pass
if sys.argv[2] == "endpoint":
    srv = http.server.HTTPServer(("127.0.0.1", 0), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    os.environ["DATABRICKS_METADATA_SERVICE_URL"] = f"http://127.0.0.1:{srv.server_port}/seg"
sys.argv = [sys.argv[1]]
runpy.run_path(sys.argv[0], run_name="__main__")
"#;
    let mut cmd = Command::new("python3");
    cmd.arg("-I")
        .arg("-c")
        .arg(program)
        .arg(&helper)
        .arg(if with_endpoint { "endpoint" } else { "none" })
        .env_remove("DATABRICKS_METADATA_SERVICE_URL")
        .env_remove("DATABRICKS_TOKEN")
        .env_remove("DATABRICKS_HOST");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let run = cmd.output();
    std::fs::remove_dir_all(&dir).ok();
    let run = run.ok()?;
    Some((
        run.status.success(),
        String::from_utf8_lossy(&run.stdout).trim().to_owned(),
        String::from_utf8_lossy(&run.stderr).into_owned(),
    ))
}

#[test]
fn the_token_helper_fetches_from_the_endpoint_or_falls_back_to_a_static_token() {
    let host = ("DATABRICKS_HOST", "https://example.cloud.databricks.com/");
    let Some((ok, out, err)) = token_helper(&[host], true) else {
        eprintln!("skipped: no python3");
        return;
    };
    assert!(ok, "{err}");
    assert_eq!(out, "from-the-endpoint");
    // The endpoint checks the host header: another workspace gets nothing.
    let (ok, _, _) = token_helper(&[("DATABRICKS_HOST", "https://elsewhere")], true).unwrap();
    assert!(!ok);
    let (ok, out, _) = token_helper(&[host, ("DATABRICKS_TOKEN", "dapi-static")], false).unwrap();
    assert!(ok);
    assert_eq!(out, "dapi-static");
    let (ok, out, err) = token_helper(&[host], false).unwrap();
    assert!(!ok && out.is_empty());
    assert!(err.contains("DATABRICKS_METADATA_SERVICE_URL"), "{err}");
}

#[test]
fn agents_use_the_stacks_uc_mcp_server() {
    use olai_stack_topology::projection::{TopologyEdgeDto, graph_dto};
    let uses_mcp = |modules: &[&str]| {
        let p = baseline_catalog()
            .plan(
                &selection(modules, &[("omnigent", "upstream", "neon")]),
                &PlanCtx::default(),
            )
            .expect("plan should succeed");
        graph_dto(&p).edges.into_iter().find_map(|e| match e {
            TopologyEdgeDto::Uses { from, to, role, .. }
                if from == "omnigent" && to == "uc-mcp" =>
            {
                Some(role)
            }
            _ => None,
        })
    };
    assert_eq!(
        uses_mcp(&["envoy", "uc-mcp", "omnigent"]).as_deref(),
        Some("mcp_server")
    );
    assert_eq!(uses_mcp(&["envoy", "unity-catalog-oss", "omnigent"]), None);
}
