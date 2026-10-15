//! Hosted (cloud-provided) resources: a hybrid environment whose Postgres and object store
//! are managed cloud services while the rest of the stack runs on Docker Compose.

use olai_stack_topology::{
    HostedConnection, HostedKind, HostedResource, ModuleId, Plan, PlanCtx, PlanError, Secrets,
    Selection, Vantage, baseline_catalog, render_all,
};

fn hybrid_selection() -> Selection {
    let mut selection = Selection::modules([
        "envoy",
        "hosted-postgres",
        "aws-s3",
        "unity-catalog",
        "mlflow",
    ]);
    selection.hosted.insert(
        ModuleId::from("hosted-postgres"),
        HostedResource {
            cloud: Some("aws".into()),
            connection: HostedConnection::Postgres {
                host: "db.example.com".into(),
                port: 5432,
                database: "{name}".into(),
                username: "app".into(),
                password: "${PG_PASSWORD}".into(),
            },
        },
    );
    selection.hosted.insert(
        ModuleId::from("aws-s3"),
        HostedResource {
            cloud: Some("aws".into()),
            connection: HostedConnection::S3 {
                region: "us-east-1".into(),
                endpoint: None,
                bucket: "{name}".into(),
                access_key_id: "${AWS_ACCESS_KEY_ID}".into(),
                secret_access_key: "${AWS_SECRET_ACCESS_KEY}".into(),
                role_arn: Some("arn:aws:iam::123456789012:role/uc".into()),
            },
        },
    );
    selection
}

fn secrets() -> Secrets {
    [
        ("PG_PASSWORD", "pg-secret-value"),
        ("AWS_ACCESS_KEY_ID", "AKIAEXAMPLE"),
        ("AWS_SECRET_ACCESS_KEY", "aws-secret-value"),
    ]
    .into_iter()
    .collect()
}

fn hybrid_plan() -> Plan {
    let ctx = PlanCtx {
        env_name: "hybrid".into(),
        secrets: Some(secrets()),
        ..PlanCtx::default()
    };
    baseline_catalog()
        .plan(&hybrid_selection(), &ctx)
        .expect("hybrid plans")
}

fn file<'a>(out: &'a olai_stack_topology::MaterializedOutput, path: &str) -> &'a str {
    out.files
        .iter()
        .find(|f| f.path == path)
        .map(|f| f.contents.as_str())
        .unwrap_or_else(|| panic!("expected {path}"))
}

#[test]
fn hosted_providers_replace_the_in_stack_ones_and_are_not_rendered() {
    let plan = hybrid_plan();
    for id in ["hosted-postgres", "aws-s3"] {
        assert!(
            plan.graph.module(&ModuleId::from(id)).is_some(),
            "{id} in graph"
        );
    }
    for id in ["postgres", "rustfs"] {
        assert!(
            plan.graph.module(&ModuleId::from(id)).is_none(),
            "{id} not in graph"
        );
    }
    let out = plan.materialize().unwrap();
    assert!(
        !out.files
            .iter()
            .any(|f| f.path.starts_with("modules/hosted-postgres/")
                || f.path.starts_with("modules/aws-s3/")),
        "nothing rendered for hosted providers"
    );
    let compose = file(&out, "compose.yaml");
    assert!(!compose.contains("modules/hosted-postgres/"), "{compose}");
    assert!(!compose.contains("modules/aws-s3/"), "{compose}");
    assert!(compose.contains("modules/unity-catalog/compose.yaml"));
}

#[test]
fn consumers_bind_the_hosted_connection_with_resolved_secrets() {
    let plan = hybrid_plan();
    let arts = render_all(&plan).unwrap();
    let (_, uc) = arts
        .modules
        .iter()
        .find(|(id, _)| id.as_str() == "unity-catalog")
        .unwrap();
    let hibernate = uc
        .files
        .iter()
        .find(|f| f.path.ends_with("hibernate.properties"))
        .expect("UC hibernate.properties");
    assert!(
        hibernate
            .contents
            .contains("jdbc:postgresql://db.example.com:5432/unitycatalog")
    );
    assert!(hibernate.contents.contains("password=pg-secret-value"));
    // A file holding a resolved secret is sensitive; this one already sits under the
    // git-ignored `modules/*/secrets/`.
    assert!(hibernate.sensitive);
    assert!(hibernate.path.starts_with("modules/unity-catalog/secrets/"));
    assert!(arts.gitignore.contains("modules/*/secrets/"));
}

