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
