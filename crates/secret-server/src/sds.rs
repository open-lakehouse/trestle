//! The SDS gRPC service: `StreamSecrets` (state of the world) and `FetchSecrets`.
//!
//! Only `generic_secret` resources are served. `DeltaSecrets` answers `UNIMPLEMENTED`.
//!
//! Nothing here logs a [`DiscoveryResponse`] or an `Any`: their derived `Debug` would print
//! the secret bytes. Logs carry consumers, names, versions and nonces only.

use std::collections::BTreeSet;
use std::pin::Pin;
use std::sync::Arc;

use envoy_types::pb::envoy::config::core::v3::DataSource;
use envoy_types::pb::envoy::config::core::v3::data_source::Specifier;
use envoy_types::pb::envoy::extensions::transport_sockets::tls::v3::{
    GenericSecret, Secret, secret,
};
use envoy_types::pb::envoy::service::discovery::v3::{
    DeltaDiscoveryRequest, DeltaDiscoveryResponse, DiscoveryRequest, DiscoveryResponse,
};
use envoy_types::pb::envoy::service::secret::v3::secret_discovery_service_server::{
    SecretDiscoveryService, SecretDiscoveryServiceServer,
};
use envoy_types::util::pack_any;
use futures::{Stream, StreamExt};
use prost::Name;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

use crate::caller::{CallerId, CallerResolver, SocketCaller};
use crate::policy::Policy;

type ResponseStream<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send + 'static>>;

/// The type URL of the resources this service serves.
pub fn secret_type_url() -> String {
    Secret::type_url()
}

/// The SDS service over a [`Policy`].
///
/// Each connection's caller comes from the resolver `R` ([`SocketCaller`] by default); the
/// policy decides what that caller may see.
#[derive(Debug)]
pub struct SdsService<R = SocketCaller> {
    policy: Arc<Policy>,
    resolver: Arc<R>,
    boot_id: Arc<str>,
}

impl<R> Clone for SdsService<R> {
    fn clone(&self) -> Self {
        Self {
            policy: self.policy.clone(),
            resolver: self.resolver.clone(),
            boot_id: self.boot_id.clone(),
        }
    }
}

impl SdsService<SocketCaller> {
    /// A service that identifies callers by the socket they connected on.
    pub fn new(policy: Arc<Policy>) -> Self {
        Self::with_resolver(policy, SocketCaller)
    }
}

impl<R: CallerResolver> SdsService<R> {
    /// A service that identifies callers with `resolver`.
    ///
    /// Each service gets a random boot id that prefixes every version it sends, so a
    /// restarted server never repeats a version a client has already seen.
    pub fn with_resolver(policy: Arc<Policy>, resolver: R) -> Self {
        Self {
            policy,
            resolver: Arc::new(resolver),
            boot_id: uuid::Uuid::new_v4().simple().to_string().into(),
        }
    }

    /// The tonic service to add to a server.
    pub fn into_server(self) -> SecretDiscoveryServiceServer<Self> {
        SecretDiscoveryServiceServer::new(self)
    }

    fn caller<T>(&self, request: &Request<T>) -> Result<CallerId, Status> {
        self.resolver
            .resolve(request.extensions())
            .ok_or_else(|| Status::unauthenticated("connection has no caller identity"))
    }
}

fn check_type_url(type_url: &str) -> Result<(), Status> {
    if type_url.is_empty() || type_url == secret_type_url() {
        Ok(())
    } else {
        Err(Status::invalid_argument(format!(
            "unsupported type_url {type_url}; this server serves {}",
            secret_type_url()
        )))
    }
}

/// The `Secret` resources for `names`, all of which the caller may see.
fn resources(
    policy: &Policy,
    caller: &CallerId,
    names: &BTreeSet<String>,
) -> Vec<envoy_types::pb::google::protobuf::Any> {
    names
        .iter()
        .filter_map(|name| {
            let value = policy.get(caller, name)?;
            Some(pack_any(Secret {
                name: name.clone(),
                r#type: Some(secret::Type::GenericSecret(GenericSecret {
                    secret: Some(DataSource {
                        specifier: Some(Specifier::InlineBytes(value.expose().to_vec())),
                        ..Default::default()
                    }),
                    ..Default::default()
                })),
            }))
        })
        .collect()
}

