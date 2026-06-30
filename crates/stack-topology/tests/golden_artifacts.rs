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

use olai_stack_topology::{AppUpstream, Catalog, PlanCtx, Selection, baseline_catalog, render_all};
use serde_yaml::Value;

const ENVOY_FIXTURE: &str = include_str!("fixtures/default/config/envoy.yaml");
const PG_FIXTURE: &str = include_str!("fixtures/default/config/init-databases.sh");
const COMPOSE_FIXTURE: &str = include_str!("fixtures/default/compose.yaml");

/// The default lakehouse selection used to capture the fixtures.
fn default_selection() -> Selection {
    Selection::modules(["envoy", "seaweedfs", "postgres", "unity-catalog", "mlflow"])
}

fn render(selection: &Selection) -> olai_stack_topology::Artifacts {
    let p = baseline_catalog()
        .plan(
            selection,
            &PlanCtx {
                env_name: "lh-ref".into(),
                ..Default::default()
            },
        )
        .expect("plan should succeed");
    render_all(&p)
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
    use olai_stack_topology::ModuleId;

    // The Postgres init script is now a module-rendered config file: the postgres module
    // emits it as a `RenderFile` (alias `postgres_init`), templated from the databases the
    // planner hands the provider as `RenderCtx.objects`. Pull it from the render output.
    let p = baseline_catalog()
        .plan(
            &default_selection(),
            &PlanCtx {
                env_name: "lh-ref".into(),
                ..Default::default()
            },
        )
        .expect("plan should succeed");
    let (_, out) = p
        .renders
        .iter()
        .find(|(id, _)| id == &ModuleId::from("postgres"))
        .expect("postgres is in the render set");
    let init = out
        .files
        .iter()
        .find(|f| f.alias.as_deref() == Some("postgres_init"))
        .expect("postgres declares a `postgres_init` config file");
    // It is co-located under the module's directory.
    assert_eq!(init.path, "modules/postgres/init-databases.sh");

    let dbs = |s: &str| -> BTreeSet<String> {
        s.lines()
            .filter_map(|l| l.trim().strip_prefix("CREATE DATABASE "))
            .map(|l| l.trim_end_matches(';').to_string())
            .collect()
    };
    assert_eq!(
        dbs(&init.contents),
        dbs(PG_FIXTURE),
        "rendered Postgres init creates a different set of databases than trestle"
    );
    // The script is a well-formed heredoc.
    assert!(init.contents.contains("<<-SQL"));
    assert!(init.contents.trim_end().ends_with("SQL"));
}

#[test]
fn env_file_emits_only_keys_referenced_as_compose_substitutions() {
    // The `.env` overlay is audited: it lists a key only when some rendered artifact (a module
    // fragment or a mounted config file) still references it as a `${KEY}` compose
    // substitution. Because the baseline modules render every coordinate *concrete* (the
    // fragments read `{{ db.url }}`, `{{ obj.credential.* }}` directly), nothing defers a value
    // to compose, so the audited `.env` for the default lakehouse is empty of keys.
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

    // Collect every `${KEY}` reference across the rendered corpus (fragments + mounted files +
    // the Envoy bootstrap). Every emitted key must appear here; nothing else may.
    let p = baseline_catalog()
        .plan(
            &default_selection(),
            &PlanCtx {
                env_name: "lh-ref".into(),
                ..Default::default()
            },
        )
        .expect("plan should succeed");
    let mut corpus = vec![arts.envoy.clone()];
    for (_, out) in &p.renders {
        corpus.push(out.fragment.clone());
        corpus.extend(out.files.iter().map(|f| f.contents.clone()));
    }
    let referenced: BTreeSet<String> = corpus
        .iter()
        .flat_map(|t| {
            t.match_indices("${").filter_map(|(i, _)| {
                let rest = &t[i + 2..];
                let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))?;
                let name = &rest[..end];
                (!name.is_empty() && !name.as_bytes()[0].is_ascii_digit()).then(|| name.to_string())
            })
        })
        .collect();

    for k in got.keys() {
        assert!(
            referenced.contains(k),
            "`{k}` is emitted into .env but referenced by no rendered fragment/file"
        );
    }

    // Concretely-consumed coordinates that used to round-trip through `.env` are now gone:
    // they are read straight from the resolved connection in the fragments.
    for k in [
        "MLFLOW_ARTIFACTS_DESTINATION",
        "MLFLOW_BACKEND_STORE_URI",
        "S3_ENDPOINT",
        "UC_DATABASE_URL",
        "AWS_ACCESS_KEY_ID",
    ] {
        assert!(
            !got.contains_key(k),
            "`{k}` should no longer be emitted into .env (consumed concretely at render time)"
        );
    }
}

#[test]
fn compose_includes_the_same_fragments() {
    let arts = render(&default_selection());
    // Valid YAML.
    let _: Value =
        serde_yaml::from_str(&arts.compose).expect("rendered compose must be valid YAML");
    // Compare the *set of modules* included, not raw paths: the layout moved from trestle's
    // `./docker/compose/<id>.yaml` to a per-module `./modules/<id>/compose.yaml`, so derive the
    // module id from each include path on both sides.
    let module_ids = |s: &str| -> BTreeSet<String> {
        s.lines()
            .filter_map(|l| l.trim().strip_prefix("- path: "))
            .filter_map(|p| {
                let p = p.trim_start_matches("./");
                if let Some(rest) = p.strip_prefix("modules/") {
                    rest.strip_suffix("/compose.yaml").map(str::to_string)
                } else if let Some(rest) = p.strip_prefix("docker/compose/") {
                    rest.strip_suffix(".yaml").map(str::to_string)
                } else {
                    None
                }
            })
            .collect()
    };
    assert_eq!(
        module_ids(&arts.compose),
        module_ids(COMPOSE_FIXTURE),
        "rendered compose includes a different set of modules than trestle"
    );
    // Unlike the captured fixture (generated without a name), the planner always
    // names the project.
    assert!(arts.compose.contains("name: lh-ref"));
}

