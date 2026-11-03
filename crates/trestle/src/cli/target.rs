//! `<workspace>[/<env>][@<rev>]`: what a store-backed command acts on.

use std::fmt;
use std::str::FromStr;

use olai_stack_state::{Environment, Revision, Store, Workspace};

use crate::error::{Error, Result};

/// Which revision of an environment a target names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rev {
    /// The latest revision (`@head`, or no `@`).
    Head,
    /// The editable draft (`@draft`).
    Draft,
    /// Revision `n` (`@n`).
    Number(u32),
}

/// A workspace, an environment of it, and maybe a revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub workspace: String,
    pub env: Option<String>,
    pub rev: Option<Rev>,
}

impl FromStr for Target {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, String> {
        let bad = || format!("expected <workspace>[/<env>][@<rev>], got `{s}`");
        let (path, rev) = match s.split_once('@') {
            Some((path, rev)) => {
                let rev = match rev {
                    "head" => Rev::Head,
                    "draft" => Rev::Draft,
                    n => Rev::Number(n.parse().map_err(|_| bad())?),
                };
                (path, Some(rev))
            }
            None => (s, None),
        };
        let (workspace, env) = match path.split_once('/') {
            Some((ws, env)) => (ws, Some(env)),
            None => (path, None),
        };
        if workspace.is_empty() || env.is_some_and(|e| e.is_empty() || e.contains('/')) {
            return Err(bad());
        }
        if rev.is_some() && env.is_none() {
            return Err(format!(
                "`{s}`: a revision needs an environment (<ws>/<env>@<rev>)"
            ));
        }
        Ok(Target {
            workspace: workspace.to_owned(),
            env: env.map(str::to_owned),
            rev,
        })
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.workspace)?;
        if let Some(env) = &self.env {
            write!(f, "/{env}")?;
        }
        match self.rev {
            None => Ok(()),
            Some(Rev::Head) => f.write_str("@head"),
            Some(Rev::Draft) => f.write_str("@draft"),
            Some(Rev::Number(n)) => write!(f, "@{n}"),
        }
    }
}

impl Target {
    /// The workspace it names.
    pub fn workspace(&self, store: &Store) -> Result<Workspace> {
        find_workspace(store, &self.workspace)
    }

    /// The workspace and the environment it names; an error without one.
    pub fn environment(&self, store: &Store) -> Result<(Workspace, Environment)> {
        let ws = self.workspace(store)?;
        let Some(name) = &self.env else {
            return Err(Error::other(format!("`{self}` names no environment"))
                .with_hint(format!("`{}/<env>`", self.workspace)));
        };
        let envs = store.environments(&ws.id)?;
        match envs.iter().find(|e| &e.name == name) {
            Some(env) => Ok((ws, env.clone())),
            None => {
                let names: Vec<&str> = envs.iter().map(|e| e.name.as_str()).collect();
                Err(Error::other(format!(
                    "workspace `{}` has no environment `{name}`",
                    ws.name
                ))
                .with_hint(if names.is_empty() {
                    format!("`trestle env create {}/{name}`", ws.name)
                } else {
                    format!("one of: {}", names.join(", "))
                }))
            }
        }
    }

    /// A target that only names a workspace, or an error.
    pub fn workspace_only(&self) -> Result<&str> {
        if self.env.is_some() {
            return Err(Error::other(format!("`{self}`: expected a workspace only")));
        }
        Ok(&self.workspace)
    }

    /// Refuse a revision in the target (for commands that change something).
    pub fn no_rev(&self) -> Result<()> {
        if self.rev.is_some() {
            return Err(Error::other(format!(
                "`{self}`: revisions are immutable; drop the `@…`"
            )));
        }
        Ok(())
    }
}

/// The workspace called `name`, or an error listing the ones there are.
pub(super) fn find_workspace(store: &Store, name: &str) -> Result<Workspace> {
    if let Some(ws) = store.workspace(name)? {
        return Ok(ws);
    }
    let names: Vec<String> = store.workspaces()?.into_iter().map(|w| w.name).collect();
    Err(
        Error::other(format!("no workspace `{name}`")).with_hint(if names.is_empty() {
            format!("`trestle ws create {name}` or `trestle ws import <dir>`")
        } else {
            format!("one of: {}", names.join(", "))
        }),
    )
}

/// The revision `rev` names of `env` (`Head` when `None`); an error for the draft or an
/// environment without revisions.
pub(super) fn revision(store: &Store, env: &Environment, rev: Option<Rev>) -> Result<Revision> {
    let number = match rev.unwrap_or(Rev::Head) {
        Rev::Number(n) => n,
        Rev::Head => env.head.ok_or_else(|| {
            Error::other(format!("environment `{}` has no revision yet", env.name))
                .with_hint("`trestle env commit <ws>/<env>`")
        })?,
        Rev::Draft => {
            return Err(Error::other(
                "the draft is not a revision; name one (`@<n>`)",
            ));
        }
    };
    Ok(store.revision(&env.id, number)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_shape() {
        let t: Target = "demo".parse().unwrap();
        assert_eq!((t.env, t.rev), (None, None));
        let t: Target = "demo/dev".parse().unwrap();
        assert_eq!(t.env.as_deref(), Some("dev"));
        let t: Target = "demo/dev@3".parse().unwrap();
        assert_eq!(t.rev, Some(Rev::Number(3)));
        assert_eq!(t.to_string(), "demo/dev@3");
        let t: Target = "demo/dev@draft".parse().unwrap();
        assert_eq!(t.rev, Some(Rev::Draft));
        let t: Target = "demo/dev@head".parse().unwrap();
        assert_eq!(t.rev, Some(Rev::Head));
        for bad in ["", "/dev", "demo/", "demo/a/b", "demo@1", "demo/dev@x"] {
            assert!(bad.parse::<Target>().is_err(), "{bad}");
        }
    }
}