#[test]
fn a_plain_file_holding_a_resolved_secret_is_sensitive_and_ignored() {
    use olai_stack_topology::{
        Catalog, DataModule, Placement, Provides, RenderFile, RenderSpec, ResourceDemand, Role,
        ServiceSpec,
    };
    // An app that writes its database URL into a plain (non-secret) mounted config file.
    let app = DataModule {
        id: ModuleId::from("app"),
        display_name: None,
        summary: None,
        category: None,
        provider_of: None,
        requires: vec![],
        conflicts_with: vec![],
        needs: vec![ResourceDemand {
            resource: "relational_db".into(),
            name: "app".into(),
            provider: None,
            bind: Default::default(),
        }],
        service_specs: vec![ServiceSpec {
            name: "app".into(),
            role: Role::new("app"),
            placement: Placement::Container {
                service: "app".into(),
            },
            endpoints: vec![],
            depends_on: vec![],
            base_path: String::new(),
        }],
        provides: Provides::default(),
        knobs: vec![],
        renders: RenderSpec {
            fragment: "services:\n  app: {}\n".into(),
            files: vec![RenderFile {
                path: "app.conf".into(),
                contents: "db = {{ connections.relational_db[0].url }}\n".into(),
                alias: Some("app_conf".into()),
                sensitive: false,
                secret_alias: None,
                at_root: false,
                preserve: false,
            }],
        }
        .for_compose(),
    };
    let catalog = baseline_catalog().merge(Catalog::from_modules([
        std::sync::Arc::new(app) as std::sync::Arc<dyn olai_stack_topology::Module>
    ]));
    let mut selection = hybrid_selection();
    selection.modules = vec!["app".into()];
    selection.hosted.remove(&ModuleId::from("aws-s3"));
    let ctx = PlanCtx {
        secrets: Some(secrets()),
        ..PlanCtx::default()
    };
    let arts = render_all(&catalog.plan(&selection, &ctx).unwrap()).unwrap();
    let (_, out) = arts
        .modules
        .iter()
        .find(|(id, _)| id.as_str() == "app")
        .unwrap();
    let conf = &out.files[0];
    assert!(conf.contents.contains("pg-secret-value"));
    assert!(conf.sensitive);
    assert!(
        arts.gitignore.lines().any(|l| l == "modules/app/app.conf"),
        "{}",
        arts.gitignore
    );
}

#[test]
fn consumers_do_not_wait_on_hosted_providers() {
    let arts = render_all(&hybrid_plan()).unwrap();
    for (id, out) in &arts.modules {
        for gate in ["db:", "rustfs-init:"] {
            assert!(
                !out.fragment.contains(gate),
                "{id} still waits on `{gate}`:\n{}",
                out.fragment
            );
        }
    }
}

#[test]
fn the_gateway_does_not_front_or_emulate_hosted_providers() {
    let plan = hybrid_plan();
    assert!(
        plan.gateway
            .clusters
            .iter()
            .all(|c| c.host != "rustfs" && c.host != "sts-shim"),
        "{:?}",
        plan.gateway.clusters
    );
    assert!(plan.gateway.tls.is_none(), "no emulated AWS hostnames");
    assert_eq!(
        plan.gateway.listeners.len(),
        1,
        "no dedicated object-store listener"
    );
    // UC's object-store connection carries no local-CA trust.
    let arts = render_all(&plan).unwrap();
    assert!(!arts.envoy.contains("emulated_tls"));
}

#[test]
fn hosted_services_resolve_to_their_origin_from_every_vantage() {
    let plan = hybrid_plan();
    let pg = plan.service(&ModuleId::from("hosted-postgres")).unwrap();
    let endpoint = &pg.spec().endpoints[0].id;
    for from in [Vantage::Host, Vantage::Container, Vantage::InProcess] {
        assert_eq!(
            pg.address(from, endpoint).unwrap().as_str(),
            "tcp://db.example.com:5432"
        );
    }
    let s3 = plan.service(&ModuleId::from("aws-s3")).unwrap();
    assert_eq!(
        s3.address(Vantage::Host, "s3").unwrap().as_str(),
        "https://s3.us-east-1.amazonaws.com/"
    );
}

#[test]
fn layout_lists_hosted_resources_and_their_secrets() {
    let out = hybrid_plan().materialize().unwrap();
    let layout = file(&out, "LAYOUT.md");
    assert!(layout.contains("## Hosted resources"), "{layout}");
    assert!(layout.contains("`PG_PASSWORD`"));
    assert!(
        layout.contains("`unitycatalog`"),
        "databases that must exist"
    );
    assert!(
        !layout.contains("pg-secret-value"),
        "never the secret itself"
    );
}

#[test]
fn missing_secrets_fail_planning_with_every_name() {
    let ctx = PlanCtx {
        secrets: Some(Secrets::new()),
        ..PlanCtx::default()
    };
    let err = baseline_catalog()
        .plan(&hybrid_selection(), &ctx)
        .unwrap_err();
    assert_eq!(
        err,
        PlanError::UnresolvedSecrets {
            vars: vec![
                "AWS_ACCESS_KEY_ID".into(),
                "AWS_SECRET_ACCESS_KEY".into(),
                "PG_PASSWORD".into(),
            ]
        }
    );
}

