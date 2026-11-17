//! The `mlflow` module's AI Gateway upstreams: the `upstream` knob and the model presets it
//! picks, the secrets each upstream declares (names only, never rendered), the required key
//! passphrase, and how the gateway seeder routes `provider:model` pairs.

use std::collections::BTreeMap;
use std::process::Command;

use olai_stack_topology::{
    DeclaredSecret, MaterializedOutput, ModuleId, Plan, PlanCtx, PlanError, Selection,
    baseline_catalog,
};

const KEYS_MODELS: &str =
    "anthropic:claude-opus-5-5,anthropic:claude-sonnet-5-5,anthropic:claude-haiku-5-5";
const DATABRICKS_MODELS: &str = "databricks:databricks-claude-opus-5-5,\
     databricks:databricks-claude-sonnet-5-5,databricks:databricks-claude-haiku-5-5";

fn selection(knobs: &[(&str, &str)]) -> Selection {
    let overrides: BTreeMap<String, String> = knobs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
    let mut selection = Selection::modules(["envoy", "postgres", "rustfs", "mlflow"]);
    selection
        .knob_overrides
        .insert(ModuleId::from("mlflow"), overrides);
    selection
}

fn plan_with(knobs: &[(&str, &str)]) -> Plan {
    baseline_catalog()
        .plan(&selection(knobs), &PlanCtx::default())
        .expect("plan should succeed")
}

fn file<'a>(out: &'a MaterializedOutput, path: &str) -> &'a str {
    &out.files
        .iter()
        .find(|f| f.path == path)
        .unwrap_or_else(|| panic!("{path} is rendered"))
        .contents
}

fn fragment(p: &Plan) -> serde_yaml::Value {
    let out = p.materialize().unwrap();
    serde_yaml::from_str(file(&out, "modules/mlflow/compose.yaml")).expect("valid YAML")
}

/// The `environment` keys of one service in the mlflow fragment, with their values (`None`
/// for a bare pass-through).
fn environment(p: &Plan, service: &str) -> BTreeMap<String, Option<String>> {
    let fragment = fragment(p);
    let env = &fragment["services"][service]["environment"];
    env.as_mapping()
        .unwrap_or_else(|| panic!("{service} has an environment"))
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().unwrap().to_string(),
                v.as_str().map(str::to_string),
            )
        })
        .collect()
}

fn secret<'a>(p: &'a Plan, name: &str) -> Option<&'a DeclaredSecret> {
    p.secrets[&ModuleId::from("mlflow")]
        .iter()
        .find(|s| s.name == name)
}

#[test]
fn the_keys_upstream_is_the_default_with_the_provider_models() {
    let p = plan_with(&[]);
    let init = environment(&p, "mlflow-gateway-init");
    assert_eq!(init["MLFLOW_UPSTREAM"].as_deref(), Some("keys"));
    assert_eq!(init["MLFLOW_GATEWAY_MODELS"].as_deref(), Some(KEYS_MODELS));
    for key in ["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "GEMINI_API_KEY"] {
        assert_eq!(init[key], None, "{key} is a bare pass-through");
        assert!(!secret(&p, key).unwrap().required, "{key} is optional");
    }
    // The server sees no Databricks environment it doesn't need.
    let server = environment(&p, "mlflow");
    assert!(!server.contains_key("DATABRICKS_HOST"));
    assert!(!server.contains_key("DATABRICKS_METADATA_SERVICE_URL"));
    assert!(!init.contains_key("NEON_AI_GATEWAY_TOKEN"));
    assert!(!secret(&p, "NEON_AI_GATEWAY_TOKEN").unwrap().required);
}

#[test]
fn the_databricks_upstream_serves_the_workspaces_claude_endpoints() {
    let p = plan_with(&[("upstream", "databricks")]);
    let init = environment(&p, "mlflow-gateway-init");
    assert_eq!(init["MLFLOW_UPSTREAM"].as_deref(), Some("databricks"));
    assert_eq!(
        init["MLFLOW_GATEWAY_MODELS"].as_deref(),
        Some(DATABRICKS_MODELS)
    );
    // The server's Databricks SDK reads the workspace and the token endpoint from its env.
    let server = environment(&p, "mlflow");
    assert_eq!(server["DATABRICKS_HOST"], None);
    assert_eq!(server["DATABRICKS_METADATA_SERVICE_URL"], None);
    assert!(secret(&p, "DATABRICKS_HOST").unwrap().required);
    assert!(
        !secret(&p, "DATABRICKS_METADATA_SERVICE_URL")
            .unwrap()
            .required
    );
    assert!(!secret(&p, "DATABRICKS_TOKEN").unwrap().required);
    assert_eq!(init["DATABRICKS_TOKEN"], None);
}

