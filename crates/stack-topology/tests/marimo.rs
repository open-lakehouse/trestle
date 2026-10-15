//! The `marimo` notebooks module: it serves the Python client workspace from the stack, so
//! selecting it generates the workspace, routes the editor under its base path, and runs the
//! notebooks against the in-network client environment.

use olai_stack_topology::{ModuleId, Plan, PlanCtx, Selection, baseline_catalog};

fn plan(modules: &[&str]) -> Plan {
    baseline_catalog()
        .plan(
            &Selection::modules(modules.iter().copied()),
            &PlanCtx::default(),
        )
        .expect("plan should succeed")
}

fn fragment(p: &Plan, id: &str) -> String {
    olai_stack_topology::render_all(p)
        .unwrap()
        .modules
        .into_iter()
        .find(|(module, _)| module.as_str() == id)
        .map(|(_, out)| out.fragment.clone())
        .unwrap_or_else(|| panic!("{id} renders"))
}

#[test]
fn selecting_marimo_generates_the_client_workspace() {
    let p = plan(&["mlflow", "marimo"]);
    let workspace = p
        .python_client
        .as_ref()
        .expect("the workspace is generated without `[selection.client]`");
    // The selected modules' packages are installed; examples stay the user's choice.
    assert!(
        workspace
            .dependencies
            .iter()
            .any(|d| d.starts_with("mlflow"))
    );
    assert!(workspace.notebooks.is_empty());

    let out = p.materialize().unwrap();
    assert!(out.files.iter().any(|f| f.path == "client/pyproject.toml"));
}

#[test]
fn the_editor_is_fronted_at_its_base_path_streaming_safe() {
    let p = plan(&["marimo"]);
    assert!(p.services.contains_key(&ModuleId::from("envoy")));
    let route = p.gateway.listeners[0]
        .routes
        .iter()
        .find(|r| r.prefix == "/marimo")
        .expect("a /marimo route");
    assert_eq!(route.cluster, "marimo");
    assert_eq!(route.rewrite, None);
    assert!(route.options.streaming);

    let host = p.client_env_host.get("MARIMO_URL").unwrap();
    assert!(host.ends_with("/marimo"), "{host}");
}

#[test]
fn notebooks_run_against_the_in_network_env_in_their_own_venv() {
    let f = fragment(&plan(&["marimo"]), "marimo");
    assert!(f.contains("--base-url /marimo"), "{f}");
    assert!(f.contains("./client:/workspace"), "{f}");
    assert!(
        f.contains("./client/container.env:/workspace/host.env:ro"),
        "{f}"
    );
    assert!(f.contains("UV_PROJECT_ENVIRONMENT: /venv"), "{f}");
    assert!(f.contains("127.0.0.1:2718/marimo/health"), "{f}");
}

#[test]
fn packages_install_from_pypi_unless_an_index_is_set() {
    let f = fragment(&plan(&["marimo"]), "marimo");
    assert!(!f.contains("UV_DEFAULT_INDEX"), "{f}");

    let mut selection = Selection::modules(["marimo"]);
    selection.knob_overrides.insert(
        ModuleId::from("marimo"),
        [("index_url".into(), "https://mirror.example/simple/".into())].into(),
    );
    let p = baseline_catalog()
        .plan(&selection, &PlanCtx::default())
        .unwrap();
    let f = fragment(&p, "marimo");
    assert!(
        f.contains("UV_DEFAULT_INDEX: https://mirror.example/simple/"),
        "{f}"
    );
}
