//! Platform features (identity, policy, observability) and the integrations they turn on.

use olai_stack_topology::{
    GatewaySettings, IntegrationState, ModuleId, Plan, PlanCtx, PlanError, PlatformFeature,
    Selection, baseline_catalog, render_all,
};

fn plan(selection: &Selection) -> Result<Plan, PlanError> {
    baseline_catalog().plan(selection, &PlanCtx::default())
}

/// Every platform feature on, over a small lakehouse.
fn all_features(modules: &[&str]) -> Selection {
    Selection::modules(modules.iter().copied())
        .with_feature(PlatformFeature::Identity)
        .with_feature(PlatformFeature::Policy)
        .with_feature(PlatformFeature::Observability)
}

fn state(plan: &Plan, module: &str, feature: PlatformFeature) -> IntegrationState {
    plan.integrations
        .iter()
        .find(|i| i.module.as_str() == module && i.feature == feature)
        .unwrap_or_else(|| panic!("no {feature} integration for {module}"))
        .state
        .clone()
}

fn opa_config(plan: &Plan) -> String {
    let (_, out) = render_all(plan)
        .unwrap()
        .modules
        .into_iter()
        .find(|(id, _)| id.as_str() == "opa")
        .expect("opa renders");
    out.files
        .into_iter()
        .find(|f| f.path.ends_with("config.yaml"))
        .expect("opa renders its config")
        .contents
}

/// The shared listener's HTTP connection manager in the rendered Envoy bootstrap.
fn shared_manager(plan: &Plan) -> serde_yaml::Value {
    let doc: serde_yaml::Value = serde_yaml::from_str(&render_all(plan).unwrap().envoy).unwrap();
    doc["static_resources"]["listeners"][0]["filter_chains"][0]["filters"][0]["typed_config"]
        .clone()
}

fn opt_out(mut selection: Selection, module: &str, feature: PlatformFeature) -> Selection {
    selection
        .integrations
        .entry(ModuleId::from(module))
        .or_default()
        .insert(feature, false);
    selection
}

#[test]
fn every_capable_service_integrates_with_a_selected_feature_by_default() {
    let p = plan(&all_features(&["postgres", "mlflow", "omnigent"])).unwrap();
    for (module, feature) in [
        ("envoy", PlatformFeature::Identity),
        ("envoy", PlatformFeature::Policy),
        ("envoy", PlatformFeature::Observability),
        ("opa", PlatformFeature::Observability),
        ("omnigent", PlatformFeature::Identity),
    ] {
        assert!(
            matches!(state(&p, module, feature), IntegrationState::Active { .. }),
            "{module} {feature}: {:?}",
            state(&p, module, feature)
        );
    }
    assert_eq!(
        state(&p, "opa", PlatformFeature::Observability),
        IntegrationState::Active {
            protocol: "otlp-grpc".into(),
            endpoint: Some("http://jaeger:4317".into()),
        }
    );
    // The providers join the environment without being selected.
    for provider in ["authelia", "opa", "jaeger"] {
        assert!(
            p.graph.module(&ModuleId::from(provider)).is_some(),
            "{provider}"
        );
    }
}

#[test]
fn the_gateway_and_the_policy_engine_export_their_traces() {
    let p = plan(&all_features(&["postgres", "mlflow"])).unwrap();

    let tracing = p.gateway.tracing.as_ref().expect("the gateway traces");
    let cluster = p
        .gateway
        .clusters
        .iter()
        .find(|c| c.name == tracing.cluster)
        .expect("the tracing cluster exists");
    assert_eq!((cluster.host.as_str(), cluster.port), ("jaeger", 4317));
    assert!(cluster.http2, "OTLP/gRPC needs an HTTP/2 upstream");
    let manager = shared_manager(&p);
    let provider = &manager["tracing"]["provider"];
    assert_eq!(
        provider["name"].as_str(),
        Some("envoy.tracers.opentelemetry")
    );
    assert_eq!(
        provider["typed_config"]["grpc_service"]["envoy_grpc"]["cluster_name"].as_str(),
        Some(tracing.cluster.as_str())
    );

    let config: serde_yaml::Value = serde_yaml::from_str(&opa_config(&p)).unwrap();
    assert_eq!(
        config["distributed_tracing"]["address"].as_str(),
        Some("jaeger:4317")
    );
    assert_eq!(config["distributed_tracing"]["type"].as_str(), Some("grpc"));
}

#[test]
fn opting_a_service_out_turns_only_its_integration_off() {
    let selection = opt_out(
        all_features(&["postgres", "mlflow"]),
        "opa",
        PlatformFeature::Observability,
    );
    let p = plan(&selection).unwrap();
    assert_eq!(
        state(&p, "opa", PlatformFeature::Observability),
        IntegrationState::Disabled
    );
    assert!(!opa_config(&p).contains("distributed_tracing"));
    // The gateway still traces.
    assert!(p.gateway.tracing.is_some());

    let p = plan(&opt_out(
        all_features(&["postgres", "mlflow"]),
        "envoy",
        PlatformFeature::Observability,
    ))
    .unwrap();
    assert!(p.gateway.tracing.is_none());
    assert!(shared_manager(&p)["tracing"].is_null());
}

