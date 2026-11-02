//! Golden tests of the layered environment: each fixture in `tests/fixtures/effective_env/`
//! is the masked, provenance-annotated resolution of the layers built here. Re-bless with
//! `BLESS=1 cargo test -p olai-stack-runtime --test effective_env`.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, SystemTime};

use olai_stack_runtime::{
    EnvLayers, Layer, Overlay, SecretError, SecretProvider, SecretProviders, SecretValue,
};
use olai_stack_state::{SecretRef, VarSet, VarValue};

struct Vault;

impl SecretProvider for Vault {
    fn name(&self) -> &str {
        "vault"
    }

    fn get(&self, key: &str) -> Result<SecretValue, SecretError> {
        Ok(SecretValue::new(format!("value-of-{key}")))
    }
}

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn vars(pairs: &[(&str, VarValue)]) -> VarSet {
    let mut set = VarSet::new();
    for (k, v) in pairs {
        set.set(*k, v.clone());
    }
    set
}

fn lit(v: &str) -> VarValue {
    VarValue::Literal(v.into())
}

fn secret(key: &str) -> VarValue {
    VarValue::Secret(SecretRef {
        provider: "vault".into(),
        key: key.into(),
    })
}

fn check(name: &str, layers: EnvLayers) {
    let providers = SecretProviders::new().with(Vault);
    let resolved = layers.resolve(&providers).unwrap();
    let actual = serde_json::to_string_pretty(&resolved.masked()).unwrap() + "\n";
    // Nothing a provider returned, or a literal over a secret, is shown.
    assert!(!actual.contains("value-of-"), "{actual}");
    assert!(!actual.contains("typed-in"), "{actual}");
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/effective_env")
        .join(format!("{name}.json"));
    if std::env::var_os("BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e} (run with BLESS=1)", path.display()));
    assert_eq!(actual, expected, "{name} drifted (re-bless with BLESS=1)");
}

/// Every layer sets `LEVEL` (the highest wins, the rest are shadowed lowest-first), each also
/// sets a key of its own, and the placement key resists every attempt to override it.
#[test]
fn precedence_runs_low_to_high() {
    let layers = EnvLayers::new()
        // Deliberately added out of order: resolution applies the layer order.
        .with_vars(
            Layer::InstanceOverrides,
            &vars(&[
                ("LEVEL", lit("instance")),
                ("COMPOSE_PROJECT_NAME", lit("nope")),
                // A literal over a secret default: still masked.
                ("LOCAL_RUSTFS_SECRET_KEY", lit("typed-in")),
            ]),
        )
        .with_plain(Layer::CatalogDefaults, &map(&[("LEVEL", "catalog")]))
        .with_secrets(
            Layer::CatalogDefaults,
            &map(&[
                ("LOCAL_POSTGRES_PASSWORD", "postgres"),
                ("LOCAL_RUSTFS_SECRET_KEY", "rustfsadmin"),
            ]),
        )
        .with_plain(
            Layer::RevisionManifest,
            &map(&[("LEVEL", "manifest"), ("DATA_ROOT", "./.data")]),
        )
        .with_plain(
            Layer::Placement,
            &map(&[
                ("COMPOSE_PROJECT_NAME", "demo_dev_r1"),
                ("TRESTLE_SLOT", "1"),
            ]),
        )
        .with_plain(
            Layer::ClientEnv,
            &map(&[
                ("LEVEL", "client"),
                ("GATEWAY_URL", "http://localhost:10080"),
            ]),
        )
        .with_vars(
            Layer::WorkspaceVars,
            &vars(&[
                ("LEVEL", lit("workspace")),
                ("LOCAL_POSTGRES_PASSWORD", secret("pg")),
                ("COMPOSE_PROJECT_NAME", lit("nope")),
            ]),
        )
        .with_vars(
            Layer::EnvironmentVars,
            &vars(&[("LEVEL", lit("environment")), ("REGION", lit("eu-west-1"))]),
        )
        .with_vars(
            Layer::RevisionVars,
            &vars(&[("LEVEL", lit("revision")), ("FEATURE", lit("on"))]),
        )
        .with_overlay(
            "neon",
            &Overlay {
                vars: map(&[("LEVEL", "neon")]),
                endpoints: map(&[("UC_URL", "https://uc.example")]),
                credentials: [("PGPASSWORD".to_owned(), SecretValue::new("branch-pw"))].into(),
                expires_at: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1)),
            },
        )
        .with_overlay(
            "user",
            &Overlay {
                vars: map(&[("LEVEL", "user"), ("UC_URL", "http://localhost:8080")]),
                ..Overlay::default()
            },
        );
    check("precedence", layers);
}

/// Sources apply in the order the instance names them, not by name.
#[test]
fn sources_apply_in_the_order_given() {
    let overlay = |v: &str| Overlay {
        vars: map(&[("WHO", v)]),
        ..Overlay::default()
    };
    let layers = EnvLayers::new()
        .with_overlay("zeta", &overlay("zeta"))
        .with_overlay("alpha", &overlay("alpha"));
    check("source_order", layers);
}

#[test]
fn an_unresolvable_reference_is_an_error() {
    let layers = EnvLayers::new().with_vars(
        Layer::WorkspaceVars,
        &vars(&[(
            "TOKEN",
            VarValue::Secret(SecretRef {
                provider: "keychain".into(),
                key: "t".into(),
            }),
        )]),
    );
    let err = layers
        .resolve(&SecretProviders::new().with(Vault))
        .unwrap_err();
    assert!(matches!(err, SecretError::UnknownProvider(p) if p == "keychain"));
}

#[test]
fn an_expired_overlay_is_noticed() {
    let overlay = Overlay {
        expires_at: Some(SystemTime::now() - Duration::from_secs(1)),
        ..Overlay::default()
    };
    assert!(overlay.is_expired(SystemTime::now()));
    assert!(!Overlay::default().is_expired(SystemTime::now()));
}