#[test]
fn the_neon_upstream_reads_the_branch_gateway() {
    let p = plan_with(&[("upstream", "neon")]);
    let init = environment(&p, "mlflow-gateway-init");
    assert_eq!(init["MLFLOW_UPSTREAM"].as_deref(), Some("neon"));
    assert_eq!(init["NEON_AI_GATEWAY_BASE_URL"], None);
    assert_eq!(init["NEON_AI_GATEWAY_TOKEN"], None);
    for name in ["NEON_AI_GATEWAY_BASE_URL", "NEON_AI_GATEWAY_TOKEN"] {
        assert!(secret(&p, name).unwrap().required, "{name} is required");
    }
    assert!(!secret(&p, "DATABRICKS_HOST").unwrap().required);
    // So the instance must run on a branch, even with nothing else hosted there.
    let req = p.neon_requirements();
    assert!(req.ai_gateway && !req.is_empty());
    assert!(!plan_with(&[]).neon_requirements().ai_gateway);
    assert!(
        init["MLFLOW_GATEWAY_MODELS"]
            .as_deref()
            .unwrap()
            .contains("anthropic:claude-opus-5-5")
    );
}

#[test]
fn explicit_models_win_over_the_upstreams_preset() {
    let p = plan_with(&[
        ("upstream", "databricks"),
        ("ai_gateway_models", "databricks:databricks-gpt-5-5"),
    ]);
    let init = environment(&p, "mlflow-gateway-init");
    assert_eq!(
        init["MLFLOW_GATEWAY_MODELS"].as_deref(),
        Some("databricks:databricks-gpt-5-5")
    );
}

#[test]
fn an_unknown_upstream_is_rejected_at_plan_time() {
    let err = baseline_catalog()
        .plan(
            &selection(&[("upstream", "openrouter")]),
            &PlanCtx::default(),
        )
        .unwrap_err();
    assert!(
        matches!(err, PlanError::InvalidKnobValue { ref key, .. } if key == "upstream"),
        "{err}"
    );
}

