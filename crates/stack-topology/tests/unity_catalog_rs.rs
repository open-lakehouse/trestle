//! `unity-catalog-rs`: mangrove's Rust Unity Catalog server. Both placements run one
//! configuration document and differ in how it seals: the container through the secret
//! server's key service, holding no key-encryption key; a Neon Function with a local one read
//! from `UC_SERVER_KEK`. No key material is ever rendered.

use olai_stack_topology::{
    HostedConnection, HostedResource, HostedVendor, KeyImport, MaterializedOutput, ModuleId, Plan,
    PlanCtx, Selection, Vantage, baseline_catalog,
};

/// The local KEK reference mangrove reads as a function; the value comes from the deployment.
const KEK_REF: &str = "key: { env: UC_SERVER_KEK }";
/// The variable holding the container's keyring, on the secret server alone.
const KEYRING_VAR: &str = "TRESTLE_KEYRING_UNITY_CATALOG_RS__SERVER_KEK";

fn plan(selection: &Selection) -> Plan {
    baseline_catalog()
        .plan(selection, &PlanCtx::default())
        .expect("plans")
}

fn local() -> Plan {
    plan(&Selection::modules(["unity-catalog-rs", "mlflow"]))
}

fn as_function() -> Plan {
    let mut selection = Selection::modules(["unity-catalog-rs", "mlflow"]);
    selection.hosted.insert(
        ModuleId::from("unity-catalog-rs"),
        HostedResource {
            cloud: None,
            vendor: Some(HostedVendor::NeonFunction),
            connection: HostedConnection::Function { url: None },
        },
    );
    plan(&selection)
}

fn file<'a>(out: &'a MaterializedOutput, path: &str) -> &'a str {
    out.files
        .iter()
        .find(|f| f.path == path)
        .map(|f| f.contents.as_str())
        .unwrap_or_else(|| panic!("expected {path}"))
}

fn yaml(text: &str) -> serde_yaml::Value {
    serde_yaml::from_str(text).unwrap()
}

#[test]
fn it_serves_the_unity_catalog_api_and_can_run_as_a_function() {
    let cat = baseline_catalog();
    let module = cat.get(&ModuleId::from("unity-catalog-rs")).unwrap();
    let provides = module.provides();
    assert_eq!(provides.api.as_deref(), Some("unity-catalog"));
    assert_eq!(provides.function.as_deref(), Some("unitycatalog"));
    assert_eq!(provides.function_secrets, ["UC_SERVER_KEK"]);
    assert_eq!(provides.keys, ["server-kek"]);
    assert_eq!(
        provides.key_imports.get("server-kek"),
        Some(&KeyImport::new("UC_SERVER_KEK", "kek-1"))
    );
    // The container declares no KEK: the keyring is the secret server's.
    assert!(
        provides.secrets.iter().all(|s| s.name != "UC_SERVER_KEK"),
        "{:?}",
        provides.secrets
    );
}

