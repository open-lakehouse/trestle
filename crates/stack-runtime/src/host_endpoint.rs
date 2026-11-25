//! [`HostEndpoint`]: the host side of the host source, an HTTP endpoint that serves the
//! registered instances' secret servers from [`SecretProviders`]. Embeddable: the desktop runs
//! it in-process (and the CLI could, in a daemon).
//!
//! - `GET /<token>/secrets/<variable>` answers the variable's value (`200`, `ETag: "<n>"`,
//!   `Cache-Control: no-store`) for the instance registered with `token`. The token is compared
//!   in constant time against every registered one. An unknown token, a variable the instance
//!   didn't register, and an unregistered instance all get the same empty `404`.
//! - With `If-None-Match` naming the current version and `?wait=<secs>` (at most
//!   [`MAX_WAIT`]), the request is held until the value changes (`200`) or the hold ends. At the
//!   end of a hold the endpoint reads the variable's provider again, so a value changed outside
//!   [`notify`](HostEndpoint::notify) (a keychain entry edited by hand) still arrives within one
//!   hold; unchanged, the answer is `304`.
//! - [`notify`](HostEndpoint::notify) reads an instance's providers again now, for a rotation:
//!   every held request for a changed variable answers at once.
//! - A provider that fails on a refresh keeps the last value; [`problems`](HostEndpoint::problems)
//!   reports it.
//!
//! Bind it where the containers reach the host ([`Reach`]), on the port the plan renders into
//! `secret-server.yaml` ([`EgressSelection::host_source_port`](olai_stack_topology::EgressSelection::host_source_port)).

use std::collections::BTreeMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use olai_stack_state::InstanceId;
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::sync::watch;

use crate::host_source::{HostEndpointError, HostSecretEndpoint, HostValue};
use crate::secrets::{SecretError, SecretProviders, SecretValue};

/// The longest a request may be held.
pub const MAX_WAIT: Duration = Duration::from_secs(60);

/// Where the containers reach the endpoint: the address to bind, and the host name they use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reach {
    /// The address to bind.
    pub bind: IpAddr,
    /// The name or address the containers reach it at.
    pub url_host: String,
}

impl Reach {
    /// The host's loopback, which Docker Desktop, OrbStack and Colima forward
    /// `host.docker.internal` to.
    pub fn loopback() -> Self {
        Reach {
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            url_host: "host.docker.internal".into(),
        }
    }

    /// Where containers reach this host, asking `docker` on Linux (blocking): the loopback on
    /// macOS and Docker Desktop, else the default bridge's gateway, the host's address on
    /// `docker0`. The secret server maps `host.docker.internal` to that gateway
    /// (`extra_hosts: host-gateway`), so the rendered URL holds on every engine.
    pub fn detect() -> Self {
        if !cfg!(target_os = "linux") {
            return Reach::loopback();
        }
        let answer = |args: &[&str]| {
            std::process::Command::new("docker")
                .args(args)
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        };
        if answer(&["info", "--format", "{{.OperatingSystem}}"])
            .is_some_and(|os| os.contains("Docker Desktop"))
        {
            return Reach::loopback();
        }
        match answer(&[
            "network",
            "inspect",
            "bridge",
            "--format",
            "{{(index .IPAM.Config 0).Gateway}}",
        ])
        .and_then(|ip| ip.parse::<IpAddr>().ok())
        {
            Some(ip) => Reach {
                bind: ip,
                url_host: "host.docker.internal".into(),
            },
            None => Reach::loopback(),
        }
    }
}

/// One served variable.
struct Served {
    source: HostValue,
    value: SecretValue,
    version: u64,
    problem: Option<String>,
}

struct Entry {
    token: SecretValue,
    vars: BTreeMap<String, Served>,
}

struct Inner {
    providers: SecretProviders,
    entries: Mutex<BTreeMap<InstanceId, Entry>>,
    /// Bumped on every change, so held requests look again.
    changed: watch::Sender<u64>,
    /// A counter for versions, so a re-registered variable never repeats one.
    versions: std::sync::atomic::AtomicU64,
}

/// The endpoint. Clones share the registrations; see the [module docs](self).
#[derive(Clone)]
pub struct HostEndpoint {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for HostEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostEndpoint")
            .field("instances", &self.instances())
            .finish_non_exhaustive()
    }
}