#[test]
fn compose_declares_config_aliases_for_mounted_files() {
    let arts = render(&default_selection());
    let doc: Value =
        serde_yaml::from_str(&arts.compose).expect("rendered compose must be valid YAML");

    // The gateway's Envoy bootstrap and the postgres init script are both declared as
    // top-level `configs:` entries, each pointing at its per-module file.
    let configs = &doc["configs"];
    assert_eq!(
        configs["envoy_config"]["file"].as_str(),
        Some("./modules/envoy/envoy.yaml"),
        "envoy_config must map to the gateway module's bootstrap: {}",
        arts.compose
    );
    assert_eq!(
        configs["postgres_init"]["file"].as_str(),
        Some("./modules/postgres/init-databases.sh"),
        "postgres_init must map to the postgres module's init script: {}",
        arts.compose
    );

    // And the fragments mount them by alias rather than by a bind-mount path.
    let p = baseline_catalog()
        .plan(
            &default_selection(),
            &PlanCtx {
                env_name: "lh-ref".into(),
                ..Default::default()
            },
        )
        .expect("plan should succeed");
    let fragment = |id: &str| -> String {
        use olai_stack_topology::ModuleId;
        p.renders
            .iter()
            .find(|(m, _)| m == &ModuleId::from(id))
            .map(|(_, out)| out.fragment.clone())
            .unwrap_or_default()
    };
    let envoy = fragment("envoy");
    assert!(
        envoy.contains("source: envoy_config"),
        "envoy fragment mounts the config by alias: {envoy}"
    );
    let pg = fragment("postgres");
    assert!(
        pg.contains("source: postgres_init"),
        "postgres fragment mounts the init script by alias: {pg}"
    );
    // No stale `docker/` bind-mount paths remain.
    assert!(
        !envoy.contains("docker/envoy") && !pg.contains("docker/postgres"),
        "fragments should no longer reference the old docker/ layout"
    );
}

