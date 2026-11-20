//! Two machines on one team's shared state, through the real binary: each a `TRESTLE_HOME`
//! of its own, with the fake engine and keychain (feature `test-engine`), and a stub `neon`
//! whose `connection-string` hands out a fresh schema on the database `DATABASE_URL_PG`
//! names. Skips when the variable is unset (`just test-state-pg` sets it).

use std::path::PathBuf;

use assert_cmd::Command;
use serde_json::Value;

/// The team's database: a schema of its own, dropped at the end.
struct Team {
    admin: String,
    schema: String,
    url: String,
}

impl Team {
    fn new() -> Option<Self> {
        let Ok(admin) = std::env::var("DATABASE_URL_PG") else {
            eprintln!("DATABASE_URL_PG is not set: skipping the shared-state CLI test");
            return None;
        };
        let schema = format!(
            "trestle_cli_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        sql(&admin, &format!("CREATE SCHEMA {schema}"));
        let separator = if admin.contains('?') { '&' } else { '?' };
        let url = format!("{admin}{separator}options=-csearch_path%3D{schema}");
        Some(Team { admin, schema, url })
    }
}

impl Drop for Team {
    fn drop(&mut self) {
        sql(&self.admin, &format!("DROP SCHEMA {} CASCADE", self.schema));
    }
}

fn sql(url: &str, statement: &str) {
    use sqlx::Connection as _;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let mut conn = sqlx::PgConnection::connect(url).await.unwrap();
            sqlx::raw_sql(statement).execute(&mut conn).await.unwrap();
        });
}

/// One machine: a trestle home with a fake Docker and keychain, and a stub `neon` signed in
/// to project `proj`.
struct Machine {
    dir: tempfile::TempDir,
}

impl Machine {
    fn new(team: &Team) -> Self {
        let machine = Machine {
            dir: tempfile::tempdir().unwrap(),
        };
        let connection = serde_json::json!({
            "connection_string": team.url,
            "host": "localhost",
            "role": "postgres",
            "password": "postgres",
            "database": "trestle_state",
            "options": "",
        });
        let script = format!(
            r#"#!/bin/sh
echo "$*" >> "{log}"
case "$*" in
  "api "*) echo '{{"branches":[{{"id":"br-main","name":"main","default":true}}],"annotations":{{}}}}' ;;
  "databases list"*) echo '[{{"name":"neondb","owner_name":"neondb_owner"}}]' ;;
  "databases create"*) echo '{{}}' ;;
  connection-string*) echo '{connection}' ;;
  *) echo "unexpected: $*" >&2; exit 2 ;;
esac
"#,
            log = machine.path("neon.log").display(),
        );
        let path = machine.path("neon");
        std::fs::write(&path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        machine
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
            .env("NEON_CLI", self.path("neon"))
            .env_remove("TRESTLE_STATE")
            .env_remove("TRESTLE_LOG");
        cmd
    }

    fn ok(&self, args: &[&str]) -> String {
        self.ok_with_stdin(args, "")
    }

    fn ok_with_stdin(&self, args: &[&str], stdin: &str) -> String {
        let out = self.cmd(args).write_stdin(stdin).output().unwrap();
        assert!(
            out.status.success(),
            "trestle {args:?} failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn fails(&self, args: &[&str]) -> String {
        let out = self.cmd(args).output().unwrap();
        assert!(
            !out.status.success(),
            "trestle {args:?} succeeded:\n{}",
            String::from_utf8_lossy(&out.stdout)
        );
        String::from_utf8(out.stderr).unwrap()
    }

    fn json(&self, args: &[&str], kind: &str) -> Value {
        let doc: Value = serde_json::from_str(&self.ok(args)).unwrap();
        assert_eq!(doc["kind"], kind, "{doc}");
        doc["data"].clone()
    }

    fn neon_calls(&self) -> String {
        std::fs::read_to_string(self.path("neon.log")).unwrap_or_default()
    }
}