#[test]
fn the_container_runs_migrate_then_serve_sealing_through_the_key_service() {
    let p = local();
    let out = p.materialize().unwrap();
    let fragment = yaml(file(&out, "modules/unity-catalog-rs/compose.yaml"));
    let services = &fragment["services"];
    assert_eq!(
        services["unitycatalog"]["depends_on"]["unitycatalog-migrate"]["condition"],
        "service_completed_successfully"
    );
    for service in ["unitycatalog", "unitycatalog-migrate"] {
        assert!(
            services[service].get("environment").is_none(),
            "{service}: {:?}",
            services[service]["environment"]
        );
    }
    // The planner's overlay gives the server the socket and its key's name; migrate, which
    // never seals, gets nothing.
    let overlay = yaml(file(
        &out,
        "modules/egress-gateway/consumers/unity-catalog-rs.yaml",
    ));
    let server = &overlay["services"]["unitycatalog"];
    assert_eq!(
        server["environment"]["TRESTLE_SDS_SOCKET"].as_str(),
        Some("/run/trestle-sds/sds.sock")
    );
    assert_eq!(
        server["environment"]["TRESTLE_KEY_SERVER_KEK"].as_str(),
        Some("server-kek")
    );
    assert_eq!(
        server["volumes"][0].as_str(),
        Some("sds-unity-catalog-rs:/run/trestle-sds")
    );
    assert!(server["environment"].get("UC_SERVER_KEK").is_none());
    assert!(overlay["services"].get("unitycatalog-migrate").is_none());

    let config = yaml(file(&out, "modules/unity-catalog-rs/config.yaml"));
    let key_service = &config["encryption"]["provider"]["key_service"];
    assert_eq!(
        key_service["socket"]["env"].as_str(),
        Some("TRESTLE_SDS_SOCKET")
    );
    assert_eq!(
        key_service["key"]["env"].as_str(),
        Some("TRESTLE_KEY_SERVER_KEK")
    );
    assert!(config["encryption"].get("active").is_none(), "{config:?}");
    assert_eq!(
        config["backend"]["url"]["env"].as_str(),
        Some("DATABASE_URL")
    );
    // The database URL lives in the git-ignored env file.
    let env = file(&out, "modules/unity-catalog-rs/.env/unitycatalog.env");
    assert!(env.starts_with("DATABASE_URL=postgres"), "{env}");

    // The served key carries where its first version comes from.
    let egress = p.egress.as_ref().expect("the egress module is planned");
    let keys: Vec<_> = egress.keys().collect();
    let [(module, key)] = keys.as_slice() else {
        panic!("{keys:?}")
    };
    assert_eq!(module.as_str(), "unity-catalog-rs");
    assert_eq!(key.env, KEYRING_VAR);
    assert_eq!(key.import, Some(KeyImport::new("UC_SERVER_KEK", "kek-1")));
}

#[test]
fn the_keyring_is_the_secret_servers_alone_and_no_file_holds_a_kek() {
    let p = local();
    // Declared on the egress module alone, required; nothing declares `UC_SERVER_KEK`.
    let declaring: Vec<&str> = p
        .secrets
        .iter()
        .filter(|(_, s)| s.iter().any(|s| s.name == KEYRING_VAR))
        .map(|(m, _)| m.as_str())
        .collect();
    assert_eq!(declaring, ["egress-gateway"]);
    assert!(
        p.declared_secrets()
            .iter()
            .any(|s| s.name == KEYRING_VAR && s.required)
    );
    assert!(
        p.declared_secrets()
            .iter()
            .all(|s| s.name != "UC_SERVER_KEK"),
        "{:?}",
        p.declared_secrets()
    );

    let out = p.materialize().unwrap();
    for f in &out.files {
        // Only prose names the old variable (the deploy step and comments), as the import's
        // source: no service or configuration reads it.
        for line in f.contents.lines() {
            if line.contains("UC_SERVER_KEK") && !line.trim_start().starts_with('#') {
                assert_eq!(f.path, "README.md", "{}: {line}", f.path);
                assert!(!line.contains("{ env: UC_SERVER_KEK }"), "{line}");
            }
        }
        if !f.contents.contains(KEYRING_VAR) {
            continue;
        }
        match f.path.as_str() {
            "modules/egress-gateway/secret-server.yaml" | "README.md" => {}
            "modules/egress-gateway/compose.yaml" => {
                let fragment = yaml(&f.contents);
                for (name, service) in fragment["services"].as_mapping().unwrap() {
                    let text = serde_yaml::to_string(service).unwrap();
                    assert!(
                        !text.contains(KEYRING_VAR) || name.as_str() == Some("secret-server"),
                        "{name:?} names {KEYRING_VAR}"
                    );
                }
            }
            other => panic!("{other} names {KEYRING_VAR}"),
        }
    }
    let readme = file(&out, "README.md");
    assert!(
        readme.contains("Give Unity Catalog's key a keyring"),
        "{readme}"
    );
    assert!(
        readme.contains(
            "trestle key import <ws>/<env> unity-catalog-rs/server-kek --from UC_SERVER_KEK \
             --alias kek-1"
        ),
        "{readme}"
    );
}

