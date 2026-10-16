//! `[context] local_secrets = "inline"`: credential-bearing files render as ordinary files.

use olai_stack_topology::{
    LocalSecrets, ModuleId, PlanCtx, PlanError, Selection, baseline_catalog,
};

fn ctx(local_secrets: LocalSecrets) -> PlanCtx {
    PlanCtx {
        local_secrets,
        ..PlanCtx::default()
    }
}

#[test]
fn separate_marks_credentials_sensitive_and_ignores_them() {
    let plan = baseline_catalog()
        .plan(
            &Selection::modules(["unity-catalog"]),
            &ctx(LocalSecrets::Separate),
        )
        .unwrap();
    let out = plan.materialize().unwrap();
    let props = out
        .files
        .iter()
        .find(|f| f.path == "modules/unity-catalog/secrets/server.properties")
        .unwrap();
    assert!(props.sensitive);
    let gitignore = &out
        .files
        .iter()
        .find(|f| f.path == ".gitignore")
        .unwrap()
        .contents;
    assert!(gitignore.contains("modules/*/secrets/"), "{gitignore}");
}

#[test]
fn inline_renders_every_file_as_ordinary() {
    let plan = baseline_catalog()
        .plan(
            &Selection::modules(["unity-catalog"]),
            &ctx(LocalSecrets::Inline),
        )
        .unwrap();
    let out = plan.materialize().unwrap();
    let sensitive: Vec<&str> = out
        .files
        .iter()
        .filter(|f| f.sensitive)
        .map(|f| f.path.as_str())
        .collect();
    assert!(sensitive.is_empty(), "{sensitive:?}");
    for path in [
        ".env",
        "modules/unity-catalog/secrets/server.properties",
        "modules/unity-catalog/secrets/hibernate.properties",
    ] {
        assert!(out.files.iter().any(|f| f.path == path), "{path}");
    }
    let gitignore = &out
        .files
        .iter()
        .find(|f| f.path == ".gitignore")
        .unwrap()
        .contents;
    assert_eq!(gitignore, ".data/\n");
    // Compose still mounts the properties as secrets, from the same paths.
    let compose = &out
        .files
        .iter()
        .find(|f| f.path == "compose.yaml")
        .unwrap()
        .contents;
    assert!(
        compose.contains("file: ./modules/unity-catalog/secrets/server.properties"),
        "{compose}"
    );
}

#[test]
fn inline_is_refused_with_hosted_modules() {
    let selection = Selection::modules(["unity-catalog", "aws-s3"]);
    let err = baseline_catalog()
        .plan(&selection, &ctx(LocalSecrets::Inline))
        .unwrap_err();
    assert_eq!(
        err,
        PlanError::InlineSecretsWithHosted {
            modules: vec![ModuleId::from("aws-s3")],
        }
    );
}

#[test]
fn local_secrets_round_trips_and_defaults_to_separate() {
    let inline: PlanCtx = toml::from_str("local_secrets = \"inline\"").unwrap();
    assert_eq!(inline.local_secrets, LocalSecrets::Inline);
    let default: PlanCtx = toml::from_str("").unwrap();
    assert_eq!(default.local_secrets, LocalSecrets::Separate);
    assert!(!toml::to_string(&default).unwrap().contains("local_secrets"));
}
