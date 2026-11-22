//! Caller identity: which consumer a connection belongs to.
//!
//! Every request is answered for exactly one [`CallerId`], a consumer name. The server takes
//! it from the connection, never from the request body:
//!
//! - **Today, the socket.** Each consumer has its own Unix socket, on a volume only that
//!   consumer mounts. The listener tags every connection it accepts with its consumer
//!   ([`ConsumerConn`]), and tonic hands that tag to the service as a request extension.
//! - **Later, a verified mTLS peer.** An mTLS listener will resolve the SPIFFE ID in the
//!   peer's verified certificate to a consumer through a table, and tag the connection with
//!   the same [`CallerId`]. The [`Policy`](crate::Policy) and the SDS service don't change.
//!
//! [`CallerResolver`] is the seam between the two: the service only ever asks a resolver for
//! the caller of a request.

use std::fmt;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tonic::transport::server::Connected;

/// The consumer a connection belongs to.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CallerId(Arc<str>);

impl CallerId {
    /// A caller identified by its consumer name.
    pub fn new(consumer: impl AsRef<str>) -> Self {
        Self(Arc::from(consumer.as_ref()))
    }

    /// The consumer name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CallerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CallerId({})", self.0)
    }
}

impl fmt::Display for CallerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Resolves the caller of a request from what its connection put into the request's
/// extensions.
///
/// `None` means the connection carries no identity the resolver accepts. The service then
/// rejects the request with `UNAUTHENTICATED`.
pub trait CallerResolver: Send + Sync + 'static {
    /// The caller of the request with these extensions.
    fn resolve(&self, extensions: &tonic::Extensions) -> Option<CallerId>;
}

/// Resolves the caller from the socket the connection arrived on: the [`CallerId`] a
/// [`ConsumerConn`] put into the extensions.
#[derive(Debug, Clone, Copy, Default)]
pub struct SocketCaller;

impl CallerResolver for SocketCaller {
    fn resolve(&self, extensions: &tonic::Extensions) -> Option<CallerId> {
        extensions.get::<CallerId>().cloned()
    }
}

/// A connection accepted on a consumer's socket, tagged with that consumer.
///
/// Its [`Connected`] implementation is what puts the [`CallerId`] into every request's
/// extensions.
#[derive(Debug)]
pub struct ConsumerConn<IO> {
    io: IO,
    caller: CallerId,
}

impl<IO> ConsumerConn<IO> {
    /// Tags `io` with `caller`.
    pub fn new(io: IO, caller: CallerId) -> Self {
        Self { io, caller }
    }
}

impl<IO> Connected for ConsumerConn<IO> {
    type ConnectInfo = CallerId;

    fn connect_info(&self) -> CallerId {
        self.caller.clone()
    }
}

impl<IO: AsyncRead + Unpin> AsyncRead for ConsumerConn<IO> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_read(cx, buf)
    }
}

impl<IO: AsyncWrite + Unpin> AsyncWrite for ConsumerConn<IO> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.io).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.io).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.io.is_write_vectored()
    }
}
