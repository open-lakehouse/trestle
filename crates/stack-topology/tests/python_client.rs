//! The Python client workspace (`Plan::python_client`, written under `client/`): opt-in, its
//! dependencies follow the selected modules, and example notebooks appear only when asked for
//! and only for selected modules.

use olai_stack_topology::{
    ClientSelection, MaterializedOutput, OutputFile, PlanCtx, Selection, baseline_catalog,
};

fn materialize(modules: &[&str], client: ClientSelection) -> MaterializedOutput {
    let mut selection = Selection::modules(modules.iter().copied());
    selection.client = client;
    baseline_catalog()
        .plan(&selection, &PlanCtx::default())
        .expect("plan should succeed")
        .materialize()
        .expect("materialize should succeed")
}

fn file<'a>(out: &'a MaterializedOutput, path: &str) -> Option<&'a OutputFile> {
    out.files.iter().find(|f| f.path == path)
}

fn dependencies(out: &MaterializedOutput) -> Vec<String> {
    let pyproject: toml::Table = file(out, "client/pyproject.toml")
        .expect("pyproject.toml is written")
        .contents
        .parse()
        .expect("pyproject.toml is valid TOML");
    pyproject["project"]["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d.as_str().unwrap().to_string())
        .collect()
}

const PYTHON: ClientSelection = ClientSelection {
    python: true,
    examples: false,
};
const EXAMPLES: ClientSelection = ClientSelection {
    python: true,
    examples: true,
};

#[test]
fn off_by_default() {
    let out = materialize(&["envoy", "postgres", "mlflow"], ClientSelection::default());
    assert!(
        !out.files
            .iter()
            .any(|f| f.path.starts_with("client/") && !f.path.ends_with(".env")),
        "only the client env files are written"
    );
}

#[test]
fn dependencies_follow_the_selected_modules() {
    let base = dependencies(&materialize(&["envoy"], PYTHON));
    assert!(base.iter().any(|d| d.starts_with("marimo")), "{base:?}");
    assert!(!base.iter().any(|d| d.starts_with("mlflow")), "{base:?}");

    let with_mlflow = dependencies(&materialize(&["envoy", "postgres", "mlflow"], PYTHON));
    assert!(
        with_mlflow.iter().any(|d| d.starts_with("mlflow")),
        "{with_mlflow:?}"
    );
    assert!(
        with_mlflow.iter().any(|d| d == "scikit-learn"),
        "{with_mlflow:?}"
    );
}

#[test]
fn examples_only_when_asked_for_and_preserved() {
    let modules = ["envoy", "postgres", "mlflow"];
    let python_only = materialize(&modules, PYTHON);
    assert!(file(&python_only, "client/notebooks/00_connect.py").is_some());
    assert!(file(&python_only, "client/notebooks/mlflow_quickstart.py").is_none());

    let examples = materialize(&modules, EXAMPLES);
    let notebook = file(&examples, "client/notebooks/mlflow_quickstart.py")
        .expect("the mlflow example is written");
    assert!(
        notebook.preserve,
        "user edits to notebooks survive a re-render"
    );
    assert!(!notebook.sensitive);
    assert!(
        !file(&examples, "client/pyproject.toml").unwrap().preserve,
        "pyproject.toml is regenerated"
    );
    assert!(
        file(&examples, "client/README.md")
            .unwrap()
            .contents
            .contains("mlflow_quickstart.py")
    );

    // Examples alone imply the workspace.
    let implied = materialize(
        &modules,
        ClientSelection {
            python: false,
            examples: true,
        },
    );
    assert!(file(&implied, "client/pyproject.toml").is_some());
}

#[test]
fn no_example_for_an_unselected_module() {
    let out = materialize(&["envoy", "postgres", "unity-catalog"], EXAMPLES);
    assert!(file(&out, "client/notebooks/mlflow_quickstart.py").is_none());
}

#[test]
fn client_selection_round_trips_through_the_manifest() {
    use olai_stack_topology::EnvManifest;

    let mut selection = Selection::modules(["envoy"]);
    selection.client = EXAMPLES;
    let manifest = EnvManifest::new(selection.clone(), PlanCtx::default());
    let text = manifest.to_toml().unwrap();
    assert!(text.contains("[selection.client]"), "{text}");
    assert_eq!(EnvManifest::from_toml(&text).unwrap().selection, selection);

    // An all-default section is omitted, so existing manifests are unchanged.
    let plain = EnvManifest::new(Selection::modules(["envoy"]), PlanCtx::default());
    assert!(!plain.to_toml().unwrap().contains("client"));
}