#[test]
fn unity_catalog_template_branches_on_the_object_store_credential() {
    use olai_stack_topology::ModuleId;

    // UC's fragment is a `RenderSpec`: it branches on the chosen object-store
    // credential flavour, so the rendered compose differs between an S3 and an Azure backend.
    // Select UC + its hard `requires` only, letting the object_store demand resolve via the
    // catalog default / `ctx` preference (so the chosen provider is unambiguous).
    let uc_fragment = |ctx: PlanCtx| -> String {
        let sel = Selection::modules(["unity-catalog"]);
        let p = baseline_catalog().plan(&sel, &ctx).expect("plan succeeds");
        let (_, out) = p
            .renders
            .iter()
            .find(|(id, _)| id == &ModuleId::from("unity-catalog"))
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
    // The fragment is rendered whole from the render context — no compose `${VAR}` left to
    // resolve at run time (the database URL itself carries `${POSTGRES_*}` defaults, so scope
    // the check to the lines that used to be `${VAR}` indirections).
    assert!(
        s3.contains("image: unitycatalog/unitycatalog:main-2f2e32d"),
        "image pinned inline, not via ${{UC_IMAGE}}: {s3}"
    );
    assert!(
        !s3.contains("${UC_IMAGE")
            && !s3.contains("${S3_ENDPOINT")
            && !s3.contains("${UC_DATABASE_URL"),
        "UC no longer round-trips coordinates through compose ${{VAR}} refs: {s3}"
    );
    let s3_yaml: Value = serde_yaml::from_str(&s3).unwrap();
    let s3_env = &s3_yaml["services"]["unitycatalog"]["environment"];
    assert_eq!(
        s3_env["S3_ENDPOINT"].as_str(),
        Some("http://envoy:9100"),
        "S3_ENDPOINT comes from the resolved object_store endpoint: {s3}"
    );
    assert!(
        s3_env["DATABASE_URL"]
            .as_str()
            .is_some_and(|u| u.contains("@db:5432/unitycatalog")),
        "DATABASE_URL comes from the resolved relational_db url: {s3}"
    );

    // Azurite-preferred → the Azure branch: a connection string, an `azurite-init`
    // dependency, and no AWS keys.
    let mut preference = BTreeMap::new();
    preference.insert(
        "object_store".to_string(),
        vec![ModuleId::from("azurite"), ModuleId::from("seaweedfs")],
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

    // MLflow's fragment is a `RenderSpec`: `--static-prefix` and the healthcheck
    // path come from the planner's chosen `BASE_PATH`, the artifact-store env branches on the
    // object-store credential flavour, and `depends_on` is driven by the chosen providers'
    // gates (db healthy + the object-store init completed) rather than hard-coded.
    let mlflow_fragment = |ctx: PlanCtx| -> String {
        let sel = Selection::modules(["mlflow"]);
        let p = baseline_catalog().plan(&sel, &ctx).expect("plan succeeds");
        let (_, out) = p
            .renders
            .iter()
            .find(|(id, _)| id == &ModuleId::from("mlflow"))
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
        vec![ModuleId::from("azurite"), ModuleId::from("seaweedfs")],
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
fn azurite_fragment_is_rendered_whole_from_typed_context() {
    use olai_stack_topology::ModuleId;

    // Azurite is a `RenderSpec` rendered entirely from the typed `RenderCtx` — its
    // own connection (the connection string) and the provisioned container names (`objects`)
    // — with no compose `${VAR}` substitution. Prefer Azurite so it is the chosen object_store
    // and its init provisions the demanded containers.
    let mut preference = BTreeMap::new();
    preference.insert(
        "object_store".to_string(),
        vec![ModuleId::from("azurite"), ModuleId::from("seaweedfs")],
    );
    let sel = Selection::modules(["mlflow"]);
    let p = baseline_catalog()
        .plan(
            &sel,
            &PlanCtx {
                provider_preference: preference,
                ..Default::default()
            },
        )
        .expect("plan succeeds");
    let (_, out) = p
        .renders
        .iter()
        .find(|(id, _)| id == &ModuleId::from("azurite"))
        .expect("azurite is the chosen object_store provider");
    let frag = &out.fragment;

    // Valid YAML, and the init service exists.
    let doc: Value = serde_yaml::from_str(frag).expect("azurite fragment must be valid YAML");
    let init = &doc["services"]["azurite-init"];

    // The connection string came from the typed credential, not a `${VAR}` placeholder.
    assert_eq!(
        init["environment"]["AZURE_STORAGE_CONNECTION_STRING"]
            .as_str()
            .map(|s| s.starts_with("DefaultEndpointsProtocol=")),
        Some(true),
        "connection string rendered from typed credential: {frag}"
    );
    // The container-init iterates the provisioned `objects` (MLflow demands one).
    assert!(
        init["entrypoint"][2]
            .as_str()
            .unwrap()
            .contains("az storage container create --name mlflow"),
        "init iterates provisioned container names: {frag}"
    );
    // Durable blob state lives under the stack's data root, baked in at plan time from
    // `{{ env.DATA_ROOT }}` (default ./.data) — not a hard-coded `./.data/azurite`.
    assert!(
        frag.contains("./.data/azurite:/data"),
        "blob state persisted under the stack data root: {frag}"
    );
    // No compose substitution remains anywhere in the rendered fragment body — every value,
    // including the data root, is resolved from the typed/render context at plan time.
    let body: String = frag
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect();
    assert!(
        !body.contains("${"),
        "no leftover ${{VAR}} substitution: {frag}"
    );
}

#[test]
fn object_store_gets_a_dedicated_envoy_listener_fronted_at_root() {
    // An object store is `Gatewayed`: the planner gives it its own Envoy listener serving `/`
    // (not a shared-listener path prefix, which would break S3/Blob URL construction), the
    // gateway publishes that port, and the consumer endpoint points through it.
    let arts = render(&default_selection());
    let doc: Value = serde_yaml::from_str(&arts.envoy).expect("valid Envoy YAML");
    let listeners = doc["static_resources"]["listeners"].as_sequence().unwrap();

    // Two listeners: the shared gateway (port 10000) and the object store's dedicated one.
    let ports: BTreeSet<u64> = listeners
        .iter()
        .map(|l| {
            l["address"]["socket_address"]["port_value"]
                .as_u64()
                .unwrap()
        })
        .collect();
    assert!(ports.contains(&10000), "shared listener present: {ports:?}");
    assert!(
        ports.contains(&9100),
        "dedicated object-store listener present: {ports:?}"
    );

    // The dedicated listener fronts the seaweedfs cluster at `/` (origin, no prefix/rewrite).
    let dedicated = listeners
        .iter()
        .find(|l| l["address"]["socket_address"]["port_value"].as_u64() == Some(9100))
        .unwrap();
    let route = &dedicated["filter_chains"][0]["filters"][0]["typed_config"]["route_config"]["virtual_hosts"]
        [0]["routes"][0];
    assert_eq!(route["match"]["prefix"].as_str(), Some("/"));
    assert_eq!(route["route"]["cluster"].as_str(), Some("seaweedfs"));
    assert!(
        route["route"]["regex_rewrite"].is_null(),
        "no rewrite — the store is served at its origin"
    );

    // The seaweedfs cluster still targets the upstream's real port (8333), not the listener.
    let clusters = doc["static_resources"]["clusters"].as_sequence().unwrap();
    let sw = clusters
        .iter()
        .find(|c| c["name"].as_str() == Some("seaweedfs"))
        .unwrap();
    let sock = &sw["load_assignment"]["endpoints"][0]["lb_endpoints"][0]["endpoint"]["address"]["socket_address"];
    assert_eq!(sock["address"].as_str(), Some("seaweedfs"));
    assert_eq!(sock["port_value"].as_u64(), Some(8333));

    // The gateway compose fragment publishes the dedicated port on the host.
    let p = baseline_catalog()
        .plan(
            &default_selection(),
            &PlanCtx {
                env_name: "lh-ref".into(),
                ..Default::default()
            },
        )
        .expect("plan succeeds");
    let (_, envoy_frag) = p
        .renders
        .iter()
        .find(|(id, _)| id == &olai_stack_topology::ModuleId::from("envoy"))
        .unwrap();
    let envoy_doc: Value = serde_yaml::from_str(&envoy_frag.fragment).unwrap();
    let published: BTreeSet<String> = envoy_doc["services"]["envoy"]["ports"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap().to_string())
        .collect();
    assert!(
        published.contains("9100:9100"),
        "envoy publishes the dedicated port: {published:?}"
    );
}

#[test]
fn adding_jaeger_aggregates_its_routes() {
    // A variant selection exercises route/cluster aggregation beyond the default set.
    let sel = Selection::modules(["envoy", "seaweedfs", "postgres", "mlflow", "jaeger"]);
    let arts = render(&sel);
    let (routes, order, clusters) = parse_envoy(&arts.envoy);

    // The Jaeger UI is fronted at its base path, with a derived cluster.
    assert!(routes.contains_key("/jaeger"), "missing /jaeger route");
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
fn adding_headwaters_fronts_its_whole_surface_under_one_prefix() {
    use olai_stack_topology::ModuleId;

    // With the UI on (the default), Headwaters relocates its whole surface under `/lineage`:
    // the UI is fronted there (prefixable, forwarded unchanged), and its static Databricks-style
    // API path `/api/v1/lineage` is rewritten to carry the base path
    // (`/lineage/api/v1/lineage`), since that is where the relocated server serves it. Both
    // routes target the one headwaters cluster. It demands only a relational_db (named
    // `lineage`); selecting it must auto-provision Postgres and the `lineage` database.
    let sel = Selection::modules(["envoy", "postgres", "headwaters"]);
    let arts = render(&sel);
    let (routes, _, clusters) = parse_envoy(&arts.envoy);

    // The UI route at /lineage, forwarded unchanged, to the headwaters cluster.
    let (ui_cluster, ui_rewrite) = routes
        .get("/lineage")
        .expect("missing /lineage UI route in the rendered Envoy config");
    assert_eq!(ui_cluster, "headwaters");
    assert_eq!(*ui_rewrite, None, "the UI prefix is forwarded unchanged");
    // The API route at the static path, rewritten to carry the base path.
    let (api_cluster, api_rewrite) = routes
        .get("/api/v1/lineage")
        .expect("missing /api/v1/lineage API route in the rendered Envoy config");
    assert_eq!(api_cluster, "headwaters");
    // The rendered Envoy substitution carries the `\1` capture suffix (the matched remainder
    // of the path), so the base-path-prefixed upstream is `/lineage/api/v1/lineage\1`.
    assert_eq!(
        api_rewrite.as_deref(),
        Some(r"/lineage/api/v1/lineage\1"),
        "the API path carries the base path when the UI is on"
    );
    assert_eq!(
        clusters.get("headwaters").map(String::as_str),
        Some("headwaters:8091")
    );

    // The planner provisioned the demanded `lineage` database on Postgres.
    let p = baseline_catalog().plan(&sel, &PlanCtx::default()).unwrap();
    assert!(p.graph.module(&ModuleId::from("postgres")).is_some());
    assert!(p.postgres_databases.contains(&"lineage".to_string()));

    // The chosen base path is injected back for the module's render.
    assert_eq!(
        p.injected
            .get(&ModuleId::from("headwaters"))
            .and_then(|e| e.get("BASE_PATH")),
        Some("/lineage")
    );

    // The rendered fragment keeps the DSN as an env (the secret stays out of the config
    // file), points the container at the mounted `config.toml`, gates on the Postgres
    // healthcheck, and declares its own distroless self-healthcheck — with no leftover ${VAR}.
    let (_, out) = p
        .renders
        .iter()
        .find(|(id, _)| id == &ModuleId::from("headwaters"))
        .expect("headwaters is in the render set");
    let doc: Value =
        serde_yaml::from_str(&out.fragment).expect("headwaters fragment must be valid YAML");
    let svc = &doc["services"]["headwaters"];
    let env = &svc["environment"];
    assert!(
        env["DATABASE_URL"]
            .as_str()
            .is_some_and(|u| u.contains("@db:5432/lineage")),
        "DATABASE_URL comes from the resolved relational_db url: {}",
        out.fragment
    );
    // The container invokes the headwaters CLI directly, pointing it at the mounted config.
    let command: Vec<&str> = svc["command"]
        .as_sequence()
        .expect("command is a sequence")
        .iter()
        .map(|v| v.as_str().expect("command entries are strings"))
        .collect();
    assert_eq!(
        command,
        ["serve", "--config", "/etc/headwaters/config.toml"]
    );
    // The config file is mounted via the `headwaters_config` alias at the expected target.
    assert_eq!(
        svc["configs"][0]["source"].as_str(),
        Some("headwaters_config")
    );
    assert_eq!(
        svc["configs"][0]["target"].as_str(),
        Some("/etc/headwaters/config.toml")
    );
    assert_eq!(
        svc["depends_on"]["db"]["condition"].as_str(),
        Some("service_healthy")
    );
    // The distroless self-healthcheck: the binary's own `healthcheck` subcommand.
    let healthcheck_test: Vec<&str> = svc["healthcheck"]["test"]
        .as_sequence()
        .expect("healthcheck.test is a sequence")
        .iter()
        .map(|v| v.as_str().expect("healthcheck.test entries are strings"))
        .collect();
    assert_eq!(
        healthcheck_test,
        ["CMD", "/usr/local/bin/app", "healthcheck"]
    );

    // The generated `config.toml` carries the effective config: the UI knob defaults to
    // `true`, and the planner-assigned base path is threaded through.
    let config = out
        .files
        .iter()
        .find(|f| f.alias.as_deref() == Some("headwaters_config"))
        .expect("headwaters renders a config.toml");
    // It is co-located under the module's directory.
    assert_eq!(config.path, "modules/headwaters/config.toml");
    assert!(
        config.contents.contains("serve = true"),
        "UI serve defaults to true: {}",
        config.contents
    );
    assert!(
        config.contents.contains(r#"base_path = "/lineage""#),
        "base_path is the planner-assigned prefix: {}",
        config.contents
    );

    // Network-only backend (no host ports) and no leftover compose substitutions.
    assert!(!out.fragment.contains("ports:"), "{}", out.fragment);
    let body: String = out
        .fragment
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect();
    assert!(
        !body.contains("${"),
        "no leftover ${{VAR}}: {}",
        out.fragment
    );
}

#[test]
fn headwaters_ui_knob_override_turns_off_the_ui() {
    use std::collections::BTreeMap;

    use olai_stack_topology::ModuleId;

    // The `HEADWATERS_SERVE_UI` knob is overridable through the selection: a config UI
    // (hydrofoil / Transler) surfaces it, and the chosen value is fed back here and lands
    // in the generated `config.toml` as `ui.serve`.
    let mut knob_overrides = BTreeMap::new();
    knob_overrides.insert(
        ModuleId::from("headwaters"),
        BTreeMap::from([("HEADWATERS_SERVE_UI".to_string(), "false".to_string())]),
    );
    let sel = Selection {
        modules: vec!["envoy".into(), "postgres".into(), "headwaters".into()],
        capabilities: vec![],
        knob_overrides,
    };

    let p = baseline_catalog().plan(&sel, &PlanCtx::default()).unwrap();
    let (_, out) = p
        .renders
        .iter()
        .find(|(id, _)| id == &ModuleId::from("headwaters"))
        .expect("headwaters is in the render set");
    let config = out
        .files
        .iter()
        .find(|f| f.alias.as_deref() == Some("headwaters_config"))
        .expect("headwaters renders a config.toml");
    assert!(
        config.contents.contains("serve = false"),
        "the override turns the UI off: {}",
        config.contents
    );
    assert!(
        !config.contents.contains("base_path"),
        "no base_path line when the UI is off: {}",
        config.contents
    );

    // With the UI off, there is no UI to relocate: Headwaters serves only its static
    // Databricks-style API path at root, so the gateway matches `/api/v1/lineage` and forwards
    // it unchanged. The prefixable-UI handshake drops away — no `BASE_PATH` is injected, the
    // `AssignedRoute` carries no `base_path`, and there is no `/lineage` route at all.
    let route = p
        .routes
        .get("headwaters", "api")
        .expect("headwaters api endpoint is routed");
    assert_eq!(route.prefix, "/api/v1/lineage");
    assert_eq!(
        route.rewrite, None,
        "API served at its static path, forwarded unchanged"
    );
    assert_eq!(
        route.base_path, None,
        "no prefixable-UI base path when the UI is off"
    );
    assert!(
        matches!(route.listener, olai_stack_topology::Listener::Shared),
        "the API stays on the shared listener"
    );
    assert!(
        p.routes.get("headwaters", "ui").is_none(),
        "no UI endpoint is emitted when the UI is off"
    );
    assert_eq!(
        p.injected
            .get(&ModuleId::from("headwaters"))
            .and_then(|e| e.get("BASE_PATH")),
        None,
        "BASE_PATH is not injected when the UI is off"
    );

    // The gateway route table fronts the static API path (and no `/lineage` route).
    let arts = render_all(&p);
    let (routes, _, _) = parse_envoy(&arts.envoy);
    let (cluster, rewrite) = routes
        .get("/api/v1/lineage")
        .expect("the API path is fronted with the UI off");
    assert_eq!(cluster, "headwaters");
    assert_eq!(*rewrite, None);
    assert!(
        !routes.contains_key("/lineage"),
        "no UI route when the UI is off"
    );
}

#[test]
fn empty_catalog_renders_a_valid_empty_envoy() {
    // No modules → a valid, route-less Envoy config (no panic, parses cleanly).
    let p = Catalog::new()
        .plan(&Selection::default(), &PlanCtx::default())
        .unwrap();
    let arts = render_all(&p);
    let (routes, _, clusters) = parse_envoy(&arts.envoy);
    assert!(routes.is_empty());
    assert!(clusters.is_empty());
}

#[test]
fn materialize_flattens_the_full_layout() {
    // `materialize()` is pure and encodes the on-disk layout once: the top-level files, the
    // Envoy bootstrap, and every module's fragment + config files.
    let p = baseline_catalog()
        .plan(&default_selection(), &PlanCtx::default())
        .expect("plan should succeed");
    let out = p.materialize();
    let paths: BTreeSet<&str> = out.files.iter().map(|f| f.path.as_str()).collect();

    // The stack-level files and the Envoy bootstrap are always present.
    for expected in ["compose.yaml", ".env", "modules/envoy/envoy.yaml"] {
        assert!(paths.contains(expected), "missing {expected}");
    }
    // Each module's fragment is rooted under its own directory.
    assert!(paths.contains("modules/postgres/compose.yaml"));
    assert!(paths.contains("modules/mlflow/compose.yaml"));
    // A module config file (the Postgres init script) is carried with its module-rooted path.
    assert!(paths.contains("modules/postgres/init-databases.sh"));

    // The flattened contents match what `render_all` produces for the stack files.
    let arts = render_all(&p);
    let by_path = |name: &str| {
        out.files
            .iter()
            .find(|f| f.path == name)
            .map(|f| f.contents.as_str())
    };
    assert_eq!(by_path("compose.yaml"), Some(arts.compose.as_str()));
    assert_eq!(
        by_path("modules/envoy/envoy.yaml"),
        Some(arts.envoy.as_str())
    );
}

#[test]
fn app_upstream_becomes_the_gateway_catch_all() {
    // The app upstream is a plan input: setting it on the `PlanCtx` makes the planner emit an
    // `app` cluster and a `/` catch-all route on the shared listener — rendered straight from
    // the gateway config, with no render-time option.
    let p = baseline_catalog()
        .plan(
            &default_selection(),
            &PlanCtx {
                env_name: "lh-ref".into(),
                app: Some(AppUpstream {
                    service: "my-app".into(),
                    port: 8000,
                }),
                ..Default::default()
            },
        )
        .expect("plan should succeed");

    let arts = render_all(&p);
    let (routes, order, clusters) = parse_envoy(&arts.envoy);

    let (cluster, rewrite) = routes.get("/").expect("the app catch-all is fronted");
    assert_eq!(cluster, "app");
    assert_eq!(*rewrite, None);
    assert_eq!(
        order.last().map(String::as_str),
        Some("/"),
        "the catch-all is the least-specific route, so it must be emitted last"
    );
    assert_eq!(
        clusters.get("app").map(String::as_str),
        Some("my-app:8000"),
        "the app cluster points at the configured service:port"
    );
}

#[test]
fn data_root_is_injected_and_relocatable() {
    use olai_stack_topology::{DATA_ROOT_DEFAULT, DATA_ROOT_VAR, ModuleId};

    let frag = |p: &olai_stack_topology::Plan, id: &str| {
        p.renders
            .iter()
            .find(|(m, _)| m == &ModuleId::from(id))
            .map(|(_, out)| out.fragment.clone())
            .unwrap()
    };

    // The data root is injected into *every* module's render env (not just data-bearing ones),
    // defaulting to `./.data` — render-only, so it's resolved into the fragment at plan time.
    let p = baseline_catalog()
        .plan(&default_selection(), &PlanCtx::default())
        .unwrap();
    for module in ["postgres", "seaweedfs", "unity-catalog", "mlflow", "envoy"] {
        assert_eq!(
            p.injected
                .get(&ModuleId::from(module))
                .and_then(|e| e.get(DATA_ROOT_VAR)),
            Some(DATA_ROOT_DEFAULT),
            "{module} should see the default DATA_ROOT"
        );
    }

    // Persisting fragments mount under it by convention, with the default root baked in: a
    // Static fragment via `${DATA_ROOT}` substitution, a Template fragment via `{{ env.DATA_ROOT }}`.
    assert!(frag(&p, "postgres").contains("./.data/postgres:/var/lib/postgresql/data"));
    assert!(frag(&p, "seaweedfs").contains("./.data/seaweedfs:/data"));

    // Azurite is a Template fragment: it bakes the same default root via `{{ env.DATA_ROOT }}`.
    // Prefer it as the object_store so a consumer (mlflow) pulls it in and its fragment renders.
    let p_az = baseline_catalog()
        .plan(
            &Selection::modules(["mlflow"]),
            &PlanCtx {
                provider_preference: BTreeMap::from([(
                    "object_store".to_string(),
                    vec![ModuleId::from("azurite"), ModuleId::from("seaweedfs")],
                )]),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        frag(&p_az, "azurite").contains("./.data/azurite:/data"),
        "azurite (Template) bakes the default root: {}",
        frag(&p_az, "azurite")
    );

    // A custom root relocates every mount through the single knob — no fragment edits, and the
    // baked path follows. Render-only, so it does NOT leak into `.env`.
    let relocated = baseline_catalog()
        .plan(
            &default_selection(),
            &PlanCtx {
                data_root: "/var/lib/mystack".into(),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(frag(&relocated, "postgres").contains("/var/lib/mystack/postgres:"));
    assert!(frag(&relocated, "seaweedfs").contains("/var/lib/mystack/seaweedfs:"));
    assert_eq!(
        relocated.env.get(DATA_ROOT_VAR),
        None,
        "DATA_ROOT is render-only (baked at plan time), so it stays out of .env"
    );
}

#[test]
fn fragments_are_rendered_concrete_with_no_compose_fallbacks() {
    use olai_stack_topology::ModuleId;

    // Every fragment is rendered whole from the typed context: concrete ports, concrete
    // credentials, and zero `${VAR}` compose fallbacks left for runtime resolution. Include
    // jaeger explicitly (it is not in the default selection) so its fragment is rendered too.
    let p = baseline_catalog()
        .plan(
            &Selection::modules([
                "envoy",
                "seaweedfs",
                "postgres",
                "unity-catalog",
                "mlflow",
                "jaeger",
            ]),
            &PlanCtx::default(),
        )
        .unwrap();
    let frag = |id: &str| {
        p.renders
            .iter()
            .find(|(m, _)| m == &ModuleId::from(id))
            .map(|(_, out)| out.fragment.clone())
            .unwrap()
    };

    // No fragment carries a compose `${VAR}` in its rendered body — all values are wired in at
    // plan time. (Header comments may *mention* `${VAR}` explanatorily, so check non-comment
    // lines only.)
    let body = |f: &str| -> String {
        f.lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .collect()
    };
    for id in ["postgres", "seaweedfs", "jaeger", "unity-catalog", "mlflow"] {
        let f = frag(id);
        assert!(
            !body(&f).contains("${"),
            "{id} fragment still has a ${{VAR}} in its body: {f}"
        );
        // Each is valid YAML.
        let _: Value = serde_yaml::from_str(&f).expect("fragment must be valid YAML");
    }

    // Backends are network-only: they `expose:` their ports to the compose network (and the
    // gateway) but never `ports:`-publish to the host, so two stacks rendered on one host don't
    // collide. The Envoy gateway is the sole host-facing surface.
    for id in ["postgres", "seaweedfs", "jaeger"] {
        assert!(
            !frag(id).contains("ports:"),
            "{id} must not publish host ports (network-only): {}",
            frag(id)
        );
    }
    assert!(frag("postgres").contains("expose:"));
    assert!(frag("postgres").contains("\"5432\""));
    assert!(frag("postgres").contains("\"8081\""));
    assert!(frag("seaweedfs").contains("\"9333\""));
    assert!(frag("seaweedfs").contains("\"8333\""));
    assert!(frag("jaeger").contains("\"16686\""));

    // Postgres credentials are concrete, in both the container env and the pgweb URL.
    let pg = frag("postgres");
    assert!(pg.contains("POSTGRES_USER: postgres"));
    assert!(pg.contains("postgres://postgres:postgres@db:5432/postgres"));

    // seaweedfs-init reads its S3 credential from the typed connection (the resolved
    // `seaweedfs`/`us-east-1` values), with no `${AWS_*:-…}` fallback, and iterates the
    // provisioned buckets directly.
    let sw = frag("seaweedfs");
    assert!(sw.contains("AWS_ACCESS_KEY_ID: seaweedfs"));
    assert!(sw.contains("AWS_DEFAULT_REGION: us-east-1"));
    assert!(sw.contains("s3 mb s3://unity"));
    assert!(sw.contains("s3 mb s3://mlflow"));

    // Edge case: seaweedfs selected with no object_store consumer → it provisions no buckets,
    // so its own connection isn't in the render context. The init service (which reads that
    // credential) must be skipped rather than failing to render.
    let p_bare = baseline_catalog()
        .plan(&Selection::modules(["seaweedfs"]), &PlanCtx::default())
        .expect("seaweedfs alone should plan");
    let sw_bare = p_bare
        .renders
        .iter()
        .find(|(m, _)| m == &ModuleId::from("seaweedfs"))
        .map(|(_, out)| out.fragment.clone())
        .unwrap();
    let _: Value =
        serde_yaml::from_str(&sw_bare).expect("bare seaweedfs fragment must be valid YAML");
    assert!(
        !sw_bare.contains("seaweedfs-init"),
        "no buckets → no init service: {sw_bare}"
    );

    // Same edge case for azurite (the other object_store provider): preferred but with no
    // consumer, so it provisions no containers. Its init service reads the credential too, so it
    // must likewise be skipped rather than failing to render against an absent connection.
    let p_az_bare = baseline_catalog()
        .plan(
            &Selection::modules(["azurite"]),
            &PlanCtx {
                provider_preference: BTreeMap::from([(
                    "object_store".to_string(),
                    vec![ModuleId::from("azurite"), ModuleId::from("seaweedfs")],
                )]),
                ..Default::default()
            },
        )
        .expect("azurite alone should plan (no init service when it provisions nothing)");
    let az_bare = p_az_bare
        .renders
        .iter()
        .find(|(m, _)| m == &ModuleId::from("azurite"))
        .map(|(_, out)| out.fragment.clone())
        .unwrap();
    let _: Value =
        serde_yaml::from_str(&az_bare).expect("bare azurite fragment must be valid YAML");
    assert!(
        !az_bare.contains("azurite-init"),
        "no containers → no init service: {az_bare}"
    );
}

/// Helpers for the forward-auth (ENVOY_AUTH knob) golden tests.
mod auth {
    use super::*;
    use olai_stack_topology::ModuleId;
    use std::collections::BTreeMap;

    /// A selection mixing a gatewayed object store (seaweedfs → dedicated listener), an API
    /// surface (mlflow), and a UI surface (headwaters), with the gateway's `ENVOY_AUTH` knob
    /// set to `on`. Exercises the protect/exempt boundary: API/UI on the shared listener get
    /// gated, the object store on its dedicated listener does not.
    fn auth_on_selection() -> Selection {
        let mut knob_overrides = BTreeMap::new();
        knob_overrides.insert(
            ModuleId::from("envoy"),
            BTreeMap::from([("ENVOY_AUTH".to_string(), "true".to_string())]),
        );
        Selection {
            modules: vec![
                "envoy".into(),
                "seaweedfs".into(),
                "postgres".into(),
                "mlflow".into(),
                "headwaters".into(),
            ],
            capabilities: vec![],
            knob_overrides,
        }
    }

    /// The shared listener (port 10000) from a rendered Envoy doc.
    fn shared_listener(doc: &Value) -> &Value {
        doc["static_resources"]["listeners"]
            .as_sequence()
            .unwrap()
            .iter()
            .find(|l| l["address"]["socket_address"]["port_value"].as_u64() == Some(10000))
            .expect("shared listener present")
    }

    fn http_filter_names(listener: &Value) -> Vec<String> {
        listener["filter_chains"][0]["filters"][0]["typed_config"]["http_filters"]
            .as_sequence()
            .unwrap()
            .iter()
            .map(|f| f["name"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn auth_on_gates_only_the_shared_listener() {
        let p = baseline_catalog()
            .plan(&auth_on_selection(), &PlanCtx::default())
            .unwrap();
        let arts = render_all(&p);
        let doc: Value = serde_yaml::from_str(&arts.envoy).expect("valid Envoy YAML");

        // The shared listener runs ext_authz before the router.
        let shared = shared_listener(&doc);
        let filters = http_filter_names(shared);
        assert_eq!(
            filters,
            vec![
                "envoy.filters.http.ext_authz".to_string(),
                "envoy.filters.http.router".to_string(),
            ],
            "ext_authz precedes the router on the shared listener: {filters:?}"
        );

        // Every dedicated listener (the gatewayed object store) is left open — router only.
        for l in doc["static_resources"]["listeners"].as_sequence().unwrap() {
            if l["address"]["socket_address"]["port_value"].as_u64() != Some(10000) {
                let filters = http_filter_names(l);
                assert_eq!(
                    filters,
                    vec!["envoy.filters.http.router".to_string()],
                    "dedicated listener is not gated: {filters:?}"
                );
            }
        }

        // The ext_authz filter targets the authelia cluster at its documented path.
        let ext_authz = &shared["filter_chains"][0]["filters"][0]["typed_config"]["http_filters"]
            [0]["typed_config"];
        let http_service = &ext_authz["http_service"];
        assert_eq!(
            http_service["server_uri"]["cluster"].as_str(),
            Some("authelia")
        );
        assert_eq!(
            http_service["server_uri"]["uri"].as_str(),
            Some("authelia:9091")
        );
        assert_eq!(
            http_service["path_prefix"].as_str(),
            Some("/authelia/api/authz/ext-authz/")
        );

        // The authelia cluster exists, targeting the service on 9091.
        let clusters = doc["static_resources"]["clusters"].as_sequence().unwrap();
        let authelia = clusters
            .iter()
            .find(|c| c["name"].as_str() == Some("authelia"))
            .expect("authelia cluster present");
        let sock = &authelia["load_assignment"]["endpoints"][0]["lb_endpoints"][0]["endpoint"]["address"]
            ["socket_address"];
        assert_eq!(sock["address"].as_str(), Some("authelia"));
        assert_eq!(sock["port_value"].as_u64(), Some(9091));

        // The login portal is routed through the gateway at /authelia with ext_authz DISABLED
        // on that route, so a logged-out browser's deny-redirect can reach the login page.
        let vh = &shared["filter_chains"][0]["filters"][0]["typed_config"]["route_config"]["virtual_hosts"]
            [0];
        let portal = vh["routes"]
            .as_sequence()
            .unwrap()
            .iter()
            .find(|r| r["match"]["prefix"].as_str() == Some("/authelia"))
            .expect("portal route present");
        assert_eq!(portal["route"]["cluster"].as_str(), Some("authelia"));
        assert_eq!(
            portal["typed_per_filter_config"]["envoy.filters.http.ext_authz"]["disabled"].as_bool(),
            Some(true),
            "ext_authz is disabled on the portal route"
        );
    }

    #[test]
    fn auth_on_strips_client_identity_headers_and_allows_them_from_authelia() {
        let p = baseline_catalog()
            .plan(&auth_on_selection(), &PlanCtx::default())
            .unwrap();
        let arts = render_all(&p);
        let doc: Value = serde_yaml::from_str(&arts.envoy).expect("valid Envoy YAML");
        let shared = shared_listener(&doc);

        // Ingress: the shared virtual host strips client-supplied identity headers (anti-spoof).
        let vh = &shared["filter_chains"][0]["filters"][0]["typed_config"]["route_config"]["virtual_hosts"]
            [0];
        let stripped: BTreeSet<String> = vh["request_headers_to_remove"]
            .as_sequence()
            .expect("identity headers are stripped on ingress")
            .iter()
            .map(|h| h.as_str().unwrap().to_string())
            .collect();
        for h in [
            "Remote-User",
            "Remote-Email",
            "Remote-Name",
            "Remote-Groups",
        ] {
            assert!(
                stripped.contains(h),
                "{h} is stripped on ingress: {stripped:?}"
            );
        }

        // Response: the same headers are allowed upstream (now sourced from Authelia). The
        // request/response policy lives under `http_service` on the ext_authz typed config.
        let http_service = &shared["filter_chains"][0]["filters"][0]["typed_config"]["http_filters"]
            [0]["typed_config"]["http_service"];
        let allowed: BTreeSet<String> =
            http_service["authorization_response"]["allowed_upstream_headers"]["patterns"]
                .as_sequence()
                .unwrap()
                .iter()
                .map(|p| p["exact"].as_str().unwrap().to_string())
                .collect();
        for h in [
            "Remote-User",
            "Remote-Email",
            "Remote-Name",
            "Remote-Groups",
        ] {
            assert!(allowed.contains(h), "{h} is allowed upstream: {allowed:?}");
        }

        // Both session (cookie) and API (authorization) auth are forwarded to Authelia.
        let req_allowed: BTreeSet<String> =
            http_service["authorization_request"]["allowed_headers"]["patterns"]
                .as_sequence()
                .unwrap()
                .iter()
                .map(|p| p["exact"].as_str().unwrap().to_string())
                .collect();
        assert!(req_allowed.contains("cookie"), "session auth forwarded");
        assert!(
            req_allowed.contains("authorization"),
            "header (API) auth forwarded"
        );
    }

    #[test]
    fn auth_on_pulls_in_the_authelia_module() {
        let p = baseline_catalog()
            .plan(&auth_on_selection(), &PlanCtx::default())
            .unwrap();

        // The knob pulls authelia into the graph: it renders a fragment and its two config files.
        let (_, out) = p
            .renders
            .iter()
            .find(|(id, _)| id == &ModuleId::from("authelia"))
            .expect("authelia is pulled into the render set by the knob");
        let _: Value =
            serde_yaml::from_str(&out.fragment).expect("authelia fragment is valid YAML");
        for alias in ["authelia_config", "authelia_users"] {
            assert!(
                out.files.iter().any(|f| f.alias.as_deref() == Some(alias)),
                "authelia mounts {alias}"
            );
        }

        // The top-level compose includes the authelia fragment.
        let arts = render_all(&p);
        assert!(
            arts.compose.contains("./modules/authelia/compose.yaml"),
            "compose includes the authelia fragment:\n{}",
            arts.compose
        );

        // The gateway waits for authelia to be healthy before serving.
        let (_, envoy_frag) = p
            .renders
            .iter()
            .find(|(id, _)| id == &ModuleId::from("envoy"))
            .unwrap();
        let envoy_doc: Value = serde_yaml::from_str(&envoy_frag.fragment).unwrap();
        assert_eq!(
            envoy_doc["services"]["envoy"]["depends_on"]["authelia"]["condition"].as_str(),
            Some("service_healthy"),
            "envoy gates on authelia health when auth is on:\n{}",
            envoy_frag.fragment
        );
    }

    #[test]
    fn auth_off_by_default_emits_no_filter_no_cluster_no_module() {
        // Same selection, knob left at its default (off).
        let sel = Selection::modules(["envoy", "seaweedfs", "postgres", "mlflow", "headwaters"]);
        let p = baseline_catalog().plan(&sel, &PlanCtx::default()).unwrap();
        let arts = render_all(&p);
        let doc: Value = serde_yaml::from_str(&arts.envoy).expect("valid Envoy YAML");

        // No listener is gated, and no identity headers are stripped anywhere.
        for l in doc["static_resources"]["listeners"].as_sequence().unwrap() {
            let filters = http_filter_names(l);
            assert_eq!(
                filters,
                vec!["envoy.filters.http.router".to_string()],
                "no ext_authz when auth is off: {filters:?}"
            );
            let vh = &l["filter_chains"][0]["filters"][0]["typed_config"]["route_config"]["virtual_hosts"]
                [0];
            assert!(
                vh["request_headers_to_remove"].is_null(),
                "no header strip when auth is off"
            );
        }

        // No authelia cluster, and the module is absent from the graph entirely.
        let clusters = doc["static_resources"]["clusters"].as_sequence().unwrap();
        assert!(
            !clusters
                .iter()
                .any(|c| c["name"].as_str() == Some("authelia")),
            "no authelia cluster when auth is off"
        );
        assert!(
            !p.renders
                .iter()
                .any(|(id, _)| id == &ModuleId::from("authelia")),
            "authelia is not pulled in when auth is off"
        );
        assert!(
            !arts.compose.contains("authelia"),
            "compose has no authelia reference when auth is off:\n{}",
            arts.compose
        );
    }
}
