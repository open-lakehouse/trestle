//! Adapters between vending protocols' wire shapes and [`VendedCredential`](crate::VendedCredential).
//!
//! Each protocol module, behind a feature of the same name, holds serde types for its
//! JSON shape and two conversions:
//!
//! - **client side**, `to_vended(&VendContext)`: a wire response rarely carries
//!   everything the model needs. UC REST may omit the `url`, and none of the shapes but
//!   the Delta API say which access level was granted. So the conversion takes the
//!   caller's request as [`VendContext`]. A scope in the response may narrow the
//!   requested scope but never widen it: a broader prefix is clamped to the request, and
//!   a credential for unrelated storage is never used. Shapes that carry one credential
//!   (UC REST, Open Sharing) reject it; shapes that carry a list (Iceberg, Delta API)
//!   skip it, as they skip prefixes that are not locations this crate parses, and fail
//!   if nothing in the list relates. A catalog therefore cannot steer
//!   a client's requests for other storage to its credentials (or to an endpoint hint
//!   of its choosing). Of a list, [`select_covering`] picks the credential for a
//!   request.
//! - **server side**, `From<&VendedCredential>` (or `TryFrom`, where the shape cannot
//!   carry every credential kind): serializing what a minter produced.
//!
//! Expiry follows one rule everywhere: the earlier of an explicit wire expiry and a SAS
//! `se=` parameter, whichever are present; with neither, the credential does not expire.
//!
//! | Module | Protocol |
//! |---|---|
//! | [`unity`] | Unity Catalog REST `temporary-*-credentials` |
//! | [`delta_api`] | UC Delta API `storage-credentials` |
//! | [`iceberg`] | Iceberg REST `storage-credentials` and FileIO config keys |
//! | [`sharing`] | Open Sharing `temporary-*-credentials` |

use chrono::{DateTime, Utc};

use crate::location::{StorageScope, longest_match};
use crate::model::{AccessLevel, StorageCredentials};
use crate::sas;

#[cfg(any(feature = "unity", feature = "sharing"))]
pub mod common;
#[cfg(feature = "delta-api")]
#[cfg_attr(docsrs, doc(cfg(feature = "delta-api")))]
pub mod delta_api;
#[cfg(feature = "iceberg")]
#[cfg_attr(docsrs, doc(cfg(feature = "iceberg")))]
pub mod iceberg;
#[cfg(feature = "sharing")]
#[cfg_attr(docsrs, doc(cfg(feature = "sharing")))]
pub mod sharing;
#[cfg(feature = "unity")]
#[cfg_attr(docsrs, doc(cfg(feature = "unity")))]
pub mod unity;

/// What the client asked for: the storage it needs and the access it requested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VendContext {
    /// The storage the request was for, such as the table's location.
    pub scope: StorageScope,
    /// The access level requested.
    pub access: AccessLevel,
}

impl VendContext {
    /// A request for `access` on `scope`.
    pub fn new(scope: StorageScope, access: AccessLevel) -> Self {
        Self { scope, access }
    }

    /// The scope a credential offered for `offered` may be used for: `offered` if it
    /// lies within the request, the request if `offered` is broader, and `None` if the
    /// two are unrelated.
    pub fn clamp(&self, offered: &StorageScope) -> Option<StorageScope> {
        if self.scope.contains(offered) {
            Some(offered.clone())
        } else if offered.contains(&self.scope) {
            Some(self.scope.clone())
        } else {
            None
        }
    }
}

/// Of `offered` credentials, as `(prefix, granted access, credential)`, the one with the
/// most specific prefix that covers all of `ctx.scope` at `ctx.access`.
///
/// For protocols that answer with a list of prefixed credentials (Iceberg, Delta API).
/// Preferring the most specific prefix uses the narrowest grant that serves the
/// request; credentials for sub-paths or unrelated storage never qualify.
pub fn select_covering<'a, T>(
    offered: impl IntoIterator<Item = (&'a StorageScope, AccessLevel, T)>,
    ctx: &VendContext,
) -> Option<T> {
    longest_match(
        offered
            .into_iter()
            .filter(|(_, access, _)| access.satisfies(ctx.access))
            .map(|(prefix, _, value)| (prefix, value)),
        ctx.scope.location(),
    )
}