#[test]
fn as_a_function_it_keeps_a_local_kek() {
    let container = local().materialize().unwrap();
    let container_config = file(&container, "modules/unity-catalog-rs/config.yaml");

    let p = as_function();
    let neon = p.neon_requirements();
    let [function] = neon.functions.as_slice() else {
        panic!("{:?}", neon.functions)
    };
    assert_eq!(function.slug, "unitycatalog");
    assert_eq!(function.secrets, ["UC_SERVER_KEK"]);
    let deployed = &function.env["UC_SERVER_CONFIG_YAML"];
    let doc = yaml(deployed);
    assert_eq!(doc["encryption"]["active"]["id"].as_str(), Some("kek-1"));
    assert_eq!(
        doc["encryption"]["active"]["key"]["env"].as_str(),
        Some("UC_SERVER_KEK")
    );
    assert!(doc["encryption"].get("provider").is_none(), "{deployed}");
    // Everything but `encryption` is the container's document, minus where it listens and its
    // database.
    let mut container_doc = yaml(container_config);
    let mut function_doc = doc.clone();
    for doc in [&mut container_doc, &mut function_doc] {
        let map = doc.as_mapping_mut().unwrap();
        for key in ["encryption", "host", "port", "backend"] {
            map.remove(key);
        }
    }
    assert_eq!(container_doc, function_doc);

    // No key service: no socket, no keyring.
    assert!(
        p.egress.as_ref().is_none_or(|e| e.keys().next().is_none()),
        "{:?}",
        p.egress
    );
    assert!(
        p.declared_secrets()
            .iter()
            .all(|s| !s.name.starts_with("TRESTLE_KEYRING_")),
        "{:?}",
        p.declared_secrets()
    );

    let out = p.materialize().unwrap();
    assert!(
        !out.files
            .iter()
            .any(|f| f.path.starts_with("modules/unity-catalog-rs/")
                || f.path == "modules/egress-gateway/consumers/unity-catalog-rs.yaml"),
        "nothing of it renders"
    );
    let readme = file(&out, "README.md");
    assert!(readme.contains("`UC_SERVER_CONFIG_YAML`"), "{readme}");
    assert!(readme.contains(KEK_REF), "{readme}");
    assert!(
        readme.contains("Set `UC_SERVER_KEK` on the deployment"),
        "{readme}"
    );
    // The container's keyring step is not the function's.
    assert!(
        !readme.contains("Give Unity Catalog's key a keyring"),
        "{readme}"
    );
}

#[test]
fn mlflow_serves_its_registry_from_it_either_way() {
    for p in [local(), as_function()] {
        let env = p.client_env(Vantage::Host);
        let uc = env.get("UNITY_CATALOG_URL").expect("UC address");
        let origin = uc.strip_suffix("/api/2.1/unity-catalog").expect("UC path");
        assert_eq!(
            env.get("MLFLOW_REGISTRY_URI"),
            Some(format!("uc:{origin}").as_str())
        );
    }
}

#[test]
fn the_data_catalog_capability_picks_one_server() {
    let selection = Selection {
        capabilities: vec!["data_catalog".into()],
        ..Selection::default()
    };
    let has = |p: &Plan, id: &str| p.graph.module(&ModuleId::from(id)).is_some();
    // The catalog's default: the OSS server.
    let p = plan(&selection);
    assert!(has(&p, "unity-catalog-oss") && !has(&p, "unity-catalog-rs"));
    // The environment's preference wins.
    let ctx = PlanCtx {
        provider_preference: [(
            "data_catalog".to_string(),
            vec![ModuleId::from("unity-catalog-rs")],
        )]
        .into(),
        ..PlanCtx::default()
    };
    let p = baseline_catalog().plan(&selection, &ctx).expect("plans");
    assert!(has(&p, "unity-catalog-rs") && !has(&p, "unity-catalog-oss"));
}
