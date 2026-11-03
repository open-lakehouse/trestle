//! [`NeonCli`]: the [`NeonControl`] that runs the Neon CLI.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use async_trait::async_trait;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tokio::process::Command;

use super::{
    Branch, Connection, CreateBranch, Function, NeonControl, NeonError, Storage, StorageCredential,
};
use crate::secrets::SecretValue;

/// The environment variable naming the Neon CLI to run, ahead of every other place it is
/// looked for.
pub const NEON_CLI_VAR: &str = "NEON_CLI";

/// Where the Neon CLI is, in order: `explicit`, `$NEON_CLI`, `neon` on `PATH`, the Homebrew
/// prefixes and `~/.npm-global/bin`, a login shell's `command -v neon`, then the same for
/// `neonctl`. An app started from the Finder has launchd's `PATH`, which holds none of them,
/// hence the fixed places and the login shell.
pub fn locate_neon(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return path.is_file().then(|| path.to_owned());
    }
    if let Some(path) = std::env::var_os(NEON_CLI_VAR).map(PathBuf::from)
        && path.is_file()
    {
        return Some(path);
    }
    ["neon", "neonctl"].into_iter().find_map(find_program)
}

fn find_program(name: &str) -> Option<PathBuf> {
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|dir| dir.join(name));
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let fixed = ["/opt/homebrew/bin", "/usr/local/bin"]
        .into_iter()
        .map(PathBuf::from)
        .chain(home.map(|h| h.join(".npm-global/bin")))
        .map(|dir| dir.join(name));
    on_path
        .chain(fixed)
        .find(|p| p.is_file())
        .or_else(|| login_shell_lookup(name))
}

/// `command -v <name>` in the user's login shell, which reads their profile's `PATH`.
fn login_shell_lookup(name: &str) -> Option<PathBuf> {
    let shell = std::env::var_os("SHELL")?;
    let out = std::process::Command::new(shell)
        .args(["-lc", &format!("command -v {name}")])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let path = PathBuf::from(String::from_utf8(out.stdout).ok()?.trim());
    (out.status.success() && path.is_file()).then_some(path)
}

/// The Neon CLI (`neon`, or `neonctl`), run with `--output json`. Signed in the way the CLI is
/// (`~/.config/neon`, shared with the terminal) or with `NEON_API_KEY`.
#[derive(Clone, Debug)]
pub struct NeonCli {
    program: PathBuf,
}

impl NeonCli {
    /// The CLI [`locate_neon`] finds; [`NeonError::NotInstalled`] when there is none.
    pub fn new() -> Result<Self, NeonError> {
        locate_neon(None)
            .map(Self::with_program)
            .ok_or(NeonError::NotInstalled)
    }

    /// The CLI at `program`.
    pub fn with_program(program: impl Into<PathBuf>) -> Self {
        NeonCli {
            program: program.into(),
        }
    }

    /// The program it runs.
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// Run `neon <args> --output json` and return its stdout.
    async fn run(&self, args: &[&str]) -> Result<String, NeonError> {
        let command = args.join(" ");
        let out = Command::new(&self.program)
            .args(args)
            .args(["--output", "json"])
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|source| match source.kind() {
                std::io::ErrorKind::NotFound => NeonError::NotInstalled,
                _ => NeonError::Io {
                    command: command.clone(),
                    source,
                },
            })?;
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        if !out.status.success() {
            return Err(if is_auth_error(&stderr) {
                NeonError::NotAuthenticated(stderr)
            } else {
                NeonError::Failed { command, stderr }
            });
        }
        String::from_utf8(out.stdout).map_err(|e| NeonError::Parse {
            command,
            reason: e.to_string(),
        })
    }

    /// Run `args` and parse the first JSON value it prints.
    async fn json<T: DeserializeOwned>(&self, args: &[&str]) -> Result<T, NeonError> {
        let stdout = self.run(args).await?;
        first_json(&stdout).map_err(|reason| NeonError::Parse {
            command: args.join(" "),
            reason,
        })
    }
}

/// Whether `stderr` says the CLI isn't signed in.
fn is_auth_error(stderr: &str) -> bool {
    let s = stderr.to_ascii_lowercase();
    [
        "unauthorized",
        "not authenticated",
        "authentication",
        "401",
        "neon auth",
    ]
    .iter()
    .any(|needle| s.contains(needle))
}

