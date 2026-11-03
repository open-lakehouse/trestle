//! The store-backed commands (`ws`, `env create …`, `var`, `up`/`down`/`ps`/`health`) through
//! the real binary, each test in a `TRESTLE_HOME` of its own, on the fake engine and keychain
//! (feature `test-engine`).

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;

/// A trestle home with a fake Docker and keychain.
struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    fn new() -> Self {
        Home {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut cmd = Command::cargo_bin("trestle").unwrap();
        cmd.args(args)
            .current_dir(self.dir.path())
            .env("TRESTLE_HOME", self.path("home"))
            .env("TRESTLE_FAKE_ENGINE", self.path("engine.json"))
            .env("TRESTLE_FAKE_KEYCHAIN", self.path("keychain.json"))
            .env_remove("TRESTLE_LOG");
        cmd
    }

    /// Run, expect success, return stdout.
    fn ok(&self, args: &[&str]) -> String {
        let out = self.cmd(args).output().unwrap();
        assert!(
            out.status.success(),
            "trestle {args:?} failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    /// Run with `stdin`, expect success.
    fn ok_with_stdin(&self, args: &[&str], stdin: &str) -> String {
        let out = self.cmd(args).write_stdin(stdin).output().unwrap();
        assert!(
            out.status.success(),
            "trestle {args:?} failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    /// Run, expect a JSON document of `kind`, return its data.
    fn json(&self, args: &[&str], kind: &str) -> Value {
        let doc: Value = serde_json::from_str(&self.ok(args)).unwrap();
        assert_eq!(doc["version"], 1, "{doc}");
        assert_eq!(doc["kind"], kind, "{doc}");
        doc["data"].clone()
    }

    /// Run, expect failure, return stderr.
    fn fails(&self, args: &[&str]) -> String {
        let out = self.cmd(args).output().unwrap();
        assert!(
            !out.status.success(),
            "trestle {args:?} succeeded:\n{}",
            String::from_utf8_lossy(&out.stdout)
        );
        String::from_utf8(out.stderr).unwrap()
    }

    /// The projects the fake engine runs.
    fn projects(&self) -> Vec<String> {
        match std::fs::read_to_string(self.path("engine.json")) {
            Ok(text) => serde_json::from_str(&text).unwrap(),
            Err(_) => Vec::new(),
        }
    }

    /// The fake keychain.
    fn keychain(&self) -> serde_json::Map<String, Value> {
        match std::fs::read_to_string(self.path("keychain.json")) {
            Ok(text) => serde_json::from_str(&text).unwrap(),
            Err(_) => Default::default(),
        }
    }
}

/// `demo` (meshed) with `dev` committed once: Unity Catalog and what it needs.
fn demo(home: &Home) {
    home.ok(&["ws", "create", "demo", "--mesh"]);
    home.ok(&["env", "create", "demo/dev", "--select", "unity-catalog"]);
    home.ok(&["env", "commit", "demo/dev"]);
}

#[test]
fn workspaces_are_created_listed_shown_and_removed() {
    let home = Home::new();
    assert_eq!(
        home.json(&["ws", "list", "--json"], "workspaces"),
        serde_json::json!([])
    );
    demo(&home);
    home.ok(&["ws", "create", "other"]);
    let err = home.fails(&["ws", "create", "demo"]);
    assert!(err.contains("demo"), "{err}");
    let err = home.fails(&["ws", "create", "Bad_Name"]);
    assert!(err.contains("Bad_Name"), "{err}");

    insta::assert_json_snapshot!(
        "ws_list",
        home.json(&["ws", "list", "--json"], "workspaces")
    );
    insta::assert_json_snapshot!(
        "ws_show",
        home.json(&["ws", "show", "demo", "--json"], "workspace")
    );

    let err = home.fails(&["ws", "show", "nope"]);
    assert!(
        err.contains("no workspace `nope`") && err.contains("demo, other"),
        "{err}"
    );

    home.ok(&["ws", "rm", "other"]);
    let names: Vec<Value> = home
        .json(&["ws", "list", "--json"], "workspaces")
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["name"].clone())
        .collect();
    assert_eq!(names, ["demo"]);
}

#[test]
fn the_draft_is_edited_committed_and_diffed() {
    let home = Home::new();
    demo(&home);

    home.ok(&["env", "add-module", "demo/dev", "mlflow"]);
    home.ok(&["env", "set-knob", "demo/dev", "mlflow.log_level=debug"]);
    let err = home.fails(&["env", "add-module", "demo/dev", "no-such-module"]);
    assert!(err.contains("no-such-module"), "{err}");
    let err = home.fails(&["env", "remove-module", "demo/dev", "jaeger"]);
    assert!(err.contains("not selected"), "{err}");
    let err = home.fails(&["env", "add-module", "demo/dev@1", "jaeger"]);
    assert!(err.contains("immutable"), "{err}");

    // The failed edits left the draft as it was.
    let draft = home.json(&["env", "show", "demo/dev", "--json"], "environment");
    assert_eq!(draft["revision"], "draft");
    assert!(
        draft["modules"]
            .as_array()
            .unwrap()
            .contains(&"mlflow".into())
    );
    assert!(
        !draft["modules"]
            .as_array()
            .unwrap()
            .contains(&"no-such-module".into())
    );
    assert_eq!(draft["knobs"]["mlflow"]["log_level"], "debug");

    let diff = home.ok(&["env", "diff", "demo/dev"]);
    assert!(
        diff.contains("--- r1/env.toml") && diff.contains("+++ draft/env.toml"),
        "{diff}"
    );
    assert!(diff.contains("+    \"mlflow\","), "{diff}");

    assert!(home.ok(&["env", "commit", "demo/dev"]).contains("r2"));
    assert!(
        home.ok(&["env", "commit", "demo/dev"])
            .contains("nothing to commit")
    );
    assert!(
        home.ok(&["env", "diff", "demo/dev"])
            .contains("are the same")
    );
    let diff = home.ok(&["env", "diff", "demo/dev@1", "--to", "2"]);
    assert!(
        diff.contains("--- r1/env.toml") && diff.contains("+++ r2/env.toml"),
        "{diff}"
    );

    insta::assert_json_snapshot!(
        "env_revisions",
        home.json(&["env", "revisions", "demo/dev", "--json"], "revisions"),
        { "[].digest" => "[digest]" }
    );
    insta::assert_json_snapshot!(
        "env_show_r1",
        home.json(&["env", "show", "demo/dev@1", "--json"], "environment"),
        { ".digest" => "[digest]" }
    );

    // A revision exports as an environment directory that renders on its own.
    home.ok(&["env", "export", "demo/dev@1", "out/dev"]);
    let toml = std::fs::read_to_string(home.path("out/dev/env.toml")).unwrap();
    assert!(
        toml.contains("unity-catalog") && !toml.contains("mlflow"),
        "{toml}"
    );
    home.ok(&["env", "render", "out/dev"]);
    assert!(home.path("out/dev/compose.yaml").is_file());

    home.ok(&["env", "rm", "demo/dev"]);
    assert!(
        home.fails(&["env", "show", "demo/dev"])
            .contains("no environment `dev`")
    );
}

#[test]
fn vars_go_to_their_scope_and_secrets_to_the_keychain() {
    let home = Home::new();
    demo(&home);

    home.ok(&["var", "set", "demo", "REGION=eu"]);
    home.ok(&["var", "set", "demo/dev", "LOG=info", "EMPTY="]);
    home.ok_with_stdin(&["var", "set", "demo/dev", "--secret", "TOKEN"], "s3cr3t\n");
    home.ok(&["var", "set", "demo/dev", "--draft", "FEATURE=on"]);
    home.ok_with_stdin(
        &["var", "set", "demo/dev", "--draft", "--secret", "DRAFT_PW"],
        "pw",
    );

    assert_eq!(home.keychain()["demo/dev/TOKEN"], "s3cr3t");
    assert_eq!(home.keychain()["demo/dev@r2/DRAFT_PW"], "pw");
    insta::assert_json_snapshot!(
        "var_list_env",
        home.json(&["var", "list", "demo/dev", "--json"], "vars")
    );
    let ws_vars = home.json(&["var", "list", "demo", "--json"], "vars");
    assert_eq!(ws_vars[0]["value"], "eu");
    // Revision 1 carries none of the draft's vars.
    assert_eq!(
        home.json(&["var", "list", "demo/dev@1", "--json"], "vars"),
        serde_json::json!([])
    );

    // A secret is never taken from the command line, a revision never changes, and the
    // draft scope needs an environment.
    assert!(
        home.fails(&["var", "set", "demo/dev", "--secret", "TOKEN=x"])
            .contains("stdin")
    );
    assert!(
        home.fails(&["var", "set", "demo/dev@1", "X=1"])
            .contains("immutable")
    );
    assert!(
        home.fails(&["var", "set", "demo", "--draft", "X=1"])
            .contains("environment")
    );
    assert!(
        home.fails(&["var", "set", "demo", "1X=1"])
            .contains("not a variable name")
    );

    // Committing freezes the draft's secret reference into r2; unsetting it from the draft
    // keeps the keychain entry r2 still uses.
    home.ok(&["env", "commit", "demo/dev"]);
    home.ok(&["var", "unset", "demo/dev", "--draft", "DRAFT_PW"]);
    assert!(home.keychain().contains_key("demo/dev@r2/DRAFT_PW"));
    // An environment secret nothing else uses goes with its var.
    home.ok(&["var", "unset", "demo/dev", "TOKEN"]);
    assert!(!home.keychain().contains_key("demo/dev/TOKEN"));

    // Removing the workspace takes its keychain entries along.
    home.ok(&["ws", "rm", "demo"]);
    assert!(home.keychain().is_empty(), "{:?}", home.keychain());
}

#[test]
fn revisions_run_side_by_side_as_instances() {
    let home = Home::new();
    demo(&home);
    home.ok_with_stdin(&["var", "set", "demo/dev", "--secret", "TOKEN"], "s3cr3t");
    home.ok(&["env", "add-module", "demo/dev", "jaeger"]);

    let up = home.ok(&["up", "demo/dev"]);
    assert!(up.contains("demo_dev_r1"), "{up}");
    home.ok(&["env", "commit", "demo/dev"]);
    home.ok(&["up", "demo/dev@2"]);
    assert_eq!(home.projects(), ["demo_dev_r1", "demo_dev_r2"]);
    // `up` again is idempotent: the same instance.
    home.ok(&["up", "demo/dev@1"]);

    insta::assert_json_snapshot!(
        "ps",
        home.json(&["ps", "--json"], "instances"),
        { "[].instance" => "[id]" }
    );
    let health = home.json(&["health", "demo/dev@2", "--json"], "health");
    assert_eq!(health["project"], "demo_dev_r2");
    assert!(
        !health["services"].as_object().unwrap().is_empty(),
        "{health}"
    );

    // The effective environment shows where each variable comes from, secrets masked.
    let effective = home.json(
        &["env", "show", "demo/dev@1", "--effective", "--json"],
        "effective_env",
    );
    let token = &effective["vars"]["TOKEN"];
    assert_eq!(token["secret"], true);
    assert_ne!(token["value"], "s3cr3t");
    assert_eq!(token["provenance"]["layer"], "environment_vars");
    assert_eq!(
        effective["vars"]["COMPOSE_PROJECT_NAME"]["value"],
        "demo_dev_r1"
    );
    assert!(!effective.to_string().contains("s3cr3t"));

    // An environment with a running instance isn't deleted by accident.
    assert!(home.fails(&["env", "rm", "demo/dev"]).contains("--force"));

    home.ok(&["down", "demo/dev@1"]);
    assert_eq!(home.projects(), ["demo_dev_r2"]);
    let ps = home.json(&["ps", "demo/dev", "--json"], "instances");
    assert_eq!(ps[0]["desired"], "stopped");
    assert_eq!(ps[1]["desired"], "running");

    home.ok(&["down", "demo", "--rm"]);
    assert!(home.projects().is_empty());
    assert_eq!(
        home.json(&["ps", "--json"], "instances"),
        serde_json::json!([])
    );
    assert!(home.fails(&["health", "demo/dev"]).contains("no instance"));
}

/// A workspace directory as the deprecated `trestle workspace` writes it.
fn workspace_dir(home: &Home) -> PathBuf {
    let dir = home.path("legacy");
    std::fs::create_dir_all(&dir).unwrap();
    let out = home
        .cmd(&["workspace", "init", "--mesh", "--name", "legacy"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("deprecated"));
    for env in ["dev", "prod"] {
        let out = home
            .cmd(&["workspace", "add", env, "--select", "unity-catalog", "-y"])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    dir
}

#[test]
fn a_workspace_directory_imports_runs_and_exports() {
    let home = Home::new();
    let dir = workspace_dir(&home);
    let imported = home.ok(&["ws", "import", dir.to_str().unwrap()]);
    assert!(imported.contains("2 environments"), "{imported}");
    assert!(
        home.fails(&["ws", "import", dir.to_str().unwrap()])
            .contains("legacy")
    );

    home.ok(&["up", "legacy/prod"]);
    let out = home.path("exported");
    home.ok(&["ws", "export", "legacy", out.to_str().unwrap()]);
    let ws = std::fs::read_to_string(out.join("trestle-workspace.toml")).unwrap();
    assert!(
        ws.contains("name = \"legacy\"") && ws.contains("mesh = true"),
        "{ws}"
    );
    for env in ["dev", "prod"] {
        assert_eq!(
            read_toml(&out.join(env).join("env.toml")),
            read_toml(&dir.join(env).join("env.toml")),
            "{env}"
        );
    }
    // The export is a workspace directory again: it renders.
    let render = home
        .cmd(&["workspace", "render"])
        .current_dir(&out)
        .output()
        .unwrap();
    assert!(
        render.status.success(),
        "{}",
        String::from_utf8_lossy(&render.stderr)
    );
    // Exporting into a non-empty directory needs --force.
    assert!(
        home.fails(&["ws", "export", "legacy", out.to_str().unwrap()])
            .contains("not empty")
    );

    assert!(
        home.fails(&["ws", "rm", "legacy"])
            .contains("running instance")
    );
    home.ok(&["ws", "rm", "legacy", "--force"]);
    assert!(home.projects().is_empty());
}

/// An `env.toml` without its comments (the trailing knob reference), as a TOML value.
fn read_toml(path: &Path) -> toml::Value {
    toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn targets_are_checked() {
    let home = Home::new();
    demo(&home);
    for (args, needle) in [
        (&["up", "demo"][..], "names no environment"),
        (&["up", "demo/nope"], "no environment `nope`"),
        (&["up", "demo/dev@9"], "revision"),
        (&["up", "demo/dev@draft"], "draft"),
        (&["env", "commit", "demo@1"], "needs an environment"),
        (&["logs", "demo/dev"], "no instance"),
    ] {
        let err = home.fails(args);
        assert!(err.contains(needle), "{args:?}: {err}");
    }
}