#[test]
fn the_key_passphrase_is_a_required_minted_secret_never_rendered() {
    let p = plan_with(&[]);
    let passphrase = secret(&p, "MLFLOW_CRYPTO_KEK_PASSPHRASE").unwrap();
    assert!(passphrase.required && passphrase.minted);
    // Passed through by name; the server's entrypoint refuses to start without it (MLflow
    // would seal under its public default), while `down` still works without it.
    let server = environment(&p, "mlflow");
    assert_eq!(server["MLFLOW_CRYPTO_KEK_PASSPHRASE"], None);
    let fragment = fragment(&p);
    let entrypoint: Vec<&str> = fragment["services"]["mlflow"]["entrypoint"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(entrypoint[..2], ["sh", "-c"]);
    assert!(entrypoint[2].contains(r#"[ -z "$$MLFLOW_CRYPTO_KEK_PASSPHRASE" ]"#));
    assert!(entrypoint[2].ends_with(r#"exec "$$@""#));
    let out = p.materialize().unwrap();
    for f in &out.files {
        assert!(
            !f.contents.contains("lakehouse-dev-kek") || f.path == "README.md",
            "the old fixed passphrase is in {}",
            f.path
        );
        assert!(
            !f.contents.contains("MLFLOW_CRYPTO_KEK_PASSPHRASE=") || f.path == "README.md",
            "a passphrase value is assigned in {}",
            f.path
        );
    }
    assert!(
        out.deploy
            .iter()
            .any(|s| s.title == "Set the AI Gateway's key passphrase"),
        "the README says how to set it"
    );
}

#[test]
fn declared_secrets_carry_names_but_never_values() {
    // Whatever the launching process holds, the rendered tree has only the names.
    let sentinels = [
        "sk-ant-sentinel-0001",
        "nt_live_sentinel_0002",
        "dapi-sentinel-0003",
    ];
    for upstream in ["keys", "databricks", "neon"] {
        let p = plan_with(&[("upstream", upstream)]);
        let (out, launch) = p.materialize().unwrap().into_launch_env().unwrap();
        for f in &out.files {
            for s in sentinels {
                assert!(!f.contents.contains(s), "{s} in {}", f.path);
            }
        }
        assert!(
            launch
                .vars()
                .keys()
                .all(|k| !k.starts_with("NEON_") && !k.starts_with("DATABRICKS_")),
            "the plan supplies no upstream credential itself"
        );
        let merged = p.declared_secrets();
        assert!(merged.iter().all(|s| s.help.is_some()));
        assert!(
            merged
                .iter()
                .any(|s| s.name == "MLFLOW_CRYPTO_KEK_PASSPHRASE" && s.required)
        );
    }
}

/// Run `code` against the rendered seeder's functions (`seed` is its module dict), or `None`
/// without a `python3` on the PATH.
fn seeder(code: &str) -> Option<String> {
    let p = plan_with(&[]);
    let out = p.materialize().unwrap();
    // One directory per call: tests run in parallel and each removes its own.
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("trestle-seed-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("seed_gateway.py");
    std::fs::write(&script, file(&out, "modules/mlflow/seed_gateway.py")).unwrap();
    let program = format!("import json, runpy, sys\nseed = runpy.run_path(sys.argv[1])\n{code}\n");
    let run = Command::new("python3")
        .arg("-I")
        .arg("-c")
        .arg(program)
        .arg(&script)
        .output();
    std::fs::remove_dir_all(&dir).ok();
    let run = run.ok()?;
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    Some(String::from_utf8(run.stdout).unwrap())
}

#[test]
fn the_seeder_names_databricks_endpoints_without_the_prefix() {
    let Some(out) = seeder(
        "pairs = seed['parse_models'](' databricks:databricks-claude-opus-5-5 , anthropic:claude-haiku-5-5,')\n\
         print(json.dumps([seed['endpoint_name'](p, m) for p, m in pairs]))",
    ) else {
        eprintln!("skipped: no python3");
        return;
    };
    assert_eq!(out.trim(), r#"["claude-opus-5-5", "claude-haiku-5-5"]"#);
}

#[test]
fn the_seeder_routes_each_pair_by_upstream() {
    let Some(out) = seeder(
        r#"
host = {"DATABRICKS_HOST": "https://example.cloud.databricks.com/"}
neon = {"NEON_AI_GATEWAY_BASE_URL": "https://br-x-api.ai.example.neon.tech", "NEON_AI_GATEWAY_TOKEN": "nt_live_x"}
cases = [
    ("databricks", "databricks-claude-opus-5-5", "databricks", host),
    ("databricks", "databricks-claude-opus-5-5", "databricks", {**host, "DATABRICKS_TOKEN": "dapi-x"}),
    ("databricks", "databricks-claude-opus-5-5", "keys", host),
    ("databricks", "databricks-claude-opus-5-5", "databricks", {}),
    ("anthropic", "claude-opus-5-5", "neon", neon),
    ("openai", "gpt-5-mini", "neon", neon),
    ("gemini", "gemini-3-flash", "neon", neon),
    ("anthropic", "claude-opus-5-5", "neon", {}),
    ("anthropic", "claude-opus-5-5", "keys", {"ANTHROPIC_API_KEY": "sk-x"}),
    ("anthropic", "claude-opus-5-5", "keys", {}),
]
for provider, model, upstream, env in cases:
    route, secret, reason = seed["plan_route"](provider, model, upstream, env)
    print(json.dumps([route, secret, reason is not None]))
"#,
    ) else {
        eprintln!("skipped: no python3");
        return;
    };
    let rows: Vec<serde_json::Value> = out
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let route = |i: usize| rows[i][0].as_str();
    let secret = |i: usize| &rows[i][1];
    // Token-less: the server's SDK authenticates, the secret only names the workspace.
    assert_eq!(route(0), Some("databricks"));
    assert_eq!(secret(0)["secret_value"], serde_json::json!({}));
    assert_eq!(
        secret(0)["auth_config"]["api_base"],
        "https://example.cloud.databricks.com"
    );
    // A static token is the key, under its own secret.
    assert_eq!(route(1), Some("databricks-token"));
    assert_eq!(secret(1)["secret_name"], "databricks-token");
    assert_eq!(secret(1)["secret_value"]["api_key"], "dapi-x");
    // Without a token, only the databricks upstream gives the server the SDK's environment.
    assert_eq!(route(2), None);
    assert_eq!(route(3), None);
    // Neon: the providers' own APIs at the branch's gateway, its token as the key.
    assert_eq!(route(4), Some("neon"));
    assert_eq!(
        secret(4)["auth_config"]["api_base"],
        "https://br-x-api.ai.example.neon.tech/anthropic/v1"
    );
    assert_eq!(secret(4)["secret_value"]["api_key"], "nt_live_x");
    assert_eq!(
        secret(5)["auth_config"]["api_base"],
        "https://br-x-api.ai.example.neon.tech/openai/v1"
    );
    assert_eq!(route(6), None, "no gemini through Neon");
    assert_eq!(route(7), None);
    assert_eq!(route(8), Some("keys"));
    assert_eq!(secret(8)["secret_name"], "anthropic");
    assert_eq!(route(9), None);
    for row in rows.iter().filter(|r| r[0].is_null()) {
        assert_eq!(row[2], true, "a skip says why");
    }
}
