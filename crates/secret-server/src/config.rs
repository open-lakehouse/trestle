//! `secret-server.yaml`: the listeners, their consumers, and which variable holds each
//! secret and each keyring.
//!
//! The file is not secret: it names environment variables, never values. Values are read
//! from the process environment once, by [`Config::resolve`].

use std::collections::HashSet;
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::caller::CallerId;
use crate::keyring::{KeyHandle, Keyring, MAX_KEY_NAME_LEN};
use crate::policy::Policy;
use crate::secret::SecretValue;

/// The only configuration version this server reads.
pub const CONFIG_VERSION: u32 = 1;

/// The parsed `secret-server.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The file format version. Must be [`CONFIG_VERSION`].
    pub version: u32,
    /// One listener per consumer.
    pub listeners: Vec<ListenerConfig>,
}

/// One Unix socket, the consumer it serves, the secrets that consumer may see, and the keys it
/// may wrap and unwrap under.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListenerConfig {
    /// The socket path the server binds.
    pub socket: PathBuf,
    /// The consumer every connection on this socket is answered for.
    pub consumer: String,
    /// The secrets the consumer may see.
    #[serde(default)]
    pub secrets: Vec<SecretRef>,
    /// The keys the consumer may use through the key service.
    #[serde(default)]
    pub keys: Vec<KeyRef>,
}

/// A secret by the name a consumer asks for, and the variable that holds its value.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretRef {
    /// The resource name the consumer asks for (Envoy's `sds_config` secret name).
    pub name: String,
    /// The environment variable that holds the value.
    pub env: String,
}

/// A key by the name a consumer uses, and the variable that holds its keyring (JSON; see
/// [`Keyring::from_json`]).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyRef {
    /// The key name the consumer asks for. Its versions' KEK ids are `<name>@v<n>`.
    pub name: String,
    /// The environment variable that holds the keyring.
    pub env: String,
}

/// Why a configuration could not be loaded or resolved. Messages name files, consumers,
/// secrets and variables, never values.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file could not be read.
    #[error("cannot read {path}: {source}")]
    Read {
        /// The file.
        path: PathBuf,
        /// The I/O error.
        source: std::io::Error,
    },
    /// The file is not valid YAML for this format.
    #[error("cannot parse {path}: {source}")]
    Parse {
        /// The file.
        path: PathBuf,
        /// The parse error.
        source: serde_yaml::Error,
    },
    /// The file parsed but is inconsistent.
    #[error("invalid secret server configuration: {0}")]
    Invalid(String),
    /// Variables the file lists are unset or empty.
    #[error("{}", MissingDisplay(.0))]
    MissingVariables(Vec<MissingVariable>),
    /// Keyring variables are set but don't hold a valid keyring.
    #[error("{}", InvalidKeyringsDisplay(.0))]
    InvalidKeyrings(Vec<InvalidKeyring>),
}

/// A key whose variable doesn't hold a valid keyring. The reason never quotes the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidKeyring {
    /// The consumer the key belongs to.
    pub consumer: String,
    /// The key's name.
    pub name: String,
    /// The variable.
    pub env: String,
    /// What is wrong with it.
    pub reason: String,
}

struct InvalidKeyringsDisplay<'a>(&'a [InvalidKeyring]);

impl fmt::Display for InvalidKeyringsDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "keyrings that cannot be read:")?;
        for k in self.0 {
            write!(
                f,
                "\n  - key {} (consumer {}): ${}: {}",
                k.name, k.consumer, k.env, k.reason
            )?;
        }
        Ok(())
    }
}

/// A secret or keyring whose variable is unset or empty in the process environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingVariable {
    /// The consumer the secret or key belongs to.
    pub consumer: String,
    /// The secret's or key's name.
    pub name: String,
    /// The variable.
    pub env: String,
    /// `true` when the variable is set but empty.
    pub empty: bool,
}

struct MissingDisplay<'a>(&'a [MissingVariable]);

