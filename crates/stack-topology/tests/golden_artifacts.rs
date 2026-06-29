//! End-to-end validation that the planner + renderers materialize *working* artifacts
//! — the compose, Postgres init, `.env`, and Envoy gateway config a Lakehouse dev
//! environment needs — matching what trestle ships today.
//!
//! The bar is **behavioral equivalence**, not byte-identity: the planner improves on
//! trestle's hand-authored output (routes ordered most-specific-first; cluster names
//! derived systematically), so these tests compare the *meaning* of the rendered
//! artifacts against the captured trestle fixtures (parsed Envoy routes/clusters, the
//! database set, the env-var set, the include list) and assert the rendered YAML is
//! valid.
//!
//! Fixtures under `tests/fixtures/default/` were captured by running the real
//! `trestle new` for the default lakehouse selection (envoy + seaweedfs + postgres +
//! unity-catalog + mlflow).

use std::collections::{BTreeMap, BTreeSet};

use olai_stack_topology::{
    Catalog, EnvoyOpts, PlanCtx, Selection, baseline_catalog, plan, render_all,
};
use serde_yaml::Value;

const ENVOY_FIXTURE: &str = include_str!("fixtures/default/config/envoy.yaml");
const PG_FIXTURE: &str = include_str!("fixtures/default/config/init-databases.sh");
const ENV_FIXTURE: &str = include_str!("fixtures/default/.env.example");
const COMPOSE_FIXTURE: &str = include_str!("fixtures/default/compose.yaml");

/// The default lakehouse selection used to capture the fixtures.
fn default_selection() -> Selection {
    Selection::modules([
        "local-stack-envoy",
        "local-stack-seaweedfs",
        "local-stack-postgres",
        "local-stack-unity-catalog",
        "local-stack-mlflow",
    ])
}

fn render(selection: &Selection) -> olai_stack_topology::Artifacts {
    let p = plan(
        selection,
        &baseline_catalog(),
        &PlanCtx {
            env_name: "lh-ref".into(),
            ..Default::default()
        },
    )
    .expect("plan should succeed");
    render_all(&p, &EnvoyOpts::default())
}

/// One parsed Envoy route: prefix → (cluster, optional rewrite substitution).
type RouteMap = BTreeMap<String, (String, Option<String>)>;

/// Parse an Envoy config's routes into `prefix → (cluster, rewrite)` and its route
/// order (prefixes in declared order), plus its clusters as `name → host:port`.
fn parse_envoy(yaml: &str) -> (RouteMap, Vec<String>, BTreeMap<String, String>) {
    let doc: Value = serde_yaml::from_str(yaml).expect("rendered Envoy must be valid YAML");

    let vh = &doc["static_resources"]["listeners"][0]["filter_chains"][0]["filters"][0]["typed_config"]
        ["route_config"]["virtual_hosts"][0];
    let mut routes = RouteMap::new();
    let mut order = Vec::new();
    // An environment with no surface endpoints emits an empty (null) routes block.
    let empty = Vec::new();
    let route_seq = vh["routes"].as_sequence().unwrap_or(&empty);
    for r in route_seq {
        let prefix = r["match"]["prefix"].as_str().unwrap().to_string();
        let cluster = r["route"]["cluster"].as_str().unwrap().to_string();
        let rewrite = r["route"]["regex_rewrite"]["substitution"]
            .as_str()
            .map(|s| s.to_string());
        order.push(prefix.clone());
        routes.insert(prefix, (cluster, rewrite));
    }

    let mut clusters = BTreeMap::new();
    let no_clusters = Vec::new();
    let cluster_seq = doc["static_resources"]["clusters"]
        .as_sequence()
        .unwrap_or(&no_clusters);
    for c in cluster_seq {
        let name = c["name"].as_str().unwrap().to_string();
        let sock = &c["load_assignment"]["endpoints"][0]["lb_endpoints"][0]["endpoint"]["address"]
            ["socket_address"];
        let host = sock["address"].as_str().unwrap();
        let port = sock["port_value"].as_u64().unwrap();
        clusters.insert(name, format!("{host}:{port}"));
    }
    (routes, order, clusters)
}

