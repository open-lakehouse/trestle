//! Against a real Docker daemon: `cargo test -p olai-stack-runtime --test live_docker -- --ignored`.
//! Uses the capture project in `scripts/runtime-capture/` (pulls `python:3.13-alpine`).

use std::path::Path;
use std::time::Duration;

use olai_stack_runtime::{ComposeEngine, DockerCli, DownOptions, Health, Progress, UpOptions};

#[tokio::test]
#[ignore = "needs a Docker daemon"]
async fn up_ps_down_round_trip() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/runtime-capture");
    let project = "trestle-runtime-live";
    let env: std::collections::BTreeMap<String, String> = [
        ("CAPTURE_PROJECT".to_owned(), project.to_owned()),
        ("CAPTURE_PORT".to_owned(), "18001".to_owned()),
    ]
    .into_iter()
    .collect();
    let docker = DockerCli::new();
    docker
        .up(
            &dir,
            &UpOptions {
                wait: true,
                wait_timeout: Some(Duration::from_secs(60)),
                env: env.clone(),
                progress: Progress::Quiet,
            },
        )
        .await
        .unwrap();

    let ls = docker.ls().await.unwrap();
    assert!(ls.iter().any(|p| p.name == project && p.is_running()));
    let ps = docker.ps(project).await.unwrap();
    let web = ps.iter().find(|s| s.service == "web").unwrap();
    assert_eq!(web.health, Health::Healthy);
    assert!(web.published.iter().any(|p| p.host_port == 18001));

    docker
        .down(
            &dir,
            &DownOptions {
                volumes: true,
                env,
                progress: Progress::Quiet,
            },
        )
        .await
        .unwrap();
    assert!(!docker.ls().await.unwrap().iter().any(|p| p.name == project));
}
