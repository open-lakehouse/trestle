use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tokio::runtime::Handle;

/// An abstraction over HTTP request execution.
///
/// This trait allows decoupling request building (which uses `reqwest::RequestBuilder`)
/// from request execution. The primary use case is [`SpawnService`], which spawns
/// execution on a dedicated I/O runtime -- useful when the CPU runtime has I/O disabled.
pub trait HttpService: Debug + Send + Sync + 'static {
    /// Execute an HTTP request, returning the response.
    ///
    /// The error type is [`crate::Error`]. Implementations that perform the
    /// actual network I/O surface transport failures as
    /// [`crate::Error::ReqwestError`], which the retry layer inspects to decide
    /// whether a failure is retryable. Other variants (e.g. a cancelled I/O
    /// task) are treated as non-retryable.
    fn call(
        &self,
        request: reqwest::Request,
    ) -> Pin<Box<dyn Future<Output = crate::Result<reqwest::Response>> + Send + '_>>;
}

/// Default [`HttpService`] that delegates directly to [`reqwest::Client::execute`].
#[derive(Debug, Clone)]
pub struct ReqwestService(reqwest::Client);

impl ReqwestService {
    pub fn new(client: reqwest::Client) -> Self {
        Self(client)
    }
}

impl HttpService for ReqwestService {
    fn call(
        &self,
        request: reqwest::Request,
    ) -> Pin<Box<dyn Future<Output = crate::Result<reqwest::Response>> + Send + '_>> {
        let client = self.0.clone();
        Box::pin(async move { Ok(client.execute(request).await?) })
    }
}

/// An [`HttpService`] that spawns each request on a separate tokio runtime.
///
/// This is useful when the calling runtime (e.g. a CPU-bound DataFusion runtime)
/// may have I/O disabled. All HTTP I/O -- including credential refresh -- is
/// routed through the provided runtime handle.
///
/// # Example
///
/// ```ignore
/// use olai_http::CloudClient;
///
/// let io_runtime = tokio::runtime::Runtime::new().unwrap();
/// let client = CloudClient::new_with_token("tok")
///     .with_runtime(io_runtime.handle().clone());
/// ```
#[derive(Debug)]
pub struct SpawnService {
    inner: Arc<dyn HttpService>,
    handle: Handle,
}

impl SpawnService {
    pub fn new(inner: Arc<dyn HttpService>, handle: Handle) -> Self {
        Self { inner, handle }
    }
}

impl HttpService for SpawnService {
    fn call(
        &self,
        request: reqwest::Request,
    ) -> Pin<Box<dyn Future<Output = crate::Result<reqwest::Response>> + Send + '_>> {
        let inner = Arc::clone(&self.inner);
        let handle = self.handle.clone();
        Box::pin(async move {
            match handle.spawn(async move { inner.call(request).await }).await {
                Ok(result) => result,
                Err(join_err) if join_err.is_cancelled() => {
                    // The spawned I/O task was cancelled, typically because the
                    // I/O runtime is being shut down concurrently with an
                    // in-flight request (a graceful-shutdown race). This is not a
                    // bug in our code, so surface it as an error rather than
                    // crashing the client.
                    Err(crate::Error::Generic {
                        source: Box::new(join_err),
                    })
                }
                Err(join_err) => {
                    // The spawned I/O task genuinely panicked. That indicates a
                    // programming error (e.g. a bug in request execution), not a
                    // transient network failure, so we re-panic with context
                    // rather than silently swallowing the cause.
                    panic!("I/O runtime task panicked: {join_err}")
                }
            }
        })
    }
}

/// Create an [`HttpService`] for the given client, optionally wrapping in [`SpawnService`].
pub(crate) fn make_service(
    client: reqwest::Client,
    runtime: Option<&Handle>,
) -> Arc<dyn HttpService> {
    let base: Arc<dyn HttpService> = Arc::new(ReqwestService::new(client));
    match runtime {
        Some(handle) => Arc::new(SpawnService::new(base, handle.clone())),
        None => base,
    }
}