/// One `StreamSecrets` stream: what it last sent and the nonce it expects back.
struct StreamState {
    caller: CallerId,
    boot_id: Arc<str>,
    sequence: u64,
    last_nonce: Option<String>,
    last_version: Option<String>,
    last_sent: Option<BTreeSet<String>>,
}

impl StreamState {
    /// Handles one request and returns the response to send, if any.
    fn on_request(
        &mut self,
        policy: &Policy,
        request: &DiscoveryRequest,
    ) -> Option<DiscoveryResponse> {
        let consumer = self.caller.as_str();
        if !request.response_nonce.is_empty() {
            if self.last_nonce.as_deref() != Some(request.response_nonce.as_str()) {
                tracing::debug!(consumer, nonce = %request.response_nonce, "ignoring a request with a stale nonce");
                return None;
            }
            if let Some(error) = &request.error_detail {
                tracing::warn!(
                    consumer,
                    names = ?self.last_sent,
                    version = ?self.last_version,
                    code = error.code,
                    error = %error.message,
                    "client rejected secrets (NACK); not re-sending this version",
                );
                return None;
            }
        }

        // An empty name list is never a wildcard: a consumer gets only what it names.
        let requested: BTreeSet<String> = request.resource_names.iter().cloned().collect();
        let (allowed, denied): (BTreeSet<String>, BTreeSet<String>) = requested
            .into_iter()
            .partition(|name| policy.get(&self.caller, name).is_some());
        if !denied.is_empty() {
            tracing::warn!(consumer, names = ?denied, "consumer asked for secrets it may not see; not sending them");
        }
        if allowed.is_empty() || self.last_sent.as_ref() == Some(&allowed) {
            return None;
        }

        self.sequence += 1;
        let version = format!("{}-{}", self.boot_id, self.sequence);
        let nonce = self.sequence.to_string();
        tracing::info!(consumer, names = ?allowed, %version, "sending secrets");
        let response = DiscoveryResponse {
            version_info: version.clone(),
            resources: resources(policy, &self.caller, &allowed),
            type_url: secret_type_url(),
            nonce: nonce.clone(),
            ..Default::default()
        };
        self.last_nonce = Some(nonce);
        self.last_version = Some(version);
        self.last_sent = Some(allowed);
        Some(response)
    }
}

#[tonic::async_trait]
impl<R: CallerResolver> SecretDiscoveryService for SdsService<R> {
    type DeltaSecretsStream = ResponseStream<DeltaDiscoveryResponse>;
    type StreamSecretsStream = ResponseStream<DiscoveryResponse>;

    async fn delta_secrets(
        &self,
        _request: Request<Streaming<DeltaDiscoveryRequest>>,
    ) -> Result<Response<Self::DeltaSecretsStream>, Status> {
        Err(Status::unimplemented(
            "DeltaSecrets is not supported; use StreamSecrets",
        ))
    }

    async fn stream_secrets(
        &self,
        request: Request<Streaming<DiscoveryRequest>>,
    ) -> Result<Response<Self::StreamSecretsStream>, Status> {
        let caller = self.caller(&request)?;
        let mut inbound = request.into_inner();
        let policy = self.policy.clone();
        let mut state = StreamState {
            caller,
            boot_id: self.boot_id.clone(),
            sequence: 0,
            last_nonce: None,
            last_version: None,
            last_sent: None,
        };
        let (tx, rx) = mpsc::channel(4);
        tokio::spawn(async move {
            tracing::debug!(consumer = state.caller.as_str(), "secret stream opened");
            while let Some(message) = inbound.next().await {
                let request = match message {
                    Ok(request) => request,
                    Err(status) => {
                        tracing::debug!(consumer = state.caller.as_str(), %status, "secret stream ended");
                        return;
                    }
                };
                if let Err(status) = check_type_url(&request.type_url) {
                    let _ = tx.send(Err(status)).await;
                    return;
                }
                if let Some(response) = state.on_request(&policy, &request)
                    && tx.send(Ok(response)).await.is_err()
                {
                    return;
                }
            }
            tracing::debug!(consumer = state.caller.as_str(), "secret stream closed");
        });
        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }

