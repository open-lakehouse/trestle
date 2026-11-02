//! Parsing what real engines print. Each `tests/fixtures/<engine>/` directory is captured by
//! `scripts/runtime-capture/capture.sh` from one Docker engine; every test runs on each of them.

use std::path::{Path, PathBuf};

use olai_stack_runtime::{
    EventAction, Health, State, parse_event, parse_log_line, parse_ls, parse_ps,
};

const PROJECT: &str = "trestle-capture";

fn engines() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("version.json").is_file())
        .collect();
    dirs.sort();
    assert!(!dirs.is_empty(), "no fixtures under {}", root.display());
    dirs
}

fn read(dir: &Path, file: &str) -> String {
    std::fs::read_to_string(dir.join(file))
        .unwrap_or_else(|e| panic!("{}: {e}", dir.join(file).display()))
}

#[test]
fn ls_reports_states_per_project() {
    for dir in engines() {
        let running = parse_ls(&read(&dir, "ls-running.json")).unwrap();
        let project = running.iter().find(|p| p.name == PROJECT).unwrap();
        assert_eq!(project.running(), 2, "{}", dir.display());
        assert!(project.is_running());

        let partial = parse_ls(&read(&dir, "ls-partial.json")).unwrap();
        let project = partial.iter().find(|p| p.name == PROJECT).unwrap();
        assert_eq!(project.states.get("exited"), Some(&1), "{}", dir.display());
        assert_eq!(project.running(), 2);

        assert!(parse_ls(&read(&dir, "ls-down.json")).unwrap().is_empty());
    }
}

#[test]
fn ps_reports_state_health_and_ports() {
    for dir in engines() {
        let rows = parse_ps(&read(&dir, "ps-running.json")).unwrap();
        let web = rows.iter().find(|s| s.service == "web").unwrap();
        assert_eq!(web.state, State::Running);
        assert_eq!(web.health, Health::Healthy, "{}", dir.display());
        assert!(web.is_ready());
        assert_eq!(web.published.len(), 1);
        assert_eq!(web.published[0].container_port, 8000);
        assert_eq!(web.published[0].host_port, 18000);
        // No healthcheck: running is ready.
        let ticker = rows.iter().find(|s| s.service == "ticker").unwrap();
        assert_eq!(ticker.health, Health::None);
        assert!(ticker.is_ready());

        let rows = parse_ps(&read(&dir, "ps-unhealthy.json")).unwrap();
        let broken = rows.iter().find(|s| s.service == "broken").unwrap();
        assert_eq!(broken.health, Health::Unhealthy, "{}", dir.display());
        assert!(!broken.is_ready());

        let rows = parse_ps(&read(&dir, "ps-partial.json")).unwrap();
        let ticker = rows.iter().find(|s| s.service == "ticker").unwrap();
        assert_eq!(ticker.state, State::Exited, "{}", dir.display());
        assert!(!ticker.is_ready());
    }
}

#[test]
fn events_cover_the_lifecycle_and_health() {
    for dir in engines() {
        let events: Vec<_> = read(&dir, "events.jsonl")
            .lines()
            .filter_map(|l| parse_event(l).unwrap())
            .collect();
        assert!(events.iter().all(|e| e.project == PROJECT));
        let has = |service: &str, action: &EventAction| {
            events
                .iter()
                .any(|e| e.service == service && e.action == *action)
        };
        for service in ["web", "ticker", "broken"] {
            assert!(has(service, &EventAction::Create), "{service} create");
            assert!(has(service, &EventAction::Start), "{service} start");
            assert!(has(service, &EventAction::Destroy), "{service} destroy");
        }
        assert!(has("web", &EventAction::Health(Health::Healthy)));
        assert!(has("broken", &EventAction::Health(Health::Unhealthy)));
        assert!(
            events
                .iter()
                .any(|e| matches!(e.action, EventAction::Die { exit_code: Some(_) }))
        );
        // Healthcheck probes are execs, which are dropped.
        assert!(
            events
                .iter()
                .all(|e| !matches!(&e.action, EventAction::Other(a) if a.starts_with("exec_")))
        );
        // Events arrive in time order.
        assert!(
            events
                .windows(2)
                .all(|w| w[0].time_nanos <= w[1].time_nanos)
        );
    }
}

#[test]
fn logs_carry_container_and_timestamp() {
    for dir in engines() {
        let lines: Vec<_> = read(&dir, "logs-ticker.txt")
            .lines()
            .map(parse_log_line)
            .collect();
        assert!(!lines.is_empty());
        for line in &lines {
            assert_eq!(line.container, "ticker-1", "{}", dir.display());
            assert!(line.timestamp.is_some(), "{line:?}");
            assert!(line.text.starts_with("tick "), "{line:?}");
        }
    }
}

#[test]
fn up_wait_fails_on_an_unhealthy_service() {
    for dir in engines() {
        assert_eq!(read(&dir, "up-wait-healthy.exit").trim(), "0");
        assert_ne!(read(&dir, "up-wait-unhealthy.exit").trim(), "0");
        let stderr = read(&dir, "up-wait-unhealthy.stderr");
        assert!(stderr.contains("is unhealthy"), "{stderr}");
    }
}