/// The first JSON value in `text`, which several verbs surround with lines of text (a
/// `WARNING:` after `credentials create`, progress lines around `functions deploy`).
pub(super) fn first_json<T: DeserializeOwned>(text: &str) -> Result<T, String> {
    let mut last = String::from("no JSON in the output");
    for (at, _) in text
        .match_indices(['{', '['])
        .filter(|(at, _)| *at == 0 || text[..*at].ends_with('\n'))
    {
        let mut values = serde_json::Deserializer::from_str(&text[at..]).into_iter::<T>();
        match values.next() {
            Some(Ok(value)) => return Ok(value),
            Some(Err(e)) => last = e.to_string(),
            None => {}
        }
    }
    Err(last)
}

/// `GET /projects/<p>/branches`: the branches, and their annotations keyed by branch id.
#[derive(Deserialize)]
pub(super) struct ApiBranches {
    pub branches: Vec<Branch>,
    #[serde(default)]
    pub annotations: BTreeMap<String, ApiAnnotation>,
}

#[derive(Deserialize)]
pub(super) struct ApiAnnotation {
    #[serde(default)]
    pub value: BTreeMap<String, String>,
}

impl ApiBranches {
    /// The branches with their annotations filled in.
    pub fn into_branches(mut self) -> Vec<Branch> {
        for branch in &mut self.branches {
            if let Some(a) = self.annotations.remove(&branch.id) {
                branch.annotations = a.value;
            }
        }
        self.branches
    }
}

/// `connection-string --extended`.
#[derive(Deserialize)]
pub(super) struct Extended {
    pub connection_string: String,
    pub host: String,
    pub role: String,
    pub password: String,
    pub database: String,
    #[serde(default)]
    pub options: String,
}

impl From<Extended> for Connection {
    fn from(e: Extended) -> Self {
        Connection {
            url: SecretValue::new(e.connection_string),
            host: e.host,
            role: e.role,
            password: SecretValue::new(e.password),
            database: e.database,
            options: e.options,
        }
    }
}

/// `credentials create`.
#[derive(Deserialize)]
pub(super) struct CreatedCredential {
    pub token_id: String,
    pub s3_secret_access_key: String,
}

#[derive(Deserialize)]
struct Named {
    name: String,
}

/// A database of `databases list`.
#[derive(Deserialize)]
struct Database {
    name: String,
    owner_name: String,
}

/// The database Neon creates with a project. `connection-string` and `databases create` only
/// pick a default while the branch has one database and one role; a branch that hosts more
/// (another service's database, a read-only role) needs both named.
const DEFAULT_DATABASE: &str = "neondb";

impl NeonCli {
    async fn database_list(&self, project: &str, branch: &str) -> Result<Vec<Database>, NeonError> {
        self.json(&[
            "databases",
            "list",
            "--project-id",
            project,
            "--branch",
            branch,
        ])
        .await
    }

    /// The branch's main database: `neondb` when it has one, else the first.
    async fn main_database(&self, project: &str, branch: &str) -> Result<Database, NeonError> {
        let mut dbs = self.database_list(project, branch).await?;
        if dbs.is_empty() {
            return Err(NeonError::Failed {
                command: "databases list".into(),
                stderr: format!("branch {branch} has no database"),
            });
        }
        let at = dbs
            .iter()
            .position(|d| d.name == DEFAULT_DATABASE)
            .unwrap_or(0);
        Ok(dbs.swap_remove(at))
    }
}

#[async_trait]
impl NeonControl for NeonCli {
    async fn auth_status(&self) -> Result<String, NeonError> {
        #[derive(Deserialize)]
        struct Me {
            #[serde(default)]
            login: Option<String>,
            #[serde(default)]
            email: Option<String>,
        }
        let me: Me = self.json(&["me"]).await?;
        Ok(me.login.or(me.email).unwrap_or_default())
    }

    async fn branches(&self, project: &str) -> Result<Vec<Branch>, NeonError> {
        let path = format!("/projects/{project}/branches");
        let api: ApiBranches = self.json(&["api", &path]).await?;
        Ok(api.into_branches())
    }

    async fn create_branch(&self, project: &str, spec: &CreateBranch) -> Result<Branch, NeonError> {
        #[derive(Deserialize)]
        struct Created {
            branch: Branch,
        }
        let annotations = serde_json::to_string(&spec.annotations).unwrap_or_default();
        let mut args = vec![
            "branches",
            "create",
            "--project-id",
            project,
            "--name",
            &spec.name,
            "--no-secrets",
        ];
        if let Some(parent) = &spec.parent {
            args.extend(["--parent", parent]);
        }
        if let Some(at) = &spec.expires_at {
            args.extend(["--expires-at", at]);
        }
        if !spec.annotations.is_empty() {
            args.extend(["--annotation", &annotations]);
        }
        let mut created: Created = self.json(&args).await?;
        // The CLI stores annotations but doesn't print them back.
        created.branch.annotations = spec.annotations.clone();
        Ok(created.branch)
    }