#[test]
fn without_observability_nothing_traces() {
    let selection = Selection::modules(["postgres", "mlflow"])
        .with_feature(PlatformFeature::Identity)
        .with_feature(PlatformFeature::Policy);
    let p = plan(&selection).unwrap();
    assert!(p.gateway.tracing.is_none());
    assert!(!opa_config(&p).contains("distributed_tracing"));
    assert!(p.graph.module(&ModuleId::from("jaeger")).is_none());
    assert!(
        p.integrations
            .iter()
            .all(|i| i.feature != PlatformFeature::Observability)
    );
}

#[test]
fn a_service_speaking_another_policy_protocol_is_incompatible() {
    let p = plan(&all_features(&["postgres", "unity-catalog-rs"])).unwrap();
    let IntegrationState::Incompatible { reason } =
        state(&p, "unity-catalog-rs", PlatformFeature::Policy)
    else {
        panic!("OPA doesn't speak olai-auth's authorize API");
    };
    assert!(
        reason.contains("`opa` doesn't serve olai-authorize"),
        "{reason}"
    );
    // Its configuration doesn't change.
    assert!(matches!(
        state(&p, "envoy", PlatformFeature::Policy),
        IntegrationState::Active { .. }
    ));
}

#[test]
fn turning_the_gateways_sign_in_off_strands_what_depends_on_it() {
    let p = plan(&opt_out(
        all_features(&["postgres", "mlflow", "omnigent"]),
        "envoy",
        PlatformFeature::Identity,
    ))
    .unwrap();
    assert!(p.gateway.auth.is_none() && p.gateway.policy.is_none());
    assert_eq!(
        state(&p, "envoy", PlatformFeature::Policy),
        IntegrationState::Incompatible {
            reason: "it needs its identity integration on too".into()
        }
    );
    assert!(matches!(
        state(&p, "omnigent", PlatformFeature::Identity),
        IntegrationState::Incompatible { .. }
    ));
    assert!(!p.stack_facts.gateway_auth);
}

#[test]
fn opting_out_of_what_a_module_cant_do_is_rejected() {
    let selection = opt_out(
        all_features(&["postgres", "mlflow"]),
        "mlflow",
        PlatformFeature::Observability,
    );
    assert!(matches!(
        plan(&selection).unwrap_err(),
        PlanError::UnknownIntegration { .. }
    ));
    let selection = opt_out(
        all_features(&["postgres", "mlflow"]),
        "omnigent",
        PlatformFeature::Identity,
    );
    assert!(matches!(
        plan(&selection).unwrap_err(),
        PlanError::UnknownIntegration { .. }
    ));
}

#[test]
fn a_feature_naming_a_module_that_doesnt_provide_it_is_rejected() {
    let mut selection = Selection::modules(["postgres", "mlflow"]);
    selection.platform.observability = Some(olai_stack_topology::FeatureChoice::Provider(
        ModuleId::from("mlflow"),
    ));
    assert!(matches!(
        plan(&selection).unwrap_err(),
        PlanError::UnknownPlatformProvider {
            feature: PlatformFeature::Observability,
            ..
        }
    ));
}

#[test]
fn observability_needs_no_gateway_but_identity_does() {
    let direct = |selection: Selection| Selection {
        gateway: GatewaySettings::DIRECT,
        ..selection
    };
    let p = plan(&direct(
        Selection::modules(["postgres"]).with_feature(PlatformFeature::Observability),
    ))
    .unwrap();
    assert!(!p.has_gateway());
    assert!(p.graph.module(&ModuleId::from("jaeger")).is_some());
    assert!(
        p.graph.module(&ModuleId::from("envoy")).is_none(),
        "the tracing backend doesn't pull the gateway back in"
    );

    let err = plan(&direct(
        Selection::modules(["postgres", "mlflow"]).with_feature(PlatformFeature::Identity),
    ))
    .unwrap_err();
    assert!(matches!(err, PlanError::GatewayRequired { .. }), "{err:?}");
}

#[test]
fn integrations_reach_the_plan_result_and_the_catalog() {
    let selection = all_features(&["postgres", "mlflow"]);
    let result = olai_stack_topology::projection::plan_result(&selection, None).unwrap();
    let json = serde_json::to_value(&result).unwrap();
    let integrations = json["integrations"].as_array().unwrap();
    assert!(integrations.iter().any(|i| i["module"] == "opa"
        && i["feature"] == "observability"
        && i["state"] == "active"
        && i["protocol"] == "otlp-grpc"));

    let catalog = serde_json::to_value(olai_stack_topology::projection::catalog_dto()).unwrap();
    let module = |id: &str| {
        catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == id)
            .unwrap()
            .clone()
    };
    assert_eq!(module("jaeger")["protocols"][0]["protocol"], "otlp-grpc");
    assert_eq!(module("envoy")["integrations"][1]["feature"], "policy");
    assert_eq!(
        module("envoy")["integrations"][1]["requires"],
        serde_json::json!(["identity"])
    );
}
