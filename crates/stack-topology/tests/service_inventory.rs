//! A module's [`ServiceSpec`](olai_stack_topology::ServiceSpec)s plus its declared
//! [`helpers`](olai_stack_topology::Provides::helpers) name every container its compose fragment
//! renders, so an export can list them from the plan without reading generated YAML.

use std::collections::{BTreeMap, BTreeSet};

use olai_stack_topology::{PlanCtx, Selection, baseline_catalog};

fn selections() -> Vec<Selection> {
    let everything = Selection::modules([
        "envoy",
        "authelia",
        "opa",
        "postgres",
        "rustfs",
        "mlflow",
        "unity-catalog",
        "jaeger",
        "headwaters",
        "omnigent",
        "marimo",
        "puppygraph",
        "rivers",
    ]);
    let azure = Selection::modules(["envoy", "postgres", "azurite", "unity-catalog", "mlflow"]);
    let bare_store = Selection::modules(["envoy", "rustfs"]);
    let mut mounts = Selection::modules(["unity-catalog"]);
    mounts.knob_overrides.insert(
        "unity-catalog".into(),
        [
            ("extra_mounts".to_string(), "./data:/data".to_string()),
            ("writable_mounts".to_string(), "/data".to_string()),
        ]
        .into(),
    );
    vec![everything, azure, bare_store, mounts]
}

fn rendered_services(fragment: &str) -> BTreeSet<String> {
    let yaml: serde_yaml::Value = serde_yaml::from_str(fragment).expect("fragment is YAML");
    yaml["services"]
        .as_mapping()
        .into_iter()
        .flat_map(|m| m.keys())
        .map(|k| k.as_str().expect("service names are strings").to_string())
        .collect()
}

#[test]
fn every_rendered_container_is_a_service_or_a_declared_helper() {
    let catalog = baseline_catalog();
    let mut helpers_seen: BTreeSet<String> = BTreeSet::new();
    for selection in selections() {
        let plan = catalog.plan(&selection, &PlanCtx::default()).unwrap();
        let out = plan.materialize().unwrap();
        for module in &plan.graph.nodes {
            let id = module.id();
            let path = format!("modules/{id}/compose.yaml");
            let Some(file) = out.files.iter().find(|f| f.path == path) else {
                continue;
            };
            let rendered = rendered_services(&file.contents);
            let services: BTreeSet<String> = plan
                .services
                .get(id)
                .into_iter()
                .flatten()
                .map(|s| s.name.clone())
                .collect();
            let helpers: BTreeSet<String> = module
                .provides()
                .helpers
                .iter()
                .map(|h| h.name.clone())
                .collect();
            let undeclared: Vec<_> = rendered
                .iter()
                .filter(|n| !services.contains(*n) && !helpers.contains(*n))
                .collect();
            assert!(
                undeclared.is_empty(),
                "{id} renders undeclared {undeclared:?}"
            );
            assert!(
                services.is_subset(&rendered),
                "{id}: services {services:?} not all rendered in {rendered:?}"
            );
            helpers_seen.extend(rendered.intersection(&helpers).cloned());
        }
    }
    // A declaration no selection renders is stale (or the selections above need widening).
    let declared: BTreeMap<String, String> = catalog
        .modules()
        .iter()
        .flat_map(|m| {
            m.provides()
                .helpers
                .iter()
                .map(|h| (h.name.clone(), m.id().to_string()))
        })
        .collect();
    for (helper, module) in declared {
        assert!(
            helpers_seen.contains(&helper),
            "{module} declares helper `{helper}`, which no tested selection renders"
        );
    }
}
