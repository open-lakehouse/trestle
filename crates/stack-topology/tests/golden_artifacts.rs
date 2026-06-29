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
fn env_file_has_the_same_variables() {
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
    assert_eq!(
        kv(&arts.env),
        kv(ENV_FIXTURE),
        "rendered .env has different variables than trestle"
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