impl fmt::Display for MissingDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "secrets and keyrings without a value in the environment:"
        )?;
        for m in self.0 {
            let state = if m.empty { "is empty" } else { "is not set" };
            write!(
                f,
                "\n  - {} (consumer {}): ${} {state}",
                m.name, m.consumer, m.env
            )?;
        }
        Ok(())
    }
}

impl Config {
    /// Reads and validates the file at `path`.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let config: Self = serde_yaml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        config.validate()?;
        Ok(config)
    }

    /// Parses and validates a configuration from a string.
    pub fn from_yaml(text: &str) -> Result<Self, ConfigError> {
        let config: Self = serde_yaml::from_str(text).map_err(|source| ConfigError::Parse {
            path: PathBuf::from("<inline>"),
            source,
        })?;
        config.validate()?;
        Ok(config)
    }

    /// Checks the version, and that consumers, sockets and each listener's secret and key
    /// names are unique and non-empty.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let invalid = |msg: String| Err(ConfigError::Invalid(msg));
        if self.version != CONFIG_VERSION {
            return invalid(format!(
                "unsupported version {} (this server reads version {CONFIG_VERSION})",
                self.version
            ));
        }
        if self.listeners.is_empty() {
            return invalid("no listeners".into());
        }
        let mut consumers = HashSet::new();
        let mut sockets = HashSet::new();
        for listener in &self.listeners {
            if listener.consumer.trim().is_empty() {
                return invalid("a listener has an empty consumer".into());
            }
            if listener.socket.as_os_str().is_empty() {
                return invalid(format!(
                    "consumer {} has an empty socket",
                    listener.consumer
                ));
            }
            if !consumers.insert(listener.consumer.as_str()) {
                return invalid(format!(
                    "consumer {} has more than one listener",
                    listener.consumer
                ));
            }
            if !sockets.insert(listener.socket.as_path()) {
                return invalid(format!(
                    "socket {} is used by more than one listener",
                    listener.socket.display()
                ));
            }
            let mut names = HashSet::new();
            for secret in &listener.secrets {
                if secret.name.is_empty() || secret.env.is_empty() {
                    return invalid(format!(
                        "consumer {} has a secret with an empty name or env",
                        listener.consumer
                    ));
                }
                if !names.insert(secret.name.as_str()) {
                    return invalid(format!(
                        "consumer {} lists secret {} more than once",
                        listener.consumer, secret.name
                    ));
                }
            }
            let mut keys = HashSet::new();
            for key in &listener.keys {
                if key.name.is_empty() || key.env.is_empty() {
                    return invalid(format!(
                        "consumer {} has a key with an empty name or env",
                        listener.consumer
                    ));
                }
                if key.name.len() > MAX_KEY_NAME_LEN {
                    return invalid(format!(
                        "consumer {} has a key name longer than {MAX_KEY_NAME_LEN} bytes",
                        listener.consumer
                    ));
                }
                if !keys.insert(key.name.as_str()) {
                    return invalid(format!(
                        "consumer {} lists key {} more than once",
                        listener.consumer, key.name
                    ));
                }
            }
        }
        Ok(())
    }

    /// Reads every listed variable through `lookup` and builds the [`Policy`].
    ///
    /// Fails with every unset or empty variable at once, so one restart fixes them all, and
    /// otherwise with every keyring that cannot be read. `main` passes [`std::env::var_os`].
    pub fn resolve(
        &self,
        lookup: impl Fn(&str) -> Option<OsString>,
    ) -> Result<Policy, ConfigError> {
        let mut policy = Policy::new();
        let mut missing = Vec::new();
        let mut invalid_keyrings = Vec::new();
        for listener in &self.listeners {
            let caller = CallerId::new(&listener.consumer);
            for secret in &listener.secrets {
                match lookup(&secret.env) {
                    Some(value) if !value.is_empty() => {
                        let value = SecretValue::new(value.into_encoded_bytes());
                        policy.allow(caller.clone(), secret.name.clone(), value);
                    }
                    other => missing.push(MissingVariable {
                        consumer: listener.consumer.clone(),
                        name: secret.name.clone(),
                        env: secret.env.clone(),
                        empty: other.is_some(),
                    }),
                }
            }
            for key in &listener.keys {
                match lookup(&key.env) {
                    Some(value) if !value.is_empty() => {
                        let json = zeroize::Zeroizing::new(value.into_encoded_bytes());
                        match Keyring::from_json(&key.name, &json) {
                            Ok(keyring) => {
                                policy.allow_key(
                                    caller.clone(),
                                    key.name.clone(),
                                    KeyHandle::new(keyring),
                                );
                            }
                            Err(e) => invalid_keyrings.push(InvalidKeyring {
                                consumer: listener.consumer.clone(),
                                name: key.name.clone(),
                                env: key.env.clone(),
                                reason: e.to_string(),
                            }),
                        }
                    }
                    other => missing.push(MissingVariable {
                        consumer: listener.consumer.clone(),
                        name: key.name.clone(),
                        env: key.env.clone(),
                        empty: other.is_some(),
                    }),
                }
            }
        }
        if !missing.is_empty() {
            Err(ConfigError::MissingVariables(missing))
        } else if !invalid_keyrings.is_empty() {
            Err(ConfigError::InvalidKeyrings(invalid_keyrings))
        } else {
            Ok(policy)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    const EXAMPLE: &str = r#"
version: 1
listeners:
  - socket: /run/trestle-sds/egress-gateway/sds.sock
    consumer: egress-gateway
    secrets:
      - name: workgraph-planner
        env: WORKGRAPH_PLANNER_API_KEY
      - name: workgraph-reviewer
        env: WORKGRAPH_REVIEWER_API_KEY
  - socket: /run/trestle-sds/other/sds.sock
    consumer: other
    keys:
      - name: server-kek
        env: OTHER_SERVER_KEK
"#;

    const KEYRING: &str = r#"{"versions": {"1": "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE="}, "active": 1, "aliases": {"kek-1": 1}}"#;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: HashMap<String, OsString> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn parses_the_documented_example() {
        let config = Config::from_yaml(EXAMPLE).unwrap();
        assert_eq!(config.listeners.len(), 2);
        assert_eq!(config.listeners[0].consumer, "egress-gateway");
        assert_eq!(
            config.listeners[0].secrets[1].env,
            "WORKGRAPH_REVIEWER_API_KEY"
        );
        assert!(config.listeners[1].secrets.is_empty());
        assert_eq!(config.listeners[1].keys[0].name, "server-kek");
        assert!(config.listeners[0].keys.is_empty());
    }

    #[test]
    fn resolves_values_per_consumer() {
        let config = Config::from_yaml(EXAMPLE).unwrap();
        let policy = config
            .resolve(env(&[
                ("WORKGRAPH_PLANNER_API_KEY", "p"),
                ("WORKGRAPH_REVIEWER_API_KEY", "r"),
                ("OTHER_SERVER_KEK", KEYRING),
            ]))
            .unwrap();
        let egress = CallerId::new("egress-gateway");
        assert_eq!(
            policy.get(&egress, "workgraph-planner").unwrap().expose(),
            b"p"
        );
        assert!(
            policy
                .get(&CallerId::new("other"), "workgraph-planner")
                .is_none()
        );
        let other = CallerId::new("other");
        assert_eq!(
            policy.key(&other, "server-kek").unwrap().current().active(),
            1
        );
        assert!(policy.key(&egress, "server-kek").is_none());
    }

    #[test]
    fn lists_every_invalid_keyring_without_values() {
        let yaml = format!(
            "{EXAMPLE}  - socket: /run/trestle-sds/third/sds.sock\n    consumer: third\n    keys:\n      - {{name: kek, env: THIRD_KEK}}\n"
        );
        let config = Config::from_yaml(&yaml).unwrap();
        let err = config
            .resolve(env(&[
                ("WORKGRAPH_PLANNER_API_KEY", "p"),
                ("WORKGRAPH_REVIEWER_API_KEY", "r"),
                (
                    "OTHER_SERVER_KEK",
                    r#"{"versions": {"1": "hunter2hunter2"}, "active": 1}"#,
                ),
                ("THIRD_KEK", "hunter2-not-json"),
            ]))
            .unwrap_err();
        let ConfigError::InvalidKeyrings(invalid) = &err else {
            panic!("{err}");
        };
        assert_eq!(invalid.len(), 2);
        let message = err.to_string();
        assert!(!message.contains("hunter2"), "{message}");
        assert!(
            message.contains("key server-kek (consumer other): $OTHER_SERVER_KEK"),
            "{message}"
        );
        assert!(message.contains("$THIRD_KEK: not a keyring"), "{message}");
    }

    #[test]
    fn a_missing_keyring_is_a_missing_variable() {
        let config = Config::from_yaml(EXAMPLE).unwrap();
        let err = config
            .resolve(env(&[
                ("WORKGRAPH_PLANNER_API_KEY", "p"),
                ("WORKGRAPH_REVIEWER_API_KEY", "r"),
            ]))
            .unwrap_err();
        assert!(
            err.to_string().contains("$OTHER_SERVER_KEK is not set"),
            "{err}"
        );
    }

    #[test]
    fn lists_every_missing_or_empty_variable_without_values() {
        let config = Config::from_yaml(EXAMPLE).unwrap();
        let err = config
            .resolve(env(&[
                ("WORKGRAPH_PLANNER_API_KEY", ""),
                ("OTHER_SERVER_KEK", KEYRING),
            ]))
            .unwrap_err();
        let ConfigError::MissingVariables(missing) = &err else {
            panic!("{err}");
        };
        assert_eq!(missing.len(), 2);
        assert!(missing[0].empty);
        assert!(!missing[1].empty);
        let message = err.to_string();
        assert!(
            message.contains("$WORKGRAPH_PLANNER_API_KEY is empty"),
            "{message}"
        );
        assert!(
            message.contains("$WORKGRAPH_REVIEWER_API_KEY is not set"),
            "{message}"
        );
    }

    #[test]
    fn rejects_other_versions() {
        let err = Config::from_yaml("version: 2\nlisteners: []\n").unwrap_err();
        assert!(err.to_string().contains("unsupported version 2"), "{err}");
    }

    #[test]
    fn rejects_unknown_fields() {
        let yaml = "version: 1\nlisteners:\n  - socket: /s\n    consumer: a\n    value: x\n";
        assert!(matches!(
            Config::from_yaml(yaml),
            Err(ConfigError::Parse { .. })
        ));
    }

    #[test]
    fn rejects_duplicates() {
        let dup_consumer = "version: 1\nlisteners:\n  - {socket: /a, consumer: c}\n  - {socket: /b, consumer: c}\n";
        assert!(
            Config::from_yaml(dup_consumer)
                .unwrap_err()
                .to_string()
                .contains("more than one listener")
        );
        let dup_socket = "version: 1\nlisteners:\n  - {socket: /a, consumer: c}\n  - {socket: /a, consumer: d}\n";
        assert!(
            Config::from_yaml(dup_socket)
                .unwrap_err()
                .to_string()
                .contains("more than one listener")
        );
        let dup_name = "version: 1\nlisteners:\n  - socket: /a\n    consumer: c\n    secrets:\n      - {name: n, env: A}\n      - {name: n, env: B}\n";
        assert!(
            Config::from_yaml(dup_name)
                .unwrap_err()
                .to_string()
                .contains("more than once")
        );
        let dup_key = "version: 1\nlisteners:\n  - socket: /a\n    consumer: c\n    keys:\n      - {name: k, env: A}\n      - {name: k, env: B}\n";
        assert!(
            Config::from_yaml(dup_key)
                .unwrap_err()
                .to_string()
                .contains("lists key k more than once")
        );
        let none = "version: 1\nlisteners: []\n";
        assert!(
            Config::from_yaml(none)
                .unwrap_err()
                .to_string()
                .contains("no listeners")
        );
    }
}
