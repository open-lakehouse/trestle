//! Rendering without credentials on disk: [`MaterializedOutput::into_launch_env`].
//!
//! A materialized environment keeps its credentials in sensitive, git-ignored files. A runtime
//! that renders into a cache directory and starts Compose itself can keep them out of the tree
//! altogether: every sensitive value moves into a [`LaunchEnv`] that the runtime passes to
//! Compose as process environment, and the files keep only `${VAR}` references, which Compose
//! interpolates. Each kind of sensitive file is handled generically:
//!
//! - a module **env file** (`modules/<m>/.env/<name>`): each `KEY=value` line becomes
//!   `KEY=${VAR}`, and the fragments referencing it drop its `format: raw` (Compose
//!   interpolates env-file contents unless the format is raw; with every value a reference,
//!   raw protects nothing);
//! - a file the head `compose.yaml` declares as a **secret or config** (`file: ./<path>`): the
//!   declaration becomes `environment: VAR` and the file is dropped (Compose delivers the
//!   variable's value as the file's contents);
//! - the root **`.env`** substitution overlay: dropped, its values passed directly.
//!
//! Any other sensitive file is an error rather than left behind.

use std::collections::BTreeMap;
use std::fmt;

use super::output::MaterializedOutput;

/// The variables a [`MaterializedOutput::into_launch_env`] output needs in Compose's process
/// environment. Holds credentials: its `Debug` prints names only.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct LaunchEnv(BTreeMap<String, String>);

impl fmt::Debug for LaunchEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.0.keys()).finish()
    }
}

impl LaunchEnv {
    /// The variables, by name.
    pub fn vars(&self) -> &BTreeMap<String, String> {
        &self.0
    }

    /// The variables, by name, owned.
    pub fn into_vars(self) -> BTreeMap<String, String> {
        self.0
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn insert(&mut self, name: String, value: String) -> Result<(), LaunchEnvError> {
        match self.0.get(&name) {
            Some(existing) if *existing != value => Err(LaunchEnvError::Clash(name)),
            _ => {
                self.0.insert(name, value);
                Ok(())
            }
        }
    }
}

/// Why an output can't be rendered without credentials on disk.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LaunchEnvError {
    /// A sensitive file is neither an env file, a declared secret/config nor the `.env`
    /// overlay, so there is no way to hand its contents to Compose without writing it.
    #[error("`{0}` holds credentials but is not an env file or a compose secret/config")]
    Unsupported(String),
    /// Two values would go under one variable name.
    #[error("two different values for the launch variable `{0}`")]
    Clash(String),
}

