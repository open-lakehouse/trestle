//! The `rustfs` module's client environment and host-side virtual-hosted names.

use std::collections::BTreeMap;

use olai_stack_topology::{Plan, PlanCtx, Selection, Vantage, baseline_catalog};

fn plan(knobs: &[(&str, &str)], ctx: &PlanCtx) -> Plan {
    let mut selection = Selection::modules(["rustfs"]);
    selection.knob_overrides.insert(
        "rustfs".into(),
        knobs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<BTreeMap<_, _>>(),
    );
    selection
        .extra_resources
        .push(olai_stack_topology::ExtraResource {
            resource: "object_store".into(),
            name: "lake".into(),
            provider: None,
        });
    baseline_catalog().plan(&selection, ctx).unwrap()
}

fn server_domains(plan: &Plan) -> String {
    let out = plan.materialize().unwrap();
    let fragment = &out
        .files
        .iter()
        .find(|f| f.path == "modules/rustfs/compose.yaml")
        .unwrap()
        .contents;
    let yaml: serde_yaml::Value = serde_yaml::from_str(fragment).unwrap();
    yaml["services"]["rustfs"]["environment"]["RUSTFS_SERVER_DOMAINS"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn generic_endpoint_env_is_opt_in() {
    let off = plan(&[], &PlanCtx::default());
    let env = off.client_env(Vantage::Host);
    assert!(env.get("AWS_ENDPOINT_URL_S3").is_some());
    assert_eq!(env.get("AWS_ENDPOINT_URL"), None);
    assert_eq!(env.get("AWS_ALLOW_HTTP"), None);

    let on = plan(&[("generic_endpoint_env", "true")], &PlanCtx::default());
    let env = on.client_env(Vantage::Host);
    assert_eq!(env.get("AWS_ENDPOINT_URL"), env.get("AWS_ENDPOINT_URL_S3"));
    assert_eq!(env.get("AWS_ALLOW_HTTP"), Some("true"));
}

#[test]
fn the_dedicated_s3_listener_is_a_server_domain() {
    // Auto-allocated from the listener base by default…
    assert_eq!(
        server_domains(&plan(&[], &PlanCtx::default())),
        "s3.us-east-1.amazonaws.com,s3.amazonaws.com,localhost:9100"
    );
    // …or the recipe's explicit port, which the gateway publishes on both sides.
    let ctx = PlanCtx {
        dedicated_listener_ports: vec![9000],
        ..PlanCtx::default()
    };
    let plan = plan(&[], &ctx);
    assert_eq!(
        server_domains(&plan),
        "s3.us-east-1.amazonaws.com,s3.amazonaws.com,localhost:9000"
    );
    assert_eq!(
        plan.client_env(Vantage::Host).get("AWS_ENDPOINT_URL_S3"),
        Some("http://localhost:9000")
    );
}

#[test]
fn emulated_s3_hosts_name_the_bucket_region() {
    let plan = plan(&[], &PlanCtx::default());
    let tls = plan
        .gateway
        .tls
        .as_ref()
        .expect("RustFS emulates the AWS hostnames");
    let s3 = tls
        .virtual_hosts
        .iter()
        .find(|v| v.cluster == "rustfs")
        .unwrap();
    assert_eq!(s3.response_headers["x-amz-bucket-region"], "us-east-1");
    let sts = tls
        .virtual_hosts
        .iter()
        .find(|v| v.cluster == "sts-shim")
        .unwrap();
    assert!(sts.response_headers.is_empty());

    let envoy: serde_yaml::Value =
        serde_yaml::from_str(&olai_stack_topology::render_all(&plan).unwrap().envoy).unwrap();
    let listener = envoy["static_resources"]["listeners"]
        .as_sequence()
        .unwrap()
        .iter()
        .find(|l| l["name"] == "emulated_tls")
        .unwrap();
    let vhost = &listener["filter_chains"][0]["filters"][0]["typed_config"]["route_config"]
        ["virtual_hosts"]
        .as_sequence()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "rustfs")
        .unwrap()
        .clone();
    assert_eq!(
        vhost["response_headers_to_add"][0]["header"]["key"],
        "x-amz-bucket-region"
    );
}
