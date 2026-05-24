//! Interactive (and non-interactive) variable collection.
//!
//! Three input channels feed the final variable map, in priority order:
//!
//! 1. `--values <file.yaml>` (highest priority)
//! 2. Interactive prompts (skipped when `--non-interactive`)
//! 3. Manifest defaults
//!
//! In `--non-interactive` mode, a variable with no value from sources (1) or (3) is a
//! hard error.

use std::collections::BTreeMap;
use std::path::Path;

use inquire::{Confirm, Select, Text, validator::StringValidator};
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

/// Collect variable values from the three input channels.
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
        for (k, v) in raw {
            if let Some(val) = VariableValue::from_yaml(&v) {
                from_file.insert(k, val);
            }
        }
    }

    let mut out: BTreeMap<String, VariableValue> = BTreeMap::new();

    for var in &manifest.variables {
        // Priority: CLI override > values file > prompt > default
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
            let mut c = Confirm::new(&prompt_text);
            if let Some(d) = &var.default {
                if let Some(b) = d.as_bool() {
                    c = c.with_default(b);
                }
            }
            if let Some(h) = &var.help {
                c = c.with_help_message(h);
            }
            let b = c
                .prompt()
                .map_err(|e| Error::other(format!("prompt failed: {e}")))?;
            Ok(VariableValue::Bool(b))
        }
        VarKind::Enum => {
            let options: Vec<&str> = var.options.iter().map(String::as_str).collect();
            if options.is_empty() {
                return Err(Error::Manifest(format!(
                    "variable `{}` is type=enum but has no options",
                    var.name
                )));
            }
            let mut s = Select::new(&prompt_text, options.clone());
            if let Some(d) = &var.default {
                if let Some(v) = d.as_str() {
                    if let Some(idx) = options.iter().position(|o| *o == v) {
                        s = s.with_starting_cursor(idx);
                    }
                }
            }
            if let Some(h) = &var.help {
                s = s.with_help_message(h);
            }
            let picked = s
                .prompt()
                .map_err(|e| Error::other(format!("prompt failed: {e}")))?;
            Ok(VariableValue::String(picked.to_string()))
        }
        VarKind::String => {
            let mut t = Text::new(&prompt_text);
            let default_str = var
                .default
                .as_ref()
                .and_then(|d| d.as_str().map(String::from));
            if let Some(d) = default_str.as_deref() {
                t = t.with_default(d);
            }
            if let Some(h) = &var.help {
                t = t.with_help_message(h);
            }
            if let Some(rx) = &var.validate {
                let validator = RegexValidator::new(&var.name, rx)?;
                t = t.with_validator(validator);
            }
            let s = t
                .prompt()
                .map_err(|e| Error::other(format!("prompt failed: {e}")))?;
            Ok(VariableValue::String(s))
        }
    }
}

#[derive(Clone)]
struct RegexValidator {
    var_name: String,
    raw: String,
    re: Regex,
}

impl RegexValidator {
    fn new(var_name: &str, raw: &str) -> Result<Self> {
        let re = Regex::new(raw).map_err(|e| Error::InvalidVariable {
            name: var_name.to_string(),
            reason: format!("bad validate regex `{raw}`: {e}"),
        })?;
        Ok(Self {
            var_name: var_name.to_string(),
            raw: raw.to_string(),
            re,
        })
    }
}

impl StringValidator for RegexValidator {
    fn validate(
        &self,
        input: &str,
    ) -> std::result::Result<inquire::validator::Validation, inquire::CustomUserError> {
        if self.re.is_match(input) {
            Ok(inquire::validator::Validation::Valid)
        } else {
            Ok(inquire::validator::Validation::Invalid(
                format!("`{}` must match `{}`", self.var_name, self.raw).into(),
            ))
        }
    }
}