#[test]
fn envoy_routes_and_clusters_match_trestle_semantically() {
    let arts = render(&default_selection());
    let (got, order, got_clusters) = parse_envoy(&arts.envoy);
    let (want, _, _) = parse_envoy(ENVOY_FIXTURE);

    // Same set of client prefixes fronted.
    let got_prefixes: BTreeSet<_> = got.keys().cloned().collect();
    let want_prefixes: BTreeSet<_> = want.keys().cloned().collect();
    assert_eq!(
        got_prefixes, want_prefixes,
        "rendered Envoy fronts a different set of prefixes than trestle"
    );

    // Each prefix routes to the same upstream service and applies the same rewrite
    // (matching by cluster's host:port, since the planner names clusters by service).
    let fixture_cluster_target = {
        let (_, _, c) = parse_envoy(ENVOY_FIXTURE);
        c
    };
    for (prefix, (got_cluster, got_rewrite)) in &got {
        let (want_cluster, want_rewrite) = &want[prefix];
        assert_eq!(
            got_clusters.get(got_cluster),
            fixture_cluster_target.get(want_cluster),
            "prefix {prefix} routes to a different upstream than trestle"
        );
        assert_eq!(
            got_rewrite, want_rewrite,
            "prefix {prefix} has a different rewrite than trestle"
        );
    }

    // Behavioral improvement: routes are ordered most-specific (longest) first, so a
    // shorter prefix never shadows a longer one.
    let lens: Vec<usize> = order.iter().map(String::len).collect();
    let mut sorted = lens.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(
        lens, sorted,
        "rendered Envoy routes must be longest-prefix-first"
    );

    // The OTel route specifically passes through unchanged (no rewrite).
    assert_eq!(got["/api/2.0/otel"].1, None);
    // The MLflow tracking API rewrites under the service base path.
    assert_eq!(
        got["/api/2.0/mlflow"].1.as_deref(),
        Some("/mlflow/api/2.0/mlflow\\1")
    );
}

#[test]
fn postgres_init_creates_the_same_databases() {
    let arts = render(&default_selection());
    let dbs = |s: &str| -> BTreeSet<String> {
        s.lines()
            .filter_map(|l| l.trim().strip_prefix("CREATE DATABASE "))
            .map(|l| l.trim_end_matches(';').to_string())
            .collect()
    };
    assert_eq!(
        dbs(&arts.postgres_init),
        dbs(PG_FIXTURE),
        "rendered Postgres init creates a different set of databases than trestle"
    );
    // The script is a well-formed heredoc.
    assert!(arts.postgres_init.contains("<<-SQL"));
    assert!(arts.postgres_init.trim_end().ends_with("SQL"));
}

#[test]
fn env_file_preserves_trestle_vars_and_adds_injected_coordinates() {
    let arts = render(&default_selection());
    let kv = |s: &str| -> BTreeMap<String, String> {
        s.lines()
            .filter(|l| !l.trim_start().starts_with('#') && l.contains('='))
            .map(|l| {
                let (k, v) = l.split_once('=').unwrap();
                (k.to_string(), v.to_string())
            })
            .collect()
    };
    let got = kv(&arts.env);
    let want = kv(ENV_FIXTURE);

    // Every var trestle shipped is still present and unchanged, with one deliberate
    // exception: `MLFLOW_S3_ENDPOINT_URL`. Trestle pointed it at the host
    // (`http://localhost:${SEAWEEDFS_S3_PORT}`), which is wrong from inside the compose
    // network; the planner now injects the object_store provider's in-network `endpoint`
    // coordinate (`http://seaweedfs:8333`) instead — a behavioral fix, not a regression.
    for (k, v) in &want {
        if k == "MLFLOW_S3_ENDPOINT_URL" {
            continue;
        }
        assert_eq!(
            got.get(k),
            Some(v),
            "rendered .env dropped or changed trestle var `{k}`"
        );
    }
    assert_eq!(
        got.get("MLFLOW_S3_ENDPOINT_URL").map(String::as_str),
        Some("http://seaweedfs:8333"),
        "MLFLOW_S3_ENDPOINT_URL should be the in-network object_store endpoint"
    );

    // The coordinate-injection rework adds these role-generic coordinates, sourced from the
    // chosen providers rather than hard-coded in fragments. Each consumer maps a role-generic
    // coordinate to its own service-specific key.
    assert_eq!(
        got.get("MLFLOW_ARTIFACTS_DESTINATION").map(String::as_str),
        Some("s3://mlflow"),
        "MLflow's artifact destination is injected from the object_store `uri` coordinate"
    );
    assert_eq!(
        got.get("MLFLOW_BACKEND_STORE_URI").map(String::as_str),
        Some(
            "postgresql://${POSTGRES_USER:-postgres}:${POSTGRES_PASSWORD:-postgres}@db:5432/mlflow"
        ),
        "MLflow's backend store is injected from the relational_db `url` coordinate"
    );
    assert_eq!(
        got.get("S3_ENDPOINT").map(String::as_str),
        Some("http://seaweedfs:8333"),
        "UC's S3 endpoint is injected from the object_store `endpoint` coordinate"
    );
    // The S3 credentials still come from the chosen provider's own env contribution (not
    // injected per-consumer), so they remain present with the SeaweedFS values.
    assert_eq!(
        got.get("AWS_ACCESS_KEY_ID").map(String::as_str),
        Some("seaweedfs")
    );
    assert_eq!(
        got.get("AWS_DEFAULT_REGION").map(String::as_str),
        Some("us-east-1")
    );
}