    async fn delete_branch(&self, project: &str, id: &str) -> Result<(), NeonError> {
        self.run(&["branches", "delete", id, "--project-id", project])
            .await
            .map(drop)
    }

    async fn reset_branch(&self, project: &str, id: &str) -> Result<(), NeonError> {
        self.run(&["branches", "reset", id, "--parent", "--project-id", project])
            .await
            .map(drop)
    }

    async fn connection(
        &self,
        project: &str,
        branch: &str,
        database: Option<&str>,
        pooled: bool,
    ) -> Result<Connection, NeonError> {
        let mut args = vec![
            "connection-string",
            branch,
            "--project-id",
            project,
            "--extended",
        ];
        if pooled {
            args.push("--pooled");
        }
        let main;
        let db = match database {
            Some(db) => db,
            None => {
                main = self.main_database(project, branch).await?;
                main.name.as_str()
            }
        };
        args.extend(["--database-name", db]);
        let extended: Extended = self.json(&args).await?;
        Ok(extended.into())
    }

    async fn databases(&self, project: &str, branch: &str) -> Result<Vec<String>, NeonError> {
        let dbs = self.database_list(project, branch).await?;
        Ok(dbs.into_iter().map(|d| d.name).collect())
    }

    async fn create_database(
        &self,
        project: &str,
        branch: &str,
        name: &str,
    ) -> Result<(), NeonError> {
        // Owned like the main database, by the role `connection-string` connects as.
        let owner = self.main_database(project, branch).await?.owner_name;
        self.run(&[
            "databases",
            "create",
            "--name",
            name,
            "--owner-name",
            &owner,
            "--project-id",
            project,
            "--branch",
            branch,
        ])
        .await
        .map(drop)
    }

    async fn functions(&self, project: &str, branch: &str) -> Result<Vec<Function>, NeonError> {
        self.json(&[
            "functions",
            "list",
            "--project-id",
            project,
            "--branch",
            branch,
        ])
        .await
    }

    async fn storage(&self, project: &str, branch: &str) -> Result<Storage, NeonError> {
        let path = format!("/projects/{project}/branches/{branch}/storage");
        self.json(&["api", &path]).await
    }

    async fn buckets(&self, project: &str, branch: &str) -> Result<Vec<String>, NeonError> {
        let buckets: Vec<Named> = self
            .json(&[
                "buckets",
                "list",
                "--project-id",
                project,
                "--branch",
                branch,
            ])
            .await?;
        Ok(buckets.into_iter().map(|b| b.name).collect())
    }

    async fn create_bucket(
        &self,
        project: &str,
        branch: &str,
        name: &str,
    ) -> Result<(), NeonError> {
        // Prints a sentence, not JSON.
        self.run(&[
            "buckets",
            "create",
            name,
            "--project-id",
            project,
            "--branch",
            branch,
        ])
        .await
        .map(drop)
    }

    async fn create_credential(
        &self,
        project: &str,
        branch: &str,
        name: &str,
    ) -> Result<StorageCredential, NeonError> {
        let created: CreatedCredential = self
            .json(&[
                "credentials",
                "create",
                "--project-id",
                project,
                "--branch",
                branch,
                "--name",
                name,
                "--scope",
                "storage:read",
                "--scope",
                "storage:write",
            ])
            .await?;
        Ok(StorageCredential {
            token_id: created.token_id,
            secret_access_key: SecretValue::new(created.s3_secret_access_key),
        })
    }

    async fn revoke_credential(
        &self,
        project: &str,
        branch: &str,
        token_id: &str,
    ) -> Result<(), NeonError> {
        self.run(&[
            "credentials",
            "revoke",
            token_id,
            "--project-id",
            project,
            "--branch",
            branch,
        ])
        .await
        .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_json_value_wins_around_text() {
        let v: serde_json::Value =
            first_json("Function deployment triggered.\n{\"id\": 1}\nDone.\n").unwrap();
        assert_eq!(v["id"], 1);
        let v: serde_json::Value = first_json("{\"a\": [1]}\nWARNING: store it\n").unwrap();
        assert_eq!(v["a"][0], 1);
        // A `{` inside a line of text isn't the start of a value.
        let v: serde_json::Value = first_json("note {curly}\n[2]").unwrap();
        assert_eq!(v[0], 2);
        assert!(first_json::<serde_json::Value>("Bucket created").is_err());
    }

    #[test]
    fn auth_errors_are_told_apart() {
        assert!(is_auth_error("ERROR: Unauthorized. Run `neon auth`"));
        assert!(!is_auth_error("ERROR: branch not found"));
    }
}