#[test]
fn a_preview_without_secrets_keeps_the_references() {
    // No secrets supplied (e.g. the browser editor): references stay verbatim.
    let plan = baseline_catalog()
        .plan(&hybrid_selection(), &PlanCtx::default())
        .expect("previews plan");
    let arts = render_all(&plan).unwrap();
    let (_, uc) = arts
        .modules
        .iter()
        .find(|(id, _)| id.as_str() == "unity-catalog")
        .unwrap();
    assert!(
        uc.files
            .iter()
            .any(|f| f.contents.contains("password=${PG_PASSWORD}"))
    );
}

#[test]
fn a_hosted_entry_must_configure_a_hosted_module_of_its_kind() {
    let postgres = HostedResource {
        cloud: None,
        connection: HostedConnection::Postgres {
            host: "db".into(),
            port: 5432,
            database: "{name}".into(),
            username: "u".into(),
            password: "p".into(),
        },
    };
    for id in ["mlflow", "postgres", "aws-s3"] {
        let mut selection = Selection::modules(["envoy"]);
        selection
            .hosted
            .insert(ModuleId::from(id), postgres.clone());
        let err = baseline_catalog()
            .plan(&selection, &PlanCtx::default())
            .unwrap_err();
        assert!(
            matches!(err, PlanError::InvalidHosted { ref module, .. } if module.as_str() == id),
            "{id}: {err:?}"
        );
    }
}

#[test]
fn configuring_a_hosted_provider_selects_it() {
    // Only UC is selected; configuring hosted Postgres pulls it in as the relational provider.
    let mut selection = hybrid_selection();
    selection.modules = vec!["unity-catalog".into()];
    selection.hosted.remove(&ModuleId::from("aws-s3"));
    let plan = baseline_catalog()
        .plan(&selection, &PlanCtx::default())
        .unwrap();
    assert!(plan.hosted.contains_key(&ModuleId::from("hosted-postgres")));
    assert!(plan.graph.module(&ModuleId::from("postgres")).is_none());
}

#[test]
fn an_unconfigured_hosted_provider_reads_the_conventional_env_vars() {
    let selection = Selection::modules(["envoy", "hosted-postgres", "aws-s3", "unity-catalog"]);
    let plan = baseline_catalog()
        .plan(&selection, &PlanCtx::default())
        .unwrap();
    assert_eq!(
        plan.hosted[&ModuleId::from("aws-s3")],
        HostedKind::S3.default_resource()
    );
    let ctx = PlanCtx {
        secrets: Some(Secrets::new()),
        ..PlanCtx::default()
    };
    let err = baseline_catalog().plan(&selection, &ctx).unwrap_err();
    assert_eq!(
        err,
        PlanError::UnresolvedSecrets {
            vars: [
                "AWS_ACCESS_KEY_ID",
                "AWS_SECRET_ACCESS_KEY",
                "PGHOST",
                "PGPASSWORD",
                "PGUSER"
            ]
            .map(String::from)
            .to_vec()
        }
    );
}

#[test]
fn azure_storage_binds_abfss_uris_for_the_account() {
    let mut selection = Selection::modules(["envoy", "azure-storage", "unity-catalog"]);
    selection.hosted.insert(
        ModuleId::from("azure-storage"),
        HostedResource {
            cloud: Some("azure".into()),
            connection: HostedConnection::AzureBlob {
                account: "lakeacct".into(),
                container: "{name}".into(),
                connection_string: "${AZURE_STORAGE_CONNECTION_STRING}".into(),
            },
        },
    );
    let plan = baseline_catalog()
        .plan(&selection, &PlanCtx::default())
        .unwrap();
    assert!(plan.graph.module(&ModuleId::from("rustfs")).is_none());
    let uris: Vec<&str> = plan
        .connections
        .values()
        .filter_map(|c| match c {
            olai_stack_topology::Connection::ObjectStore { uri, .. } => Some(uri.as_str()),
            _ => None,
        })
        .collect();
    assert!(!uris.is_empty());
    assert!(
        uris.iter()
            .all(|u| u.starts_with("abfss://") && u.ends_with("@lakeacct.dfs.core.windows.net")),
        "{uris:?}"
    );
}

#[test]
fn a_hosted_and_an_in_stack_provider_of_one_role_conflict() {
    let selection = Selection::modules(["envoy", "rustfs", "aws-s3", "unity-catalog"]);
    let err = baseline_catalog()
        .plan(&selection, &PlanCtx::default())
        .unwrap_err();
    assert_eq!(
        err,
        PlanError::ConflictingRoleProviders {
            role: "object_store".into(),
            providers: vec!["aws-s3".into(), "rustfs".into()],
        }
    );
}

#[test]
fn hosted_resources_round_trip_through_env_toml() {
    let manifest = olai_stack_topology::EnvManifest::new(hybrid_selection(), PlanCtx::default());
    let toml = manifest.to_toml().unwrap();
    assert!(
        toml.contains("[selection.hosted.hosted-postgres.connection]"),
        "{toml}"
    );
    assert!(toml.contains(r#"password = "${PG_PASSWORD}""#), "{toml}");
    let back = olai_stack_topology::EnvManifest::from_toml(&toml).unwrap();
    assert_eq!(back.selection, manifest.selection);
}