#[test]
fn two_machines_share_a_workspace_but_not_instances_or_local_secrets() {
    let Some(team) = Team::new() else { return };
    let (alice, bob) = (Machine::new(&team), Machine::new(&team));

    // Bob can't attach before anyone set the state up.
    let err = bob.fails(&["state", "attach", "--neon", "proj", "--name", "team"]);
    assert!(err.contains("trestle state init"), "{err}");
    assert!(bob.ok(&["state", "list"]).contains("no shared state"));

    // Alice sets it up: the database is made on the default branch, then attached.
    let out = alice.ok(&["state", "init", "--neon", "proj", "--name", "team"]);
    assert!(
        out.contains("attached neon:proj/trestle_state as `team`"),
        "{out}"
    );
    assert!(
        alice
            .neon_calls()
            .contains("databases create --name trestle_state")
    );
    assert!(
        alice
            .ok(&["state", "init", "--neon", "proj"])
            .contains("already attached here as `team`")
    );
    alice.ok(&["--state", "team", "ws", "create", "demo"]);
    alice.ok(&[
        "--state",
        "team",
        "env",
        "create",
        "demo/dev",
        "--select",
        "unity-catalog-oss",
    ]);
    alice.ok(&["--state", "team", "env", "commit", "demo/dev"]);
    alice.ok(&["--state", "team", "var", "set", "demo/dev", "LOG=info"]);
    // Her own store doesn't hold the team's workspace.
    assert_eq!(
        alice.json(&["ws", "list", "--json"], "workspaces"),
        serde_json::json!([])
    );

    // Bob attaches and sees it.
    bob.ok(&["state", "attach", "--neon", "proj", "--name", "team"]);
    let listed = bob.json(&["--state", "team", "ws", "list", "--json"], "workspaces");
    assert_eq!(listed[0]["name"], "demo", "{listed}");
    let states = bob.json(&["state", "list", "--json"], "states");
    assert_eq!(states[0]["location"]["neon"]["project"], "proj", "{states}");

    // A secret Bob sets on the team's environment stays on his machine.
    let out = bob.ok_with_stdin(
        &[
            "--state", "team", "var", "set", "demo/dev", "--secret", "API_KEY",
        ],
        "bobs-key",
    );
    assert!(out.contains("this machine only"), "{out}");
    let scopes = |m: &Machine| -> Vec<String> {
        m.json(
            &["--state", "team", "var", "list", "demo/dev", "--json"],
            "vars",
        )
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            format!(
                "{}:{}",
                r["name"].as_str().unwrap(),
                r["scope"].as_str().unwrap()
            )
        })
        .collect()
    };
    assert_eq!(scopes(&bob), ["LOG:environment", "API_KEY:local"]);
    assert_eq!(scopes(&alice), ["LOG:environment"]);
    // A workspace secret would be shared with keychains that lack it.
    let err = bob.fails(&["--state", "team", "var", "set", "demo", "--secret", "X"]);
    assert!(err.contains("--shared"), "{err}");

    // Bob runs it; the instance is his alone, and holds the attachment.
    bob.ok(&["--state", "team", "up", "demo/dev"]);
    assert_eq!(
        bob.json(&["--state", "team", "ps", "--json"], "instances")
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        alice
            .json(&["--state", "team", "ps", "--json"], "instances")
            .as_array()
            .unwrap()
            .is_empty()
    );
    let err = bob.fails(&["state", "detach", "team"]);
    assert!(
        err.contains("instance") && err.contains("--state team down"),
        "{err}"
    );

    // Alice's next revision is Bob's to run.
    alice.ok(&["--state", "team", "env", "add-module", "demo/dev", "jaeger"]);
    alice.ok(&["--state", "team", "env", "commit", "demo/dev"]);
    let revisions = bob.json(
        &["--state", "team", "env", "revisions", "demo/dev", "--json"],
        "revisions",
    );
    assert_eq!(revisions.as_array().unwrap().len(), 2, "{revisions}");

    bob.ok(&["--state", "team", "down", "demo/dev", "--rm"]);
    bob.ok(&["state", "detach", "team"]);
    assert!(bob.ok(&["state", "list"]).contains("no shared state"));
}
