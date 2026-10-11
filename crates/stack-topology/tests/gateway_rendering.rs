//! Gateway-side rendering details that depend on plan context: the auth portal's public URL
//! and the Envoy rewrite patterns.

use std::collections::BTreeMap;

use olai_stack_topology::{ModuleId, PlanCtx, Selection, baseline_catalog, render_all};

fn authenticated(port: u16) -> olai_stack_topology::Plan {
    let sel = Selection {
        modules: vec![
            "envoy".into(),
            "postgres".into(),
            "mlflow".into(),
            "rustfs".into(),
        ],
        knob_overrides: BTreeMap::from([(
            ModuleId::from("envoy"),
            BTreeMap::from([("auth".to_string(), "true".to_string())]),
        )]),
        ..Default::default()
    };
    baseline_catalog()
        .plan(
            &sel,
            &PlanCtx {
                gateway_host_port: port,
                ..PlanCtx::default()
            },
        )
        .expect("plan should succeed")
}

#[test]
fn auth_portal_url_follows_the_gateway_port() {
    let p = authenticated(9090);
    let (_, out) = p
        .renders
        .iter()
        .find(|(id, _)| id.as_str() == "authelia")
        .expect("auth pulls in authelia");
    let config = out
        .files
        .iter()
        .find(|f| f.path.ends_with("configuration.yml"))
        .expect("authelia renders its configuration");
    assert!(
        config
            .contents
            .contains("authelia_url: http://localhost:9090/authelia"),
        "{}",
        config.contents
    );
}

