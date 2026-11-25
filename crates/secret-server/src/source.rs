//! Where values come from: the process environment, or a host endpoint followed live.
//!
//! [`resolve`] builds the [`Policy`] for either source of [`SourceConfig`]:
//!
//! - **`env`**: every listed variable is read from the process environment once
//!   ([`Config::resolve`]). Nothing changes until the server is restarted.
//! - **`host`**: the process environment holds only the endpoint's path token. Every listed
//!   variable is fetched once with `GET <url>/<token>/secrets/<variable>`; a variable the
//!   endpoint doesn't serve (`404`) or serves empty fails startup as an unset one does. While
//!   the endpoint can't be reached, startup retries until [`HostOptions::startup_timeout`].
//!   The returned [`HostWatcher`] then long-polls each variable
//!   (`?wait=<secs>` with `If-None-Match`), and a changed value reaches every open stream as a
//!   new SDS version ([`SecretCell::set`](crate::policy::SecretCell::set)), or every key
//!   service watcher as a new keyring ([`KeyHandle::replace`]).
//!
//! When the endpoint is down or stops serving a variable later, the server keeps the last
//! values and retries with backoff. Logs name the base URL and variables, never the token or a
//! value.

use std::ffi::OsString;
use std::fmt;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{ETAG, IF_NONE_MATCH};
use tokio::task::JoinSet;
use zeroize::Zeroizing;

use crate::config::{Binding, Bindings, Config, ConfigError, MissingVariable, SourceConfig};
use crate::keyring::{KeyHandle, Keyring};
use crate::policy::Policy;
use crate::secret::SecretValue;

/// Timings of the host source. [`Default`] is what the binary uses.
#[derive(Debug, Clone)]
pub struct HostOptions {
    /// How long the endpoint may hold a long poll before answering `304 Not Modified`.
    pub wait: Duration,
    /// How long startup keeps retrying an endpoint it can't reach.
    pub startup_timeout: Duration,
    /// The longest pause between retries after a failure.
    pub max_backoff: Duration,
}

impl Default for HostOptions {
    fn default() -> Self {
        Self {
            wait: Duration::from_secs(30),
            startup_timeout: Duration::from_secs(30),
            max_backoff: Duration::from_secs(30),
        }
    }
}

/// The policy a source built, and, for the `host` source, the watcher that keeps it current.
#[derive(Debug)]
pub struct Resolved {
    /// The policy to serve.
    pub policy: Policy,
    /// Follows the host endpoint. `None` for the `env` source.
    pub watcher: Option<HostWatcher>,
}

/// Builds the policy from the configured source. `lookup` reads the process environment
/// (`main` passes [`std::env::var_os`]); with the `host` source it is asked for the token only.
pub async fn resolve(
    config: &Config,
    lookup: impl Fn(&str) -> Option<OsString>,
    options: HostOptions,
) -> Result<Resolved, ConfigError> {
    let (url, token_env) = match &config.source {
        SourceConfig::Env => {
            return Ok(Resolved {
                policy: config.resolve(lookup)?,
                watcher: None,
            });
        }
        SourceConfig::Host { url, token_env } => (url, token_env),
    };
    let token = match lookup(token_env) {
        Some(token) if !token.is_empty() => token,
        other => {
            return Err(ConfigError::MissingVariables(vec![MissingVariable {
                consumer: "secret-server".into(),
                name: "host source token".into(),
                env: token_env.clone(),
                empty: other.is_some(),
            }]));
        }
    };
    let token = Zeroizing::new(token.into_encoded_bytes());
    if !token
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(ConfigError::Invalid(format!(
            "${token_env} must hold a URL-safe path token (letters, digits, - and _)"
        )));
    }
    let token = Zeroizing::new(String::from_utf8_lossy(&token).into_owned());
    let client = HostClient::new(url, token, &options)?;

    let mut fetched = std::collections::HashMap::new();
    let deadline = tokio::time::Instant::now() + options.startup_timeout;
    for variable in config.variables() {
        let mut backoff = Duration::from_millis(200);
        loop {
            match client.fetch(variable, None, None).await {
                Ok(Fetched::Value { value, etag }) => {
                    fetched.insert(variable.to_string(), (value, etag));
                    break;
                }
                // Missing: reported with every other missing variable below.
                Ok(Fetched::NotFound) => break,
                Ok(Fetched::NotModified) => {
                    return Err(ConfigError::HostUnreachable {
                        url: client.base.clone(),
                        reason: "answered 304 to a request without a version".into(),
                    });
                }
                Err(e) if tokio::time::Instant::now() + backoff < deadline => {
                    tracing::info!(variable, url = %client.base, error = %e, "host source not ready; retrying");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(options.max_backoff);
                }
                Err(e) => {
                    return Err(ConfigError::HostUnreachable {
                        url: client.base.clone(),
                        reason: e.to_string(),
                    });
                }
            }
        }
    }
    let (policy, bindings) = config.build(|variable| {
        fetched
            .get(variable)
            .map(|(value, _): &(Zeroizing<Vec<u8>>, _)| value.to_vec())
    })?;
    let etags = bindings
        .keys()
        .map(|variable| {
            let etag = fetched.get(variable).and_then(|(_, etag)| etag.clone());
            (variable.clone(), etag)
        })
        .collect();
    tracing::info!(url = %client.base, variables = bindings.len(), "values from the host source");
    Ok(Resolved {
        policy,
        watcher: Some(HostWatcher {
            client,
            bindings,
            etags,
            options,
        }),
    })
}

/// Long-polls the host endpoint for every variable and applies changed values.
pub struct HostWatcher {
    client: HostClient,
    bindings: Bindings,
    etags: std::collections::BTreeMap<String, Option<String>>,
    options: HostOptions,
}