#[test]
fn compose_includes_the_same_fragments() {
    let arts = render(&default_selection());
    // Valid YAML.
    let _: Value =
        serde_yaml::from_str(&arts.compose).expect("rendered compose must be valid YAML");
    let includes = |s: &str| -> BTreeSet<String> {
        s.lines()
            .filter_map(|l| l.trim().strip_prefix("- path: "))
            .map(str::to_string)
            .collect()
    };
    assert_eq!(
        includes(&arts.compose),
        includes(COMPOSE_FIXTURE),
        "rendered compose includes a different set of fragments than trestle"
    );
    // Unlike the captured fixture (generated without a name), the planner always
    // names the project.
    assert!(arts.compose.contains("name: lh-ref"));
}

#[test]
fn unity_catalog_template_branches_on_the_object_store_credential() {
    use olai_stack_topology::ModuleId;

    // UC's fragment is a `RenderSpec::Template`: it branches on the chosen object-store
    // credential flavour, so the rendered compose differs between an S3 and an Azure backend.
    // Select UC + its hard `requires` only, letting the object_store demand resolve via the
    // catalog default / `ctx` preference (so the chosen provider is unambiguous).
    let uc_fragment = |ctx: PlanCtx| -> String {
        let sel = Selection::modules(["local-stack-unity-catalog"]);
        let p = plan(&sel, &baseline_catalog(), &ctx).expect("plan succeeds");
        let (_, out) = p
            .renders
            .iter()
            .find(|(id, _)| id == &ModuleId::from("local-stack-unity-catalog"))
            .expect("UC is in the render set");
        // Valid YAML in either branch.
        let _: Value =
            serde_yaml::from_str(&out.fragment).expect("rendered UC fragment must be valid YAML");
        out.fragment.clone()
    };

    // Default → SeaweedFS (S3): static AWS keys from the typed credential, a `seaweedfs-init`
    // dependency, and no `${AWS_*:-}` fallback hack or Azure connection string.
    let s3 = uc_fragment(PlanCtx::default());
    assert!(s3.contains("seaweedfs-init:"), "S3 init dependency: {s3}");
    assert!(s3.contains("AWS_ACCESS_KEY_ID: seaweedfs"), "S3 keys: {s3}");
    assert!(
        !s3.contains("${AWS_ACCESS_KEY_ID:-"),
        "no fallback hack: {s3}"
    );
    assert!(
        !s3.contains("AZURE_STORAGE_CONNECTION_STRING"),
        "no Azure leak: {s3}"
    );

    // Azurite-preferred → the Azure branch: a connection string, an `azurite-init`
    // dependency, and no AWS keys.
    let mut preference = BTreeMap::new();
    preference.insert(
        "object_store".to_string(),
        vec![
            ModuleId::from("local-stack-azurite"),
            ModuleId::from("local-stack-seaweedfs"),
        ],
    );
    let azure = uc_fragment(PlanCtx {
        provider_preference: preference,
        ..Default::default()
    });
    // Assert on the rendered compose body, not the header comment (which names both inits).
    let azure_yaml: Value = serde_yaml::from_str(&azure).expect("valid YAML");
    let uc = &azure_yaml["services"]["unitycatalog"];
    assert!(
        !uc["depends_on"]["azurite-init"].is_null(),
        "Azure init dependency: {azure}"
    );
    assert!(
        uc["depends_on"]["seaweedfs-init"].is_null(),
        "no S3 init under Azure: {azure}"
    );
    let env = &uc["environment"];
    assert_eq!(
        env["AZURE_STORAGE_CONNECTION_STRING"]
            .as_str()
            .map(|s| s.starts_with("DefaultEndpointsProtocol=")),
        Some(true),
        "Azure connection string from the typed credential: {azure}"
    );
    assert!(
        env["AWS_ACCESS_KEY_ID"].is_null(),
        "no AWS keys under Azure: {azure}"
    );
}

