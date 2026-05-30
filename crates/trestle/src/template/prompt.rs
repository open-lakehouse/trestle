//! Interactive (and non-interactive) variable collection.
//!
//! Three input channels feed the final variable map, in priority order:
//!
//! 1. `--set key=value` CLI overrides (highest priority).
//! 2. `--values <file.yaml>` (flat key→value map; also accepted in the new
//!    structured-values format with a `variables:` section — that's handled in
//!    the CLI layer before this function sees it).
//! 3. Interactive prompts (skipped when `--non-interactive`).
//! 4. Manifest defaults.
//!
//! In `--non-interactive` mode, a variable with no value from sources (1)–(2)
//! and no default is a hard error.
//!
//! The prompts use [`cliclack`] so they share the wizard's look-and-feel.

use std::collections::BTreeMap;
use std::path::Path;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

use super::manifest::{Manifest, VarKind, Variable};

/// A resolved variable value. Stored as a tagged JSON-style value so MiniJinja sees
/// proper bool/string typing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum VariableValue {
    String(String),
    Bool(bool),
}

impl VariableValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            VariableValue::String(s) => Some(s.as_str()),
            VariableValue::Bool(_) => None,
        }
    }

    fn from_yaml(v: &serde_yaml::Value) -> Option<Self> {
        match v {
            serde_yaml::Value::Bool(b) => Some(Self::Bool(*b)),
            serde_yaml::Value::String(s) => Some(Self::String(s.clone())),
            serde_yaml::Value::Number(n) => Some(Self::String(n.to_string())),
            _ => None,
        }
    }
}

/// Collect variable values from the configured input channels.
pub fn collect_variables(
    manifest: &Manifest,
    values_file: Option<&Path>,
    overrides: &BTreeMap<String, String>,
    non_interactive: bool,
) -> Result<BTreeMap<String, VariableValue>> {
    let mut from_file: BTreeMap<String, VariableValue> = BTreeMap::new();
    if let Some(path) = values_file {
        let bytes = std::fs::read(path).map_err(|e| Error::io_at(path, e))?;
        let raw: BTreeMap<String, serde_yaml::Value> =
            serde_yaml::from_slice(&bytes).map_err(|e| Error::yaml_at(path, e))?;
        // The structured format has top-level keys we don't want to treat as
        // variables (`apps`, `selections`, `variables`). If we see a
        // `variables:` block, prefer that; otherwise treat the whole map as a
        // flat name→value pairing (legacy format).
        if let Some(vars) = raw.get("variables") {
            if let serde_yaml::Value::Mapping(m) = vars {
                for (k, v) in m {
                    if let (serde_yaml::Value::String(name), Some(val)) =
                        (k, VariableValue::from_yaml(v))
                    {
                        from_file.insert(name.clone(), val);
                    }
                }
            }
        } else {
            for (k, v) in raw {
                if matches!(k.as_str(), "apps" | "selections") {
                    continue;
                }
                if let Some(val) = VariableValue::from_yaml(&v) {
                    from_file.insert(k, val);
                }
            }
        }
    }

    let mut out: BTreeMap<String, VariableValue> = BTreeMap::new();

    for var in &manifest.variables {
        // Priority: CLI override > values file > prompt > default.
        if let Some(s) = overrides.get(&var.name) {
            out.insert(var.name.clone(), coerce(var, s)?);
            continue;
        }
        if let Some(v) = from_file.get(&var.name) {
            out.insert(var.name.clone(), v.clone());
            continue;
        }
        if non_interactive {
            if let Some(d) = &var.default {
                if let Some(v) = VariableValue::from_yaml(d) {
                    out.insert(var.name.clone(), v);
                    continue;
                }
            }
            return Err(Error::MissingVariable {
                name: var.name.clone(),
            });
        }
        // Interactive: if the variable has a default and no prompt text, treat
        // it as a silent default (we follow the "opinionated defaults instead
        // of low-level prompts" principle). Variables without a `prompt:`
        // field never block the user with a question.
        if var.prompt.is_none() {
            if let Some(d) = &var.default {
                if let Some(v) = VariableValue::from_yaml(d) {
                    out.insert(var.name.clone(), v);
                    continue;
                }
            }
        }
        out.insert(var.name.clone(), prompt_for(var)?);
    }

    Ok(out)
}

