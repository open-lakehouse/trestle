//! `trestle workspace` through the real binary: members get their own ports, project names
//! and the shared mesh network, and a render rejects members that would collide.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_trestle"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run trestle")
}

fn trestle(dir: &Path, args: &[&str]) -> Output {
    let out = run(dir, args);
    assert!(
        out.status.success(),
        "trestle {args:?} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn read(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path.as_ref()).unwrap_or_else(|e| panic!("{}: {e}", path.as_ref().display()))
}

/// A `demo` workspace with a mesh and two members, `dev` and `prod`.
fn demo(root: &Path) -> std::path::PathBuf {
    let ws = root.join("demo");
    fs::create_dir_all(&ws).unwrap();
    trestle(&ws, &["workspace", "init", "--mesh"]);
    for env in ["dev", "prod"] {
        trestle(
            &ws,
            &["workspace", "add", env, "--select", "unity-catalog", "-y"],
        );
    }
    ws
}

#[test]
fn members_get_their_own_ports_and_projects() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = demo(tmp.path());

    let workspace = read(ws.join("trestle-workspace.toml"));
    assert!(workspace.contains("name = \"demo\""), "{workspace}");
    assert!(workspace.contains("slot = 1"), "{workspace}");

    assert!(read(ws.join("dev/compose.yaml")).contains("name: demo-dev"));
    assert!(read(ws.join("prod/compose.yaml")).contains("name: demo-prod"));
    let dev_gateway = read(ws.join("dev/modules/envoy/compose.yaml"));
    let prod_gateway = read(ws.join("prod/modules/envoy/compose.yaml"));
    assert!(dev_gateway.contains("\"9080:10000\""), "{dev_gateway}");
    assert!(prod_gateway.contains("\"10080:10000\""), "{prod_gateway}");
    for gateway in [&dev_gateway, &prod_gateway] {
        assert!(gateway.contains("name: trestle-demo-mesh"), "{gateway}");
    }
    assert!(prod_gateway.contains("- prod.mesh"), "{prod_gateway}");

    let listing = trestle(&ws, &["workspace", "list", "--json"]);
    let rows: serde_json::Value = serde_json::from_slice(&listing.stdout).unwrap();
    assert_eq!(rows[1]["project"], "demo-prod");
    assert_eq!(rows[1]["gateway"], "http://localhost:10080");

    // A clean workspace renders, and a duplicate member is refused.
    trestle(&ws, &["workspace", "render"]);
    let dup = run(&ws, &["workspace", "add", "dev", "-y"]);
    assert!(!dup.status.success());
}

#[test]
fn render_restores_the_slot_and_rejects_shared_slots() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = demo(tmp.path());

    // A member hand-edited back to the single-env ports is put back on its slot.
    let prod_toml = ws.join("prod/env.toml");
    fs::write(
        &prod_toml,
        read(&prod_toml).replace("gateway_host_port = 10080", "gateway_host_port = 9080"),
    )
    .unwrap();
    trestle(&ws.join("prod"), &["env", "render"]);
    assert!(read(&prod_toml).contains("gateway_host_port = 10080"));

    // Two members on one slot can't run side by side.
    let manifest = ws.join("trestle-workspace.toml");
    fs::write(&manifest, read(&manifest).replace("slot = 1", "slot = 0")).unwrap();
    let clash = run(&ws, &["workspace", "render"]);
    assert!(!clash.status.success());
    let stderr = String::from_utf8_lossy(&clash.stderr);
    assert!(stderr.contains("slot 0"), "{stderr}");
}