impl fmt::Debug for HostWatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostWatcher")
            .field("url", &self.client.base)
            .field("variables", &self.bindings.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl HostWatcher {
    /// Follows every variable until the returned guard is dropped.
    pub fn spawn(self) -> WatchGuard {
        let mut tasks = JoinSet::new();
        for (variable, targets) in self.bindings {
            let etag = self.etags.get(&variable).cloned().flatten();
            let client = self.client.clone();
            let options = self.options.clone();
            tasks.spawn(watch(client, variable, targets, etag, options));
        }
        WatchGuard { _tasks: tasks }
    }
}

/// Stops the watchers when dropped.
#[derive(Debug)]
pub struct WatchGuard {
    _tasks: JoinSet<()>,
}

async fn watch(
    client: HostClient,
    variable: String,
    targets: Vec<Binding>,
    mut etag: Option<String>,
    options: HostOptions,
) {
    let mut backoff = Duration::from_secs(1);
    let mut failing = false;
    loop {
        let failure = match client
            .fetch(&variable, etag.as_deref(), Some(options.wait))
            .await
        {
            Ok(Fetched::Value { value, etag: next }) => {
                if value.is_empty() {
                    Some("an empty value".to_string())
                } else {
                    apply(&variable, &targets, &value);
                    if next.is_none() {
                        // No version to hold on: poll at the hold interval instead.
                        tokio::time::sleep(options.wait).await;
                    }
                    etag = next;
                    None
                }
            }
            Ok(Fetched::NotModified) => None,
            Ok(Fetched::NotFound) => Some("404: the endpoint no longer serves it".into()),
            Err(e) => Some(e.to_string()),
        };
        match failure {
            None => {
                if failing {
                    tracing::info!(variable = %variable, "host source answering again");
                }
                failing = false;
                backoff = Duration::from_secs(1).min(options.max_backoff);
            }
            Some(reason) => {
                if !failing {
                    tracing::warn!(variable = %variable, url = %client.base, %reason, "host source failed; keeping the last value");
                } else {
                    tracing::debug!(variable = %variable, %reason, "host source still failing");
                }
                failing = true;
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(options.max_backoff);
            }
        }
    }
}

fn apply(variable: &str, targets: &[Binding], value: &[u8]) {
    for target in targets {
        match target {
            Binding::Secret(cell) => {
                if cell.set(SecretValue::new(value.to_vec())) {
                    tracing::info!(variable, "secret changed at the host source");
                }
            }
            Binding::Key { name, handle } => replace_keyring(variable, name, handle, value),
        }
    }
}

fn replace_keyring(variable: &str, name: &str, handle: &KeyHandle, value: &[u8]) {
    let keyring = match Keyring::from_json(name, value) {
        Ok(keyring) => keyring,
        Err(e) => {
            tracing::warn!(variable, key = name, error = %e, "keyring from the host source can't be read; keeping the last one");
            return;
        }
    };
    match handle.replace(keyring) {
        Ok(()) => tracing::info!(variable, key = name, "keyring changed at the host source"),
        Err(e) => {
            tracing::warn!(variable, key = name, error = %e, "keyring from the host source refused")
        }
    }
}

/// What the endpoint answered for one variable.
enum Fetched {
    Value {
        value: Zeroizing<Vec<u8>>,
        etag: Option<String>,
    },
    NotModified,
    NotFound,
}

/// Why a fetch failed. Never carries the URL (it holds the token) or a body.
#[derive(Debug, thiserror::Error)]
enum FetchError {
    #[error("{0}")]
    Transport(String),
    #[error("status {0}")]
    Status(u16),
}

/// The endpoint's client. The token lives only inside the request URLs it builds.
#[derive(Clone)]
struct HostClient {
    http: reqwest::Client,
    base: String,
    token: std::sync::Arc<Zeroizing<String>>,
}

impl HostClient {
    fn new(
        url: &str,
        token: Zeroizing<String>,
        options: &HostOptions,
    ) -> Result<Self, ConfigError> {
        let http = reqwest::Client::builder()
            // A proxy from the container's environment must never see the token.
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            // Longer than any hold, so a long poll ends with the endpoint's `304`.
            .timeout(options.wait + Duration::from_secs(15))
            .build()
            .map_err(|e| ConfigError::Invalid(format!("cannot build the host client: {e}")))?;
        Ok(Self {
            http,
            base: url.trim_end_matches('/').to_string(),
            token: std::sync::Arc::new(token),
        })
    }

    async fn fetch(
        &self,
        variable: &str,
        etag: Option<&str>,
        wait: Option<Duration>,
    ) -> Result<Fetched, FetchError> {
        let mut url = format!("{}/{}/secrets/{variable}", self.base, self.token.as_str());
        if let Some(wait) = wait {
            url.push_str(&format!("?wait={}", wait.as_secs().max(1)));
        }
        let url = Zeroizing::new(url);
        let mut request = self.http.get(url.as_str());
        if let Some(etag) = etag {
            request = request.header(IF_NONE_MATCH, etag);
        }
        let response = request
            .send()
            .await
            .map_err(|e| FetchError::Transport(e.without_url().to_string()))?;
        match response.status() {
            StatusCode::OK => {
                let etag = response
                    .headers()
                    .get(ETAG)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned);
                let value = response
                    .bytes()
                    .await
                    .map_err(|e| FetchError::Transport(e.without_url().to_string()))?;
                Ok(Fetched::Value {
                    value: Zeroizing::new(value.to_vec()),
                    etag,
                })
            }
            StatusCode::NOT_MODIFIED => Ok(Fetched::NotModified),
            StatusCode::NOT_FOUND => Ok(Fetched::NotFound),
            status => Err(FetchError::Status(status.as_u16())),
        }
    }
}
