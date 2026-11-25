//! The host source: the secret server of an instance whose egress takes its values from the
//! host ([`SecretSource::Host`](olai_stack_topology::SecretSource::Host)) fetches them from
//! an endpoint the launcher runs, under a path token minted for the instance.
//!
//! [`Instances`](crate::Instances) needs a [`HostSecretEndpoint`] for such an instance
//! ([`with_host_endpoint`](crate::Instances::with_host_endpoint)). On start it mints the token,
//! registers the instance with the token and the variables the server serves
//! ([`HostValue`]s), and launches the server with the token alone; on stop it unregisters it.
//! Without an endpoint the start fails with [`InstanceError::NoHostEndpoint`](crate::InstanceError::NoHostEndpoint)
//! before Compose runs: `trestle up` stays on the launch-environment source.
//!
//! The endpoint itself, embeddable in the desktop (and later the CLI), is
//! [`HostEndpoint`](crate::HostEndpoint) (feature `host-endpoint`).

use std::collections::BTreeMap;
use std::fmt;

use async_trait::async_trait;
use olai_stack_state::{InstanceId, SecretRef};

use crate::secrets::{SecretError, SecretValue};

/// How the endpoint gets one variable's value.
#[derive(Clone, PartialEq, Eq)]
pub enum HostValue {
    /// Resolve the reference with the endpoint's providers, again on each refresh: a value
    /// changed there (a rotated keychain entry) reaches the instance live.
    Ref(SecretRef),
    /// A value the instance resolved from something the endpoint can't re-read (a literal,
    /// a source's credential): fixed until the instance is registered again.
    Fixed(SecretValue),
}

impl fmt::Debug for HostValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostValue::Ref(r) => f.debug_tuple("Ref").field(r).finish(),
            HostValue::Fixed(_) => f.write_str("Fixed(<redacted>)"),
        }
    }
}

/// Why registering an instance failed. Messages name variables, never values.
#[derive(Debug, thiserror::Error)]
pub enum HostEndpointError {
    /// A variable's value couldn't be resolved.
    #[error("host source variable {var}: {source}")]
    Secret {
        /// The variable.
        var: String,
        /// Why.
        #[source]
        source: SecretError,
    },
    /// The endpoint can't serve (it isn't listening, say).
    #[error("host secret endpoint: {0}")]
    Unavailable(String),
}

/// A host endpoint instances register with: it answers
/// `GET <base>/<token>/secrets/<variable>` for the registered variables of the instance
/// holding `token`, and `404` for anything else.
#[async_trait]
pub trait HostSecretEndpoint: Send + Sync {
    /// Serve `vars` to the holder of `token`, replacing an earlier registration of `instance`
    /// (and its token).
    async fn register(
        &self,
        instance: &InstanceId,
        token: &SecretValue,
        vars: BTreeMap<String, HostValue>,
    ) -> Result<(), HostEndpointError>;

    /// Stop serving `instance`: its token answers `404` from now on.
    async fn unregister(&self, instance: &InstanceId);
}

/// A fresh path token: 32 random bytes, hex.
pub(crate) fn mint_token() -> Result<SecretValue, HostEndpointError> {
    let mut bytes = zeroize::Zeroizing::new([0u8; 32]);
    getrandom::fill(bytes.as_mut()).map_err(|e| {
        HostEndpointError::Unavailable(format!("no randomness for a path token: {e}"))
    })?;
    Ok(SecretValue::new(
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_fresh_hex() {
        let (a, b) = (mint_token().unwrap(), mint_token().unwrap());
        assert_eq!(a.expose().len(), 64);
        assert!(a.expose().bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
        let fixed = HostValue::Fixed(a.clone());
        assert!(!format!("{fixed:?}").contains(a.expose()));
    }
}