/// `s` upper-cased, with every character outside `[A-Z0-9]` mapped to `_`.
fn var_part(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

impl MaterializedOutput {
    /// This output with every sensitive value moved into the returned [`LaunchEnv`], so no
    /// file holds a credential (see the [module docs](self)). The result runs only with the
    /// launch variables in Compose's environment.
    pub fn into_launch_env(mut self) -> Result<(MaterializedOutput, LaunchEnv), LaunchEnvError> {
        let mut env = LaunchEnv::default();
        let head_idx = self
            .files
            .iter()
            .position(|f| f.path == "compose.yaml")
            .expect("a materialized output has a head compose.yaml");
        let mut head = std::mem::take(&mut self.files[head_idx].contents);
        let mut kept = Vec::with_capacity(self.files.len());
        // The `env_file` entries whose `format: raw` must go.
        let mut raw_env_files: Vec<String> = Vec::new();
        for mut file in std::mem::take(&mut self.files) {
            if !file.sensitive {
                kept.push(file);
                continue;
            }
            if file.path == ".env" {
                for (key, value) in env_lines(&file.contents) {
                    env.insert(key.to_owned(), value.to_owned())?;
                }
                continue;
            }
            let declaration = format!("    file: ./{}", file.path);
            if head.lines().any(|l| l == declaration) {
                let var = format!("TRESTLE_FILE_{}", var_part(&file.path));
                head = head
                    .lines()
                    .map(|l| {
                        if l == declaration {
                            format!("    environment: {var}")
                        } else {
                            l.to_owned()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
                    + "\n";
                env.insert(var, file.contents)?;
                continue;
            }
            if let Some(name) = env_file_name(&file.path) {
                let prefix = format!("TRESTLE_ENV_{}_", var_part(&name));
                let mut out = String::with_capacity(file.contents.len());
                for line in file.contents.lines() {
                    match split_env_line(line) {
                        Some((key, value)) => {
                            let var = format!("{prefix}{}", var_part(key));
                            out.push_str(&format!("{key}=${{{var}}}\n"));
                            env.insert(var, value.to_owned())?;
                        }
                        None => {
                            out.push_str(line);
                            out.push('\n');
                        }
                    }
                }
                file.contents = out;
                file.sensitive = false;
                raw_env_files.push(format!("- path: ./{}", file.path));
                kept.push(file);
                continue;
            }
            return Err(LaunchEnvError::Unsupported(file.path));
        }
        let head_file = kept
            .iter_mut()
            .find(|f| f.path == "compose.yaml")
            .expect("the head compose.yaml is not sensitive");
        head_file.contents = head;
        if !raw_env_files.is_empty() {
            for file in kept.iter_mut().filter(|f| f.path.ends_with("compose.yaml")) {
                file.contents = interpolate_env_files(&file.contents, &raw_env_files);
            }
        }
        self.files = kept;
        Ok((self, env))
    }
}

/// `fragment` with the `format: raw` line dropped right after each `env_file` entry in
/// `entries` (`- path: ./<file>`), so Compose interpolates those files.
fn interpolate_env_files(fragment: &str, entries: &[String]) -> String {
    let mut out = String::with_capacity(fragment.len());
    let mut after_entry = false;
    for line in fragment.lines() {
        if after_entry && line.trim() == "format: raw" {
            after_entry = false;
            continue;
        }
        after_entry = entries.iter().any(|e| line.trim() == e);
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// `modules/<m>/.env/<file>` → `<m>_<file>`.
fn env_file_name(path: &str) -> Option<String> {
    let rest = path.strip_prefix("modules/")?;
    let (module, file) = rest.split_once("/.env/")?;
    Some(format!("{module}_{file}"))
}

/// A `KEY=value` line (not a comment or blank).
fn split_env_line(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let (key, value) = line.split_once('=')?;
    let key = key.trim();
    (!key.is_empty() && !key.contains(char::is_whitespace)).then_some((key, value))
}

fn env_lines(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines().filter_map(split_env_line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::output::OutputFile;

    fn file(path: &str, contents: &str, sensitive: bool) -> OutputFile {
        OutputFile {
            sensitive,
            ..OutputFile::plain(path, contents)
        }
    }

    #[test]
    fn each_kind_of_sensitive_file_moves_into_the_env() {
        let out = MaterializedOutput {
            files: vec![
                file(
                    "compose.yaml",
                    "name: x\n\nsecrets:\n  pw:\n    file: ./modules/db/secrets/pw\n",
                    false,
                ),
                file(".env", "# overlay\nTOKEN=t0k\n", true),
                file("modules/db/secrets/pw", "hunter2", true),
                file(
                    "modules/app/.env/app.env",
                    "# c\nPASS=p=ss\nMODE=dev\n",
                    true,
                ),
                file(
                    "modules/app/compose.yaml",
                    "services:\n  app:\n    env_file:\n      - path: ./modules/app/.env/app.env\n        format: raw\n      - path: ./modules/app/plain.env\n        format: raw\n",
                    false,
                ),
            ],
            deploy: Vec::new(),
        };
        let (out, env) = out.into_launch_env().unwrap();
        assert!(out.files.iter().all(|f| !f.sensitive));
        assert!(
            !out.files
                .iter()
                .any(|f| f.path == ".env" || f.path.ends_with("/pw"))
        );
        let head = &out
            .files
            .iter()
            .find(|f| f.path == "compose.yaml")
            .unwrap()
            .contents;
        assert!(
            head.contains("    environment: TRESTLE_FILE_MODULES_DB_SECRETS_PW"),
            "{head}"
        );
        let app = &out
            .files
            .iter()
            .find(|f| f.path == "modules/app/.env/app.env")
            .unwrap()
            .contents;
        assert_eq!(
            app,
            "# c\nPASS=${TRESTLE_ENV_APP_APP_ENV_PASS}\nMODE=${TRESTLE_ENV_APP_APP_ENV_MODE}\n"
        );
        let fragment = &out
            .files
            .iter()
            .find(|f| f.path == "modules/app/compose.yaml")
            .unwrap()
            .contents;
        // The rewritten env file is interpolated now; a plain one stays raw.
        assert_eq!(
            fragment,
            "services:\n  app:\n    env_file:\n      - path: ./modules/app/.env/app.env\n      - path: ./modules/app/plain.env\n        format: raw\n"
        );
        let vars = env.vars();
        assert_eq!(vars["TOKEN"], "t0k");
        assert_eq!(vars["TRESTLE_FILE_MODULES_DB_SECRETS_PW"], "hunter2");
        assert_eq!(vars["TRESTLE_ENV_APP_APP_ENV_PASS"], "p=ss");
        assert!(!format!("{env:?}").contains("hunter2"));
    }

    #[test]
    fn an_unknown_sensitive_file_is_refused() {
        let out = MaterializedOutput {
            files: vec![
                file("compose.yaml", "name: x\n", false),
                file("modules/app/key.pem", "secret", true),
            ],
            deploy: Vec::new(),
        };
        assert_eq!(
            out.into_launch_env().unwrap_err(),
            LaunchEnvError::Unsupported("modules/app/key.pem".into())
        );
    }
}
