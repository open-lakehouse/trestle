//! [`FakeEngine`] behaves like Docker for the code that drives it.
#![cfg(feature = "testing")]

use std::collections::BTreeSet;

use futures::StreamExt;
use olai_stack_runtime::{
    ComposeEngine, DownOptions, EventAction, EventFilter, FakeCall, FakeEngine, Health,
    ProjectMatch, UpOptions, managed_projects, remove_mesh_if_idle,
};

fn project_dir(name: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("compose.yaml"), format!("name: {name}\n")).unwrap();
    dir
}

#[tokio::test]
async fn up_and_down_drive_the_listing_and_events() {
    let engine = FakeEngine::new();
    engine.script(
        "demo-dev",
        [("web", Health::Healthy), ("db", Health::Starting)],
    );
    let mut events = engine
        .events(&EventFilter::Project("demo-dev".into()))
        .await
        .unwrap();

    let dir = project_dir("demo-dev");
    let opts = UpOptions {
        wait: true,
        env: [("SECRET".to_owned(), "s3cr3t".to_owned())].into(),
        ..UpOptions::default()
    };
    engine.up(dir.path(), &opts).await.unwrap();
    let ls = engine.ls().await.unwrap();
    assert_eq!(ls.len(), 1);
    assert_eq!(ls[0].name, "demo-dev");
    assert_eq!(ls[0].running(), 2);
    let ps = engine.ps("demo-dev").await.unwrap();
    assert!(!ps.iter().find(|s| s.service == "db").unwrap().is_ready());

    // The variables reach the engine, not a file.
    assert!(matches!(
        &engine.calls()[0],
        FakeCall::Up { project, env, .. } if project == "demo-dev" && env["SECRET"] == "s3cr3t"
    ));

    let first = events.next().await.unwrap().unwrap();
    assert_eq!(first.action, EventAction::Create);

    engine.set_health("demo-dev", "db", Health::Healthy);
    engine
        .down(dir.path(), &DownOptions::default())
        .await
        .unwrap();
    assert!(engine.ls().await.unwrap().is_empty());
    let mut seen = Vec::new();
    while let Some(Ok(e)) = events.next().await {
        let done = e.action == EventAction::Destroy && e.service == "db";
        seen.push(e.action);
        if done {
            break;
        }
    }
    assert!(
        seen.contains(&EventAction::Health(Health::Healthy)),
        "{seen:?}"
    );
}

#[tokio::test]
async fn the_mesh_goes_once_the_last_member_stops() {
    let engine = FakeEngine::new();
    engine.add_network("trestle-demo-mesh");
    let members: BTreeSet<String> = ["demo-dev".into(), "demo-prod".into()].into();
    let (dev, prod) = (project_dir("demo-dev"), project_dir("demo-prod"));
    engine.up(dev.path(), &UpOptions::default()).await.unwrap();
    engine.up(prod.path(), &UpOptions::default()).await.unwrap();

    engine
        .down(dev.path(), &DownOptions::default())
        .await
        .unwrap();
    assert!(
        !remove_mesh_if_idle(&engine, "trestle-demo-mesh", &members)
            .await
            .unwrap()
    );
    assert!(engine.networks().contains("trestle-demo-mesh"));

    engine
        .down(prod.path(), &DownOptions::default())
        .await
        .unwrap();
    assert!(
        remove_mesh_if_idle(&engine, "trestle-demo-mesh", &members)
            .await
            .unwrap()
    );
    assert!(engine.networks().is_empty());
}

#[tokio::test]
async fn managed_projects_match_exactly_or_by_prefix() {
    let engine = FakeEngine::new();
    for name in ["demo-dev", "demo-x-prod", "other"] {
        engine.adopt(name);
    }
    let exact = ProjectMatch::Exact(["demo-dev".into()].into());
    let names = |ps: Vec<olai_stack_runtime::ProjectStatus>| -> Vec<String> {
        ps.into_iter().map(|p| p.name).collect()
    };
    assert_eq!(
        names(managed_projects(&engine, &exact).await.unwrap()),
        ["demo-dev"]
    );
    // A dash-separated prefix also catches a workspace called `demo-x`.
    let prefix = ProjectMatch::Prefix("demo-".into());
    assert_eq!(
        names(managed_projects(&engine, &prefix).await.unwrap()),
        ["demo-dev", "demo-x-prod"]
    );
}

#[tokio::test]
async fn a_down_daemon_is_unavailable_not_empty() {
    let engine = FakeEngine::new();
    engine.set_unavailable(true);
    assert!(engine.ls().await.unwrap_err().is_unavailable());
}

#[tokio::test]
async fn a_failed_up_still_leaves_the_project_behind() {
    let engine = FakeEngine::new();
    engine.fail_up("demo-dev");
    let dir = project_dir("demo-dev");
    assert!(engine.up(dir.path(), &UpOptions::default()).await.is_err());
    // As with `--wait`: the containers exist, they just aren't healthy.
    assert_eq!(engine.projects(), ["demo-dev".to_owned()].into());
    // The failure is one-shot.
    engine.up(dir.path(), &UpOptions::default()).await.unwrap();
}