    async fn fetch_secrets(
        &self,
        request: Request<DiscoveryRequest>,
    ) -> Result<Response<DiscoveryResponse>, Status> {
        let caller = self.caller(&request)?;
        let request = request.into_inner();
        check_type_url(&request.type_url)?;
        if request.resource_names.is_empty() {
            return Err(Status::invalid_argument(
                "FetchSecrets needs at least one resource name",
            ));
        }
        let names: BTreeSet<String> = request.resource_names.into_iter().collect();
        // The same answer for a name nobody holds and a name another consumer holds.
        if let Some(name) = names
            .iter()
            .find(|name| self.policy.get(&caller, name).is_none())
        {
            tracing::warn!(consumer = caller.as_str(), name = %name, "fetch for a secret the consumer may not see");
            return Err(Status::not_found(format!("secret {name} not found")));
        }
        tracing::info!(consumer = caller.as_str(), names = ?names, "fetched secrets");
        Ok(Response::new(DiscoveryResponse {
            version_info: self.boot_id.to_string(),
            resources: resources(&self.policy, &caller, &names),
            type_url: secret_type_url(),
            ..Default::default()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::SecretValue;

    fn state(caller: &str) -> StreamState {
        StreamState {
            caller: CallerId::new(caller),
            boot_id: "boot".into(),
            sequence: 0,
            last_nonce: None,
            last_version: None,
            last_sent: None,
        }
    }

    fn policy() -> Policy {
        let mut policy = Policy::new();
        policy.allow(CallerId::new("a"), "planner", SecretValue::new("p"));
        policy.allow(CallerId::new("a"), "reviewer", SecretValue::new("r"));
        policy.allow(CallerId::new("b"), "other", SecretValue::new("o"));
        policy
    }

    fn request(names: &[&str], version: &str, nonce: &str) -> DiscoveryRequest {
        DiscoveryRequest {
            version_info: version.into(),
            resource_names: names.iter().map(|n| n.to_string()).collect(),
            type_url: secret_type_url(),
            response_nonce: nonce.into(),
            ..Default::default()
        }
    }

    fn names(response: &DiscoveryResponse) -> Vec<String> {
        use prost::Message;
        response
            .resources
            .iter()
            .map(|any| Secret::decode(any.value.as_slice()).unwrap().name)
            .collect()
    }

    #[test]
    fn ack_nack_and_stale_nonces() {
        let policy = policy();
        let mut s = state("a");
        let first = s
            .on_request(&policy, &request(&["planner"], "", ""))
            .unwrap();
        assert_eq!(first.version_info, "boot-1");
        assert_eq!(first.nonce, "1");
        assert_eq!(names(&first), ["planner"]);
        // ACK: nothing to send.
        assert!(
            s.on_request(&policy, &request(&["planner"], "boot-1", "1"))
                .is_none()
        );
        // Stale nonce, even with new names: ignored.
        assert!(
            s.on_request(&policy, &request(&["planner", "reviewer"], "boot-1", "0"))
                .is_none()
        );
        // ACK with more names: a new version with both.
        let second = s
            .on_request(&policy, &request(&["planner", "reviewer"], "boot-1", "1"))
            .unwrap();
        assert_eq!(second.version_info, "boot-2");
        assert_eq!(names(&second), ["planner", "reviewer"]);
        // NACK: logged, not re-sent.
        let mut nack = request(&["planner", "reviewer"], "boot-1", "2");
        nack.error_detail = Some(envoy_types::pb::google::rpc::Status {
            code: 3,
            message: "bad".into(),
            ..Default::default()
        });
        assert!(s.on_request(&policy, &nack).is_none());
    }

    #[test]
    fn other_consumers_names_are_never_sent() {
        let policy = policy();
        let mut b = state("b");
        assert!(
            b.on_request(&policy, &request(&["planner"], "", ""))
                .is_none()
        );
        let mixed = b
            .on_request(&policy, &request(&["planner", "other"], "", ""))
            .unwrap();
        assert_eq!(names(&mixed), ["other"]);
        // An empty list is not a wildcard.
        assert!(
            state("a")
                .on_request(&policy, &request(&[], "", ""))
                .is_none()
        );
    }

    #[test]
    fn rejects_foreign_type_urls() {
        assert!(check_type_url("").is_ok());
        assert!(check_type_url(&secret_type_url()).is_ok());
        assert!(check_type_url("type.googleapis.com/envoy.config.cluster.v3.Cluster").is_err());
    }
}