/// `scope` as a prefix string for the server direction, always ending in `/`.
///
/// Clients of the config-map protocols (Iceberg, Delta API) commonly match prefixes as
/// plain strings; without the trailing `/`, `s3://b/tab` would also match
/// `s3://b/table2`.
#[cfg(any(feature = "delta-api", feature = "iceberg"))]
pub(crate) fn prefix_string(scope: &StorageScope) -> String {
    let mut prefix = scope.to_string();
    if !prefix.ends_with('/') {
        prefix.push('/');
    }
    prefix
}

/// The error for a list of prefixed credentials (Iceberg, Delta API) none of which
/// relates to the request, noting how many prefixes were not locations at all.
#[cfg(any(feature = "delta-api", feature = "iceberg"))]
pub(crate) fn no_covering_credential<'a>(
    prefixes: impl IntoIterator<Item = &'a str>,
    ctx: &VendContext,
) -> crate::Error {
    let unparsed = prefixes
        .into_iter()
        .filter(|p| p.parse::<StorageScope>().is_err())
        .count();
    let note = match unparsed {
        0 => String::new(),
        n => format!(" ({n} prefixes are not storage locations this crate parses)"),
    };
    crate::Error::invalid_credential(format!(
        "no credential covers the requested {}{note}",
        ctx.scope
    ))
}

/// Parses an epoch-milliseconds expiry from a config-map value.
#[cfg(feature = "iceberg")]
pub(crate) fn parse_expiry_millis(key: &str, value: &str) -> crate::Result<Option<DateTime<Utc>>> {
    value
        .trim()
        .parse::<i64>()
        .map(crate::model::expiry_from_epoch_millis)
        .map_err(|_| crate::Error::invalid_credential(format!("`{key}` is not epoch milliseconds")))
}

/// Renders a config map's keys with every value redacted: which keys a server sent is
/// useful when debugging, and any value, known key or not, may be a secret.
#[cfg(any(feature = "delta-api", feature = "iceberg"))]
pub(crate) struct RedactedConfig<'a>(pub(crate) &'a std::collections::BTreeMap<String, String>);

#[cfg(any(feature = "delta-api", feature = "iceberg"))]
impl std::fmt::Debug for RedactedConfig<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map()
            .entries(self.0.keys().map(|k| (k, "<redacted>")))
            .finish()
    }
}

/// The earlier of an explicit wire expiry and the SAS `se=` of an Azure SAS, whichever
/// are present. A SAS is dead past its `se=` whatever the server reports, so a later
/// explicit expiry must not keep it cached.
pub(crate) fn resolve_expiry(
    explicit: Option<DateTime<Utc>>,
    credentials: &StorageCredentials,
) -> Option<DateTime<Utc>> {
    let sas = match credentials {
        StorageCredentials::AzureSas(pairs) => sas::sas_expiry(pairs),
        _ => None,
    };
    match (explicit, sas) {
        (Some(explicit), Some(sas)) => Some(explicit.min(sas)),
        (explicit, sas) => explicit.or(sas),
    }
}

/// `Option<i64>` that also reads the string form proto JSON uses for `int64`, and
/// `null`.
#[cfg(any(feature = "unity", feature = "sharing"))]
pub(crate) mod int64 {
    use serde::{Deserialize, Deserializer};

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Repr {
        Number(i64),
        String(String),
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
        match Option::<Repr>::deserialize(d)? {
            None => Ok(None),
            Some(Repr::Number(n)) => Ok(Some(n)),
            Some(Repr::String(s)) => s.parse().map(Some).map_err(serde::de::Error::custom),
        }
    }