#[test]
fn rewrite_patterns_match_prefixes_literally() {
    let envoy = render_all(&authenticated(9080)).envoy;
    // `/api/2.0/mlflow` is rewritten under MLflow's base path; its `.` must not be a wildcard.
    assert!(
        envoy.contains(r#"regex: "^/api/2\\x{2E}0/mlflow(.*)$""#),
        "{envoy}"
    );
    assert!(!envoy.contains("regex: \"^/api/2.0/mlflow"));
}

fn with_opa(knobs: BTreeMap<ModuleId, BTreeMap<String, String>>) -> Selection {
    Selection {
        modules: vec![
            "envoy".into(),
            "postgres".into(),
            "mlflow".into(),
            "opa".into(),
        ],
        knob_overrides: knobs,
        ..Default::default()
    }
}

#[test]
fn selecting_opa_checks_every_request_after_forward_auth() {
    let p = baseline_catalog()
        .plan(&with_opa(BTreeMap::new()), &PlanCtx::default())
        .expect("plan should succeed");
    // Selecting the policy engine turns gateway auth on: OPA decides on Authelia's identity.
    assert!(p.gateway.auth.is_some());
    let policy = p.gateway.policy.as_ref().expect("policy is wired");
    let cluster = p
        .gateway
        .clusters
        .iter()
        .find(|c| c.name == policy.cluster)
        .expect("the policy cluster exists");
    assert_eq!((cluster.host.as_str(), cluster.port), ("opa", 9191));
    assert!(cluster.http2, "ext_authz gRPC needs an HTTP/2 upstream");

    let doc: serde_yaml::Value = serde_yaml::from_str(&render_all(&p).envoy).unwrap();
    let shared =
        &doc["static_resources"]["listeners"][0]["filter_chains"][0]["filters"][0]["typed_config"];
    let filters: Vec<&str> = shared["http_filters"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        filters,
        [
            "envoy.filters.http.header_mutation",
            "envoy.filters.http.ext_authz",
            "envoy.filters.http.ext_authz.policy",
            "envoy.filters.http.router",
        ]
    );
    let check = &shared["http_filters"][2]["typed_config"];
    assert_eq!(
        check["grpc_service"]["envoy_grpc"]["cluster_name"].as_str(),
        Some(policy.cluster.as_str())
    );
    assert_eq!(check["failure_mode_allow"].as_bool(), Some(false));

    // The login portal must stay reachable: both checks are disabled on it.
    let portal = &shared["route_config"]["virtual_hosts"][0]["routes"][0];
    assert_eq!(portal["match"]["prefix"].as_str(), Some("/authelia"));
    for filter in [
        "envoy.filters.http.ext_authz",
        "envoy.filters.http.ext_authz.policy",
    ] {
        assert_eq!(
            portal["typed_per_filter_config"][filter]["disabled"].as_bool(),
            Some(true),
            "{filter} is disabled on the portal route"
        );
    }

    // The policy is operator-owned: beside compose.yaml, written once.
    let out = p.materialize();
    for path in ["policies/authz.rego", "policies/authz_test.rego"] {
        let f = out
            .files
            .iter()
            .find(|f| f.path == path)
            .unwrap_or_else(|| panic!("{path} not materialized"));
        assert!(f.preserve, "{path} must survive re-renders");
    }
}

#[test]
fn opa_with_auth_explicitly_off_is_rejected() {
    let knobs = BTreeMap::from([(
        ModuleId::from("envoy"),
        BTreeMap::from([("auth".to_string(), "false".to_string())]),
    )]);
    let err = baseline_catalog()
        .plan(&with_opa(knobs), &PlanCtx::default())
        .unwrap_err();
    assert!(
        matches!(
            err,
            olai_stack_topology::PlanError::PolicyRequiresAuth { .. }
        ),
        "{err}"
    );
}

#[test]
fn opa_dry_run_knob_reaches_the_plugin_config() {
    let knobs = BTreeMap::from([(
        ModuleId::from("opa"),
        BTreeMap::from([("dry_run".to_string(), "true".to_string())]),
    )]);
    let p = baseline_catalog()
        .plan(&with_opa(knobs), &PlanCtx::default())
        .unwrap();
    let (_, out) = p
        .renders
        .iter()
        .find(|(id, _)| id.as_str() == "opa")
        .unwrap();
    let config = out
        .files
        .iter()
        .find(|f| f.path.ends_with("config.yaml"))
        .unwrap();
    assert!(
        config.contents.contains("dry-run: true"),
        "{}",
        config.contents
    );
}

/// A one-service module whose fragment echoes the planner's stack-wide render variables, with
/// one plain and one streaming, credential-stripping `Api` endpoint.
fn probe_module() -> std::sync::Arc<dyn olai_stack_topology::Module> {
    use olai_stack_topology::{
        DataModule, Endpoint, Placement, Provides, RenderSpec, Rewrite, Role, RouteIntent,
        RouteOptions, Scheme, ServiceSpec,
    };
    let api = |id: &str, prefix: &str, route: RouteOptions| Endpoint {
        id: id.into(),
        scheme: Scheme::Http,
        internal_port: 8080,
        host_port: None,
        intent: RouteIntent::Api,
        path: String::new(),
        mount_prefix: Some(prefix.into()),
        rewrite: Rewrite::Inherit,
        route,
    };
    let mut provides = Provides::default();
    provides.client_env.insert(
        "PROBE_URL".into(),
        olai_stack_topology::ClientValue::endpoint(
            "probe",
            "plain",
            olai_stack_topology::UrlPart::Full,
        ),
    );
    std::sync::Arc::new(DataModule {
        id: ModuleId::from("probe"),
        display_name: None,
        summary: None,
        category: None,
        provider_of: None,
        requires: vec![ModuleId::from("envoy")],
        conflicts_with: vec![],
        needs: vec![],
        service_specs: vec![ServiceSpec {
            name: "probe".into(),
            role: Role::new("probe"),
            placement: Placement::Container {
                service: "probe".into(),
            },
            base_path: String::new(),
            endpoints: vec![
                api("plain", "/plain", RouteOptions::default()),
                api(
                    "stream",
                    "/stream",
                    RouteOptions {
                        streaming: true,
                        strip_client_credentials: true,
                    },
                ),
            ],
            depends_on: vec![],
        }],
        provides,
        knobs: vec![],
        render: RenderSpec {
            fragment: "auth={{ env.GATEWAY_AUTH }} internal={{ env.GATEWAY_INTERNAL_URL }} \
                       keys={{ env.CLIENT_ENV_KEYS }}"
                .into(),
            files: vec![],
        },
    })
}

fn plan_with_probe(auth: bool) -> olai_stack_topology::Plan {
    let catalog = olai_stack_topology::Catalog::from_modules(
        baseline_catalog()
            .modules()
            .iter()
            .cloned()
            .chain([probe_module()]),
    );
    let mut sel = Selection::modules(["envoy", "probe"]);
    if auth {
        sel.knob_overrides = BTreeMap::from([(
            ModuleId::from("envoy"),
            BTreeMap::from([("auth".to_string(), "true".to_string())]),
        )]);
    }
    catalog
        .plan(&sel, &PlanCtx::default())
        .expect("plan should succeed")
}

/// The Envoy route entry for `prefix`, up to the next route.
fn route_block<'a>(envoy: &'a str, prefix: &str) -> &'a str {
    let start = envoy
        .find(&format!("- match: {{ prefix: \"{prefix}\" }}"))
        .unwrap_or_else(|| panic!("no route for {prefix}:\n{envoy}"));
    let rest = &envoy[start + 1..];
    let end = rest
        .find("- match:")
        .or_else(|| rest.find("http_filters:"))
        .unwrap_or(rest.len());
    &envoy[start..start + 1 + end]
}

#[test]
fn route_options_reach_the_gateway_config() {
    let envoy = render_all(&plan_with_probe(false)).envoy;

    let stream = route_block(&envoy, "/stream");
    assert!(stream.contains("timeout: 0s"), "{stream}");
    assert!(
        stream.contains(r#"request_headers_to_remove: ["authorization", "x-api-key"]"#),
        "{stream}"
    );

    // An endpoint that asks for neither keeps Envoy's defaults.
    let plain = route_block(&envoy, "/plain");
    assert!(!plain.contains("timeout:"), "{plain}");
    assert!(!plain.contains("request_headers_to_remove"), "{plain}");
}

#[test]
fn fragments_see_the_stack_wide_render_variables() {
    let fragment = |p: &olai_stack_topology::Plan| {
        p.renders
            .iter()
            .find(|(id, _)| id.as_str() == "probe")
            .map(|(_, out)| out.fragment.clone())
            .expect("probe renders")
    };

    let open = fragment(&plan_with_probe(false));
    assert!(open.contains("auth=false"), "{open}");
    assert!(open.contains("internal=http://envoy:10000"), "{open}");
    assert!(open.contains("keys=PROBE_URL"), "{open}");

    let gated = fragment(&plan_with_probe(true));
    assert!(gated.contains("auth=true"), "{gated}");
}
