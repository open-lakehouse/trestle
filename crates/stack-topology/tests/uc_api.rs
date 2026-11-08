//! Consumers of Unity Catalog find it by the API it serves, not by module id: any module that
//! serves the `unity-catalog` API fills in, and two of them are alternatives.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;

use olai_stack_topology::projection::{TopologyEdgeDto, graph_dto};
use olai_stack_topology::{
    Catalog, ClientValue, HelperService, Knob, KnobRejection, Module, ModuleId, PlanCtx, PlanError,
    Provides, RenderSpec, ResolvedKnobs, ResourceDemand, Selection, ServiceSpec, StackFacts,
    TargetId, Vantage, baseline_catalog,
};

/// The baseline Unity Catalog under another id: a stand-in for a second implementation.
struct OtherUc {
    id: ModuleId,
    inner: Arc<dyn Module>,
}

impl Module for OtherUc {
    fn id(&self) -> &ModuleId {
        &self.id
    }
    fn provider_of(&self) -> Option<&str> {
        self.inner.provider_of()
    }
    fn requires(&self) -> &[ModuleId] {
        self.inner.requires()
    }
    fn needs_in(&self, knobs: &ResolvedKnobs) -> Cow<'_, [ResourceDemand]> {
        self.inner.needs_in(knobs)
    }
    fn knobs(&self) -> &[Knob] {
        self.inner.knobs()
    }
    fn check_knobs(&self, knobs: &ResolvedKnobs) -> Result<(), KnobRejection> {
        self.inner.check_knobs(knobs)
    }
    fn provides(&self) -> &Provides {
        self.inner.provides()
    }
    fn render_for(&self, target: &TargetId) -> Option<&RenderSpec> {
        self.inner.render_for(target)
    }
    fn services(&self, knobs: &ResolvedKnobs) -> Vec<ServiceSpec> {
        self.inner.services(knobs)
    }
    fn client_env_in(
        &self,
        knobs: &ResolvedKnobs,
        stack: &StackFacts,
    ) -> Cow<'_, BTreeMap<String, ClientValue>> {
        self.inner.client_env_in(knobs, stack)
    }
    fn helpers_in(&self, knobs: &ResolvedKnobs) -> Cow<'_, [HelperService]> {
        self.inner.helpers_in(knobs)
    }
}

fn catalog() -> Catalog {
    let mut catalog = baseline_catalog();
    let inner = catalog
        .get(&ModuleId::from("unity-catalog-oss"))
        .expect("baseline UC")
        .clone();
    catalog.insert(Arc::new(OtherUc {
        id: ModuleId::from("other-uc"),
        inner,
    }));
    catalog
}

fn plan(modules: &[&str]) -> Result<olai_stack_topology::Plan, PlanError> {
    catalog().plan(
        &Selection::modules(modules.iter().copied()),
        &PlanCtx::default(),
    )
}

#[test]
fn unity_catalog_serves_the_unity_catalog_api() {
    let cat = baseline_catalog();
    let uc = cat.get(&ModuleId::from("unity-catalog-oss")).unwrap();
    assert_eq!(uc.provides().api.as_deref(), Some("unity-catalog"));
}

#[test]
fn two_servers_of_one_api_are_alternatives() {
    let err = plan(&["unity-catalog-oss", "other-uc"]).unwrap_err();
    assert_eq!(
        err,
        PlanError::ConflictingApiProviders {
            api: "unity-catalog".into(),
            providers: vec![
                ModuleId::from("other-uc"),
                ModuleId::from("unity-catalog-oss")
            ],
        }
    );
}

#[test]
fn mlflow_uses_whichever_module_serves_the_api() {
    let p = plan(&["mlflow", "other-uc"]).expect("plans");
    // The registry is the other server's: MLflow integrates with it, by its id.
    let graph = graph_dto(&p);
    assert!(
        graph
            .edges
            .iter()
            .any(|e| matches!(e, TopologyEdgeDto::Uses { from, to, .. } if from == "mlflow" && to == "other-uc")),
        "{:?}",
        graph.edges
    );
    // A host finds the API's server in the graph by its `api`.
    let server: Vec<&str> = graph
        .nodes
        .iter()
        .filter(|n| n.api.as_deref() == Some("unity-catalog"))
        .map(|n| n.id.as_str())
        .collect();
    assert_eq!(server, ["other-uc"]);
    let env = p.client_env(Vantage::Host);
    let uc_url = env.get("UNITY_CATALOG_URL").expect("UC address");
    let origin = uc_url
        .strip_suffix("/api/2.1/unity-catalog")
        .expect("UC at the gateway root");
    assert_eq!(
        env.get("MLFLOW_REGISTRY_URI"),
        Some(format!("uc:{origin}").as_str())
    );
}

#[test]
fn mlflow_renders_its_registry_against_whichever_module_serves_the_api() {
    let p = plan(&["mlflow", "other-uc"]).expect("plans");
    let out = olai_stack_topology::render_all(&p).expect("renders");
    let (_, mlflow) = out
        .modules
        .iter()
        .find(|(id, _)| id.as_str() == "mlflow")
        .expect("mlflow renders");
    let fragment: serde_yaml::Value = serde_yaml::from_str(&mlflow.fragment).unwrap();
    let svc = &fragment["services"]["mlflow"];
    let command: Vec<&str> = svc["command"]
        .as_sequence()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(command.contains(&"--registry-store-uri"), "{command:?}");
    assert!(
        svc["depends_on"].get("unitycatalog").is_some(),
        "{:?}",
        svc["depends_on"]
    );
    assert!(svc["environment"].get("PYTHONPATH").is_some(), "{svc:?}");
}