    /// Writes a JSON number.
    #[cfg(feature = "unity")]
    pub(crate) fn serialize<S: serde::Serializer>(
        v: &Option<i64>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match v {
            Some(n) => s.serialize_i64(*n),
            None => s.serialize_none(),
        }
    }

    /// Writes the proto JSON string form.
    #[cfg(feature = "sharing")]
    pub(crate) mod as_string {
        use serde::Serializer;

        pub(crate) fn serialize<S: Serializer>(v: &Option<i64>, s: S) -> Result<S::Ok, S::Error> {
            match v {
                Some(n) => s.serialize_str(&n.to_string()),
                None => s.serialize_none(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(scope: &str) -> VendContext {
        VendContext::new(scope.parse().unwrap(), AccessLevel::Read)
    }

    fn scope(s: &str) -> StorageScope {
        s.parse().unwrap()
    }

    #[test]
    fn clamp_never_widens() {
        let c = ctx("s3://b/warehouse/tab");
        // Narrower: kept.
        assert_eq!(
            c.clamp(&scope("s3://b/warehouse/tab/data")),
            Some(scope("s3://b/warehouse/tab/data"))
        );
        // Broader: clamped to the request.
        assert_eq!(
            c.clamp(&scope("s3://b")),
            Some(scope("s3://b/warehouse/tab"))
        );
        // Unrelated or a sibling: dropped.
        assert_eq!(c.clamp(&scope("s3://b/warehouse/table2")), None);
        assert_eq!(c.clamp(&scope("s3://other/warehouse/tab")), None);
    }

    #[test]
    fn select_covering_prefers_the_narrowest_sufficient_grant() {
        let offered = [
            (scope("s3://b"), AccessLevel::ReadWrite, "bucket"),
            (
                scope("s3://b/warehouse"),
                AccessLevel::ReadWrite,
                "warehouse",
            ),
            (scope("s3://b/warehouse/tab"), AccessLevel::Read, "read"),
            (
                scope("s3://b/warehouse/tab/part"),
                AccessLevel::ReadWrite,
                "part",
            ),
        ];
        let select = |access| {
            let ctx = VendContext::new(scope("s3://b/warehouse/tab"), access);
            select_covering(offered.iter().map(|(p, a, v)| (p, *a, *v)), &ctx)
        };
        assert_eq!(select(AccessLevel::Read), Some("read"));
        assert_eq!(select(AccessLevel::ReadWrite), Some("warehouse"));
        let other = VendContext::new(scope("s3://other/tab"), AccessLevel::Read);
        assert_eq!(
            select_covering(offered.iter().map(|(p, a, v)| (p, *a, *v)), &other),
            None
        );
    }

    #[test]
    fn expiry_precedence() {
        let sas = StorageCredentials::azure_sas("se=2030-01-01&sig=x").unwrap();
        let explicit = DateTime::from_timestamp(1_000, 0);
        assert_eq!(resolve_expiry(explicit, &sas), explicit, "earlier explicit");
        // A server reporting a later expiry than the SAS's own `se=` does not extend it.
        let later = DateTime::from_timestamp(4_000_000_000, 0);
        assert_eq!(
            resolve_expiry(later, &sas)
                .map(|t| t.to_rfc3339())
                .as_deref(),
            Some("2030-01-01T00:00:00+00:00")
        );
        assert_eq!(
            resolve_expiry(later, &StorageCredentials::GcpBearer("t".into())),
            later
        );
        assert_eq!(
            resolve_expiry(None, &sas)
                .map(|t| t.to_rfc3339())
                .as_deref(),
            Some("2030-01-01T00:00:00+00:00")
        );
        assert_eq!(
            resolve_expiry(None, &StorageCredentials::GcpBearer("t".into())),
            None
        );
    }
}