fn coerce(var: &Variable, raw: &str) -> Result<VariableValue> {
    match var.kind {
        VarKind::Bool => match raw.to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" | "y" => Ok(VariableValue::Bool(true)),
            "false" | "no" | "0" | "n" => Ok(VariableValue::Bool(false)),
            other => Err(Error::InvalidVariable {
                name: var.name.clone(),
                reason: format!("expected bool, got `{other}`"),
            }),
        },
        VarKind::Enum => {
            if var.options.iter().any(|o| o == raw) {
                Ok(VariableValue::String(raw.to_string()))
            } else {
                Err(Error::InvalidVariable {
                    name: var.name.clone(),
                    reason: format!("must be one of {:?}", var.options),
                })
            }
        }
        VarKind::String => {
            if let Some(rx) = &var.validate {
                let re = Regex::new(rx).map_err(|e| Error::InvalidVariable {
                    name: var.name.clone(),
                    reason: format!("bad validate regex `{rx}`: {e}"),
                })?;
                if !re.is_match(raw) {
                    return Err(Error::InvalidVariable {
                        name: var.name.clone(),
                        reason: format!("does not match `{rx}`"),
                    });
                }
            }
            Ok(VariableValue::String(raw.to_string()))
        }
    }
}

fn prompt_for(var: &Variable) -> Result<VariableValue> {
    let prompt_text = var
        .prompt
        .clone()
        .unwrap_or_else(|| format!("{}?", var.name));

    match var.kind {
        VarKind::Bool => {
            let mut c = cliclack::confirm(prompt_text);
            if let Some(d) = &var.default {
                if let Some(b) = d.as_bool() {
                    c = c.initial_value(b);
                }
            }
            let b = c.interact().map_err(|e| io_err("confirm", e))?;
            Ok(VariableValue::Bool(b))
        }
        VarKind::Enum => {
            if var.options.is_empty() {
                return Err(Error::Manifest(format!(
                    "variable `{}` is type=enum but has no options",
                    var.name
                )));
            }
            let items: Vec<(String, String, String)> = var
                .options
                .iter()
                .map(|o| (o.clone(), o.clone(), String::new()))
                .collect();
            let mut s = cliclack::select(prompt_text).items(&items);
            if let Some(d) = &var.default {
                if let Some(v) = d.as_str() {
                    s = s.initial_value(v.to_string());
                }
            }
            let picked = s.interact().map_err(|e| io_err("select", e))?;
            Ok(VariableValue::String(picked))
        }
        VarKind::String => {
            let mut t = cliclack::input(prompt_text);
            let default_str = var
                .default
                .as_ref()
                .and_then(|d| d.as_str().map(String::from));
            if let Some(d) = default_str.as_deref() {
                t = t.default_input(d);
            }
            if let Some(rx) = &var.validate {
                let var_name = var.name.clone();
                let regex_src = rx.clone();
                let compiled = Regex::new(rx).map_err(|e| Error::InvalidVariable {
                    name: var.name.clone(),
                    reason: format!("bad validate regex `{rx}`: {e}"),
                })?;
                t = t.validate(move |input: &String| {
                    if compiled.is_match(input) {
                        Ok(())
                    } else {
                        Err(format!("`{var_name}` must match `{regex_src}`"))
                    }
                });
            }
            let s: String = t.interact().map_err(|e| io_err("input", e))?;
            Ok(VariableValue::String(s))
        }
    }
}

fn io_err(kind: &str, e: std::io::Error) -> Error {
    Error::other(format!("{kind} prompt failed: {e}"))
}