#[test]
fn mlflow_template_uses_base_path_and_planner_driven_depends_on() {
    use olai_stack_topology::ModuleId;

    // MLflow's fragment is a `RenderSpec::Template`: `--static-prefix` and the healthcheck
    // path come from the planner's chosen `BASE_PATH`, the artifact-store env branches on the
    // object-store credential flavour, and `depends_on` is driven by the chosen providers'
    // gates (db healthy + the object-store init completed) rather than hard-coded.
    let mlflow_fragment = |ctx: PlanCtx| -> String {
        let sel = Selection::modules(["local-stack-mlflow"]);
        let p = plan(&sel, &baseline_catalog(), &ctx).expect("plan succeeds");
        let (_, out) = p
            .renders
            .iter()
            .find(|(id, _)| id == &ModuleId::from("local-stack-mlflow"))
            .expect("MLflow is in the render set");
        let _: Value = serde_yaml::from_str(&out.fragment)
            .expect("rendered MLflow fragment must be valid YAML");
        out.fragment.clone()
    };

    // Default → SeaweedFS (S3).
    let s3 = mlflow_fragment(PlanCtx::default());
    let s3_yaml: Value = serde_yaml::from_str(&s3).unwrap();
    let svc = &s3_yaml["services"]["mlflow"];

    // The base path drives both `--static-prefix` and the healthcheck URL — not a literal.
    let command: Vec<String> = svc["command"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let prefix_idx = command.iter().position(|a| a == "--static-prefix").unwrap();
    assert_eq!(
        command[prefix_idx + 1],
        "/mlflow",
        "static-prefix is BASE_PATH"
    );
    let health = svc["healthcheck"]["test"][1].as_str().unwrap();
    assert!(
        health.contains("/mlflow/health"),
        "healthcheck path: {health}"
    );

    // S3 branch: static AWS keys from the typed credential (no `:-` fallback), no Azure leak.
    let env = &svc["environment"];
    assert_eq!(env["AWS_ACCESS_KEY_ID"].as_str(), Some("seaweedfs"));
    assert!(
        !s3.contains("${AWS_ACCESS_KEY_ID:-"),
        "no fallback hack: {s3}"
    );
    assert!(env["AZURE_STORAGE_CONNECTION_STRING"].is_null());

    // depends_on follows the chosen providers: db healthy + seaweedfs-init completed.
    let dep = &svc["depends_on"];
    assert_eq!(dep["db"]["condition"].as_str(), Some("service_healthy"));
    assert_eq!(
        dep["seaweedfs-init"]["condition"].as_str(),
        Some("service_completed_successfully")
    );
    assert!(dep["azurite-init"].is_null(), "no Azure init under S3");

    // Azurite-preferred → the object-store gate and env switch to the Azure backend.
    let mut preference = BTreeMap::new();
    preference.insert(
        "object_store".to_string(),
        vec![
            ModuleId::from("local-stack-azurite"),
            ModuleId::from("local-stack-seaweedfs"),
        ],
    );
    let azure = mlflow_fragment(PlanCtx {
        provider_preference: preference,
        ..Default::default()
    });
    let azure_yaml: Value = serde_yaml::from_str(&azure).unwrap();
    let svc = &azure_yaml["services"]["mlflow"];
    assert!(
        !svc["environment"]["AZURE_STORAGE_CONNECTION_STRING"].is_null(),
        "Azure connection string present: {azure}"
    );
    assert!(
        svc["environment"]["AWS_ACCESS_KEY_ID"].is_null(),
        "no AWS keys under Azure"
    );
    assert!(
        !svc["depends_on"]["azurite-init"].is_null(),
        "Azure init dependency: {azure}"
    );
    assert!(
        svc["depends_on"]["seaweedfs-init"].is_null(),
        "no S3 init under Azure"
    );
}

#[test]
fn adding_trino_and_jaeger_aggregates_their_routes() {
    // A variant selection exercises route/cluster aggregation beyond the default set.
    let sel = Selection::modules([
        "local-stack-envoy",
        "local-stack-seaweedfs",
        "local-stack-postgres",
        "local-stack-mlflow",
        "local-stack-trino",
        "local-stack-jaeger",
    ]);
    let arts = render(&sel);
    let (routes, order, clusters) = parse_envoy(&arts.envoy);

    // Trino and Jaeger UIs are fronted at their base paths.
    assert!(routes.contains_key("/trino"), "missing /trino route");
    assert!(routes.contains_key("/jaeger"), "missing /jaeger route");
    assert_eq!(
        clusters.get("trino").map(String::as_str),
        Some("trino:8080")
    );
    assert_eq!(
        clusters.get("jaeger").map(String::as_str),
        Some("jaeger:16686")
    );

    // Still longest-first overall.
    let lens: Vec<usize> = order.iter().map(String::len).collect();
    let mut sorted = lens.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(lens, sorted);
}

#[test]
fn empty_catalog_renders_a_valid_empty_envoy() {
    // No modules → a valid, route-less Envoy config (no panic, parses cleanly).
    let p = plan(&Selection::default(), &Catalog::new(), &PlanCtx::default()).unwrap();
    let arts = render_all(&p, &EnvoyOpts::default());
    let (routes, _, clusters) = parse_envoy(&arts.envoy);
    assert!(routes.is_empty());
    assert!(clusters.is_empty());
}