/// A listening endpoint. Dropping it stops listening (registrations are kept).
#[derive(Debug)]
pub struct Listening {
    addr: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl Listening {
    /// The bound address.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

impl Drop for Listening {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Deserialize)]
struct Wait {
    wait: Option<u64>,
}

type Found = (InstanceId, SecretValue, u64);

impl HostEndpoint {
    /// An endpoint resolving references with `providers`, serving nobody yet.
    pub fn new(providers: SecretProviders) -> Self {
        Self {
            inner: Arc::new(Inner {
                providers,
                entries: Mutex::new(BTreeMap::new()),
                changed: watch::channel(0).0,
                versions: std::sync::atomic::AtomicU64::new(0),
            }),
        }
    }

    /// Listen on `addr` (e.g. [`Reach::bind`] and the plan's port) until the returned guard is
    /// dropped.
    pub async fn listen(&self, addr: SocketAddr) -> io::Result<Listening> {
        let listener = TcpListener::bind(addr).await?;
        let addr = listener.local_addr()?;
        let app = Router::new()
            .route("/{token}/secrets/{variable}", get(handle))
            .with_state(self.clone());
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Listening { addr, task })
    }

    /// The registered instances.
    pub fn instances(&self) -> Vec<InstanceId> {
        self.entries().keys().cloned().collect()
    }

