//! A [`ConnectRPC`](::connectrpc) client transport backed by [`CloudClient`].
//!
//! [`CloudTransport`] adapts this crate's cloud-provider authentication to the
//! [`ClientTransport`](::connectrpc::client::ClientTransport) interface that
//! ConnectRPC's generated clients are generic over. This lets a generated
//! client talk to a server deployed behind any supported cloud provider's auth
//! (AWS SigV4, Azure AD, GCP OAuth, Databricks OAuth/OIDC/PAT) by signing every
//! outgoing request with the configured [`CloudClient`].
//!
//! Available when the `connectrpc` feature is enabled.
//!
//! ```no_run
//! # use olai_http::CloudClient;
//! # use olai_http::connectrpc::CloudTransport;
//! # fn build() {
//! let client = CloudClient::new_with_token("token");
//! let transport = CloudTransport::new(client);
//! // let api = MyServiceClient::new(transport, config);
//! # let _ = transport;
//! # }
//! ```

use bytes::Bytes;
use connectrpc::client::{BoxFuture, ClientBody, ClientTransport};
use http::{Request, Response};
use http_body_util::BodyExt;

use crate::CloudClient;

/// Errors surfaced by [`CloudTransport::send`].
#[derive(Debug, thiserror::Error)]
pub enum CloudTransportError {
    /// The request body could not be collected into memory before sending.
    #[error("failed to read request body: {0}")]
    Body(#[source] Box<dyn std::error::Error + Send + Sync>),

    /// The collected request could not be converted into a `reqwest::Request`.
    #[error("failed to build request: {0}")]
    Build(#[source] reqwest::Error),

    /// Signing or transport failed (including after exhausting retries).
    #[error(transparent)]
    Olai(#[from] crate::Error),
}

/// A [`ClientTransport`](::connectrpc::client::ClientTransport) that signs and
/// sends ConnectRPC requests through a [`CloudClient`].
///
/// Construct one with [`CloudTransport::new`] and hand it to a generated
/// ConnectRPC client. Cloning is cheap: a [`CloudClient`] is `Arc`-backed.
#[derive(Clone)]
pub struct CloudTransport(CloudClient);

impl CloudTransport {
    /// Wrap a [`CloudClient`] as a ConnectRPC transport.
    pub fn new(client: CloudClient) -> Self {
        Self(client)
    }

    /// Borrow the underlying [`CloudClient`].
    pub fn client(&self) -> &CloudClient {
        &self.0
    }
}

impl From<CloudClient> for CloudTransport {
    fn from(client: CloudClient) -> Self {
        Self(client)
    }
}

impl ClientTransport for CloudTransport {
    // reqwest's Response implements `http_body::Body<Data = Bytes>`, and
    // `http::Response<reqwest::Body>: From<reqwest::Response>` — so we pass the
    // streaming response body straight back to ConnectRPC without re-boxing.
    type ResponseBody = reqwest::Body;
    type Error = CloudTransportError;

    fn send(
        &self,
        request: Request<ClientBody>,
    ) -> BoxFuture<'static, Result<Response<Self::ResponseBody>, Self::Error>> {
        // Clone the (Arc-backed) client into the 'static future; the signer
        // borrows `&self` only for the duration of `sign_and_send`.
        let client = self.0.clone();
        Box::pin(async move {
            let (parts, body) = request.into_parts();

            // ConnectRPC unary calls hand us a fully-buffered body, but the type
            // is the erased `BoxBody`, so collect it. Buffering is also required
            // for AWS SigV4, which hashes the body to sign it (a streaming body
            // would be signed as the empty payload and rejected by the server).
            let bytes: Bytes = body
                .collect()
                .await
                .map_err(|e| CloudTransportError::Body(Box::new(e)))?
                .to_bytes();

            // Rebuild as an http::Request<Bytes>, then into a reqwest::Request.
            // The connect URI is absolute, so the URL conversion is well-formed.
            let http_req = Request::from_parts(parts, bytes);
            let reqwest_req =
                reqwest::Request::try_from(http_req).map_err(CloudTransportError::Build)?;

            let response = client.sign_and_send(reqwest_req).await?;
            Ok(Response::from(response))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use connectrpc::client::full_body;
    use http_body_util::BodyExt;

    // Drive a request through the transport against a mock server: the body is
    // collected and sent, the bearer signer adds `authorization`, and the
    // response comes back as an http::Response whose body we can read.
    #[tokio::test]
    async fn cloud_transport_signs_and_round_trips() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/svc.Service/Method")
            .match_header("authorization", "Bearer tok")
            .match_body("request-bytes")
            .with_status(200)
            .with_body("response-bytes")
            .expect(1)
            .create_async()
            .await;

        let transport = CloudTransport::new(crate::CloudClient::new_with_token("tok"));

        let request = Request::builder()
            .method(http::Method::POST)
            .uri(format!("{}/svc.Service/Method", server.url()))
            .header("content-type", "application/proto")
            .body(full_body(Bytes::from_static(b"request-bytes")))
            .unwrap();

        let response = transport.send(request).await.unwrap();
        assert_eq!(response.status(), 200);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"response-bytes");
        mock.assert_async().await;
    }
}