    /// The variables of `instance` whose provider failed on the last refresh, with why. Their
    /// last values are still served.
    pub fn problems(&self, instance: &InstanceId) -> Vec<(String, String)> {
        self.entries()
            .get(instance)
            .map(|e| {
                e.vars
                    .iter()
                    .filter_map(|(k, s)| s.problem.clone().map(|p| (k.clone(), p)))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Read `instance`'s references again now (after a rotation): changed values reach the
    /// held requests at once.
    pub async fn notify(&self, instance: &InstanceId) {
        self.refresh(instance, None).await;
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, BTreeMap<InstanceId, Entry>> {
        self.inner
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn next_version(&self) -> u64 {
        self.inner
            .versions
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1
    }

    /// The instance, value and version of `variable` for the holder of `token`.
    fn find(&self, token: &str, variable: &str) -> Option<Found> {
        let entries = self.entries();
        // Every token is compared, matching or not, so timing doesn't tell how far a guess got.
        let mut found = None;
        for (instance, entry) in entries.iter() {
            if constant_time_eq(entry.token.expose().as_bytes(), token.as_bytes()) {
                found = Some((instance, entry));
            }
        }
        let (instance, entry) = found?;
        let served = entry.vars.get(variable)?;
        Some((instance.clone(), served.value.clone(), served.version))
    }

    /// Re-read the references of `instance` (all of them, or `only` one) and record changes.
    async fn refresh(&self, instance: &InstanceId, only: Option<&str>) {
        let refs: Vec<(String, olai_stack_state::SecretRef)> = {
            let entries = self.entries();
            let Some(entry) = entries.get(instance) else {
                return;
            };
            entry
                .vars
                .iter()
                .filter(|(k, _)| only.is_none_or(|o| o == k.as_str()))
                .filter_map(|(k, s)| match &s.source {
                    HostValue::Ref(r) => Some((k.clone(), r.clone())),
                    HostValue::Fixed(_) => None,
                })
                .collect()
        };
        if refs.is_empty() {
            return;
        }
        let providers = self.inner.providers.clone();
        // Providers block (the keychain), so off the runtime's threads.
        let read = tokio::task::spawn_blocking(move || {
            refs.into_iter()
                .map(|(k, r)| {
                    let value = providers.resolve(&r);
                    (k, value)
                })
                .collect::<Vec<_>>()
        })
        .await;
        let Ok(read) = read else { return };
        let mut changed = false;
        {
            let mut entries = self.entries();
            let Some(entry) = entries.get_mut(instance) else {
                return;
            };
            for (var, value) in read {
                let Some(served) = entry.vars.get_mut(&var) else {
                    continue;
                };
                match value {
                    Ok(value) if value.expose().is_empty() => {
                        served.problem = Some("the provider holds an empty value".into());
                    }
                    Ok(value) => {
                        served.problem = None;
                        if value != served.value {
                            served.value = value;
                            served.version = self.next_version();
                            changed = true;
                        }
                    }
                    Err(e) => served.problem = Some(e.to_string()),
                }
            }
        }
        if changed {
            self.inner.changed.send_modify(|n| *n += 1);
        }
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn etag(version: u64) -> String {
    format!("\"{version}\"")
}

fn value_response(value: &SecretValue, version: u64) -> Response {
    let mut response = (StatusCode::OK, value.expose().to_owned()).into_response();
    let headers = response.headers_mut();
    if let Ok(etag) = HeaderValue::from_str(&etag(version)) {
        headers.insert(header::ETAG, etag);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    response
}

async fn handle(
    State(endpoint): State<HostEndpoint>,
    UrlPath((token, variable)): UrlPath<(String, String)>,
    Query(wait): Query<Wait>,
    headers: HeaderMap,
) -> Response {
    // Subscribed before the first look, so a change in between isn't missed.
    let mut changes = endpoint.inner.changed.subscribe();
    let Some((instance, value, version)) = endpoint.find(&token, &variable) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let seen = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    if seen.as_deref() != Some(etag(version).as_str()) {
        return value_response(&value, version);
    }
    let hold = Duration::from_secs(wait.wait.unwrap_or(0)).min(MAX_WAIT);
    let deadline = tokio::time::Instant::now() + hold;
    loop {
        let timed_out = tokio::select! {
            () = tokio::time::sleep_until(deadline) => true,
            changed = changes.changed() => changed.is_err(),
        };
        if timed_out {
            endpoint.refresh(&instance, Some(&variable)).await;
        }
        let Some((_, value, now)) = endpoint.find(&token, &variable) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        if now != version {
            return value_response(&value, now);
        }
        if timed_out {
            return StatusCode::NOT_MODIFIED.into_response();
        }
    }
}

#[async_trait]
impl HostSecretEndpoint for HostEndpoint {
    async fn register(
        &self,
        instance: &InstanceId,
        token: &SecretValue,
        vars: BTreeMap<String, HostValue>,
    ) -> Result<(), HostEndpointError> {
        let providers = self.inner.providers.clone();
        let resolved = tokio::task::spawn_blocking(move || {
            vars.into_iter()
                .map(|(var, source)| {
                    let value = match &source {
                        HostValue::Ref(r) => providers.resolve(r),
                        HostValue::Fixed(v) => Ok(v.clone()),
                    };
                    (var, source, value)
                })
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|e| HostEndpointError::Unavailable(e.to_string()))?;
        let mut served = BTreeMap::new();
        for (var, source, value) in resolved {
            let value = match value {
                Ok(v) if v.expose().is_empty() => {
                    return Err(HostEndpointError::Secret {
                        var: var.clone(),
                        source: SecretError::Missing {
                            provider: provider_of(&source),
                            key: var,
                        },
                    });
                }
                Ok(v) => v,
                Err(source) => return Err(HostEndpointError::Secret { var, source }),
            };
            served.insert(
                var,
                Served {
                    source,
                    value,
                    version: self.next_version(),
                    problem: None,
                },
            );
        }
        self.entries().insert(
            instance.clone(),
            Entry {
                token: token.clone(),
                vars: served,
            },
        );
        // Held requests under an old token look again, and find nothing.
        self.inner.changed.send_modify(|n| *n += 1);
        Ok(())
    }

    async fn unregister(&self, instance: &InstanceId) {
        if self.entries().remove(instance).is_some() {
            self.inner.changed.send_modify(|n| *n += 1);
        }
    }
}

fn provider_of(source: &HostValue) -> String {
    match source {
        HostValue::Ref(r) => r.provider.clone(),
        HostValue::Fixed(_) => "fixed".into(),
    }
}

#[cfg(test)]
mod tests {
    use olai_stack_state::SecretRef;

    use super::*;
    use crate::secrets::SecretProvider;

    /// A provider whose values the test changes.
    #[derive(Clone, Default)]
    struct Mutable(Arc<Mutex<BTreeMap<String, String>>>);

    impl Mutable {
        fn set(&self, key: &str, value: &str) {
            self.0.lock().unwrap().insert(key.into(), value.into());
        }
    }

    impl SecretProvider for Mutable {
        fn name(&self) -> &str {
            "mutable"
        }

        fn get(&self, key: &str) -> Result<SecretValue, SecretError> {
            self.0
                .lock()
                .unwrap()
                .get(key)
                .map(|v| SecretValue::new(v.clone()))
                .ok_or_else(|| SecretError::Missing {
                    provider: "mutable".into(),
                    key: key.into(),
                })
        }
    }

    fn reference(key: &str) -> HostValue {
        HostValue::Ref(SecretRef {
            provider: "mutable".into(),
            key: key.into(),
        })
    }

    async fn endpoint() -> (HostEndpoint, Mutable, Listening, InstanceId, String) {
        let provider = Mutable::default();
        provider.set("route", "v1");
        let endpoint = HostEndpoint::new(SecretProviders::new().with(provider.clone()));
        let listening = endpoint
            .listen("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let instance = InstanceId("i1".to_string());
        let token = "a".repeat(64);
        endpoint
            .register(
                &instance,
                &SecretValue::new(token.clone()),
                [
                    ("ROUTE_KEY".to_string(), reference("route")),
                    (
                        "FIXED".to_string(),
                        HostValue::Fixed(SecretValue::new("fixed-v")),
                    ),
                ]
                .into(),
            )
            .await
            .unwrap();
        (endpoint, provider, listening, instance, token)
    }

    async fn get(
        addr: SocketAddr,
        path: &str,
        etag: Option<&str>,
    ) -> (u16, Option<String>, String) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let etag = etag
            .map(|e| format!("If-None-Match: {e}\r\n"))
            .unwrap_or_default();
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: x\r\n{etag}Connection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let mut raw = String::new();
        stream.read_to_string(&mut raw).await.unwrap();
        let (head, body) = raw.split_once("\r\n\r\n").unwrap();
        let status = head[9..12].parse().unwrap();
        let etag = head
            .lines()
            .find_map(|l| l.strip_prefix("etag: "))
            .map(str::to_owned);
        (status, etag, body.to_owned())
    }

    #[tokio::test]
    async fn serves_registered_variables_and_404s_everything_else() {
        let (endpoint, _provider, listening, instance, token) = endpoint().await;
        let addr = listening.addr();
        let (status, etag, body) = get(addr, &format!("/{token}/secrets/ROUTE_KEY"), None).await;
        assert_eq!((status, body.as_str()), (200, "v1"));
        assert!(etag.is_some());
        let (status, _, body) = get(addr, &format!("/{token}/secrets/FIXED"), None).await;
        assert_eq!((status, body.as_str()), (200, "fixed-v"));
        for path in [
            format!("/{}/secrets/ROUTE_KEY", "b".repeat(64)),
            format!("/{}/secrets/ROUTE_KEY", &token[..63]),
            format!("/{token}/secrets/OTHER"),
            "/secrets/ROUTE_KEY".to_string(),
        ] {
            let (status, _, body) = get(addr, &path, None).await;
            assert_eq!((status, body.as_str()), (404, ""), "{path}");
        }
        endpoint.unregister(&instance).await;
        let (status, _, _) = get(addr, &format!("/{token}/secrets/ROUTE_KEY"), None).await;
        assert_eq!(status, 404);
        assert!(endpoint.instances().is_empty());
    }

    #[tokio::test]
    async fn a_held_request_answers_a_rotation_or_304_at_the_end() {
        let (endpoint, provider, listening, instance, token) = endpoint().await;
        let addr = listening.addr();
        let path = format!("/{token}/secrets/ROUTE_KEY");
        let (_, etag, _) = get(addr, &path, None).await;
        let etag = etag.unwrap();
        // Unchanged: 304 when the hold ends.
        let (status, _, _) = get(addr, &format!("{path}?wait=1"), Some(&etag)).await;
        assert_eq!(status, 304);

        // A rotation with notify answers the held request at once.
        let held = tokio::spawn({
            let (path, etag) = (path.clone(), etag.clone());
            async move { get(addr, &format!("{path}?wait=30"), Some(&etag)).await }
        });
        tokio::time::sleep(Duration::from_millis(200)).await;
        provider.set("route", "v2");
        endpoint.notify(&instance).await;
        let (status, next, body) = tokio::time::timeout(Duration::from_secs(5), held)
            .await
            .expect("answered before the hold ends")
            .unwrap();
        assert_eq!((status, body.as_str()), (200, "v2"));
        assert_ne!(next.as_deref(), Some(etag.as_str()));

        // Without notify, the end of a hold reads the provider again.
        provider.set("route", "v3");
        let (status, _, body) = get(addr, &format!("{path}?wait=1"), next.as_deref()).await;
        assert_eq!((status, body.as_str()), (200, "v3"));
    }

    #[tokio::test]
    async fn a_failing_provider_keeps_the_last_value() {
        let (endpoint, provider, listening, instance, token) = endpoint().await;
        provider.0.lock().unwrap().clear();
        endpoint.notify(&instance).await;
        let (status, _, body) = get(
            listening.addr(),
            &format!("/{token}/secrets/ROUTE_KEY"),
            None,
        )
        .await;
        assert_eq!((status, body.as_str()), (200, "v1"));
        let problems = endpoint.problems(&instance);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].0, "ROUTE_KEY");
    }

    #[tokio::test]
    async fn registering_an_unresolvable_variable_fails() {
        let endpoint = HostEndpoint::new(SecretProviders::new().with(Mutable::default()));
        let error = endpoint
            .register(
                &InstanceId("i1".to_string()),
                &SecretValue::new("t"),
                [("ROUTE_KEY".to_string(), reference("route"))].into(),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("ROUTE_KEY"), "{error}");
        assert!(endpoint.instances().is_empty());
    }

    #[test]
    fn reach_is_the_loopback_off_linux() {
        if !cfg!(target_os = "linux") {
            assert_eq!(Reach::detect(), Reach::loopback());
        }
    }
}
