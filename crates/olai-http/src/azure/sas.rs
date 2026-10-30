use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::{Method, header::AUTHORIZATION};
use serde::Deserialize;

use crate::retry::RetryExt;
use crate::service::{HttpService, make_service};
use crate::util::hmac_sha256;
use crate::{ClientOptions, Result, RetryConfig};

/// SAS signed version — must be at least 2020-02-10 to support `sdd=` directory depth.
const SAS_VERSION: &str = "2020-12-06";

// ── User Delegation Key ──────────────────────────────────────────────────────

/// A user delegation key from the Azure Storage `Get User Delegation Key` endpoint.
///
/// It signs any number of user delegation SAS until it expires, so a server can fetch
/// one and reuse it (see [`build_user_delegation_sas`]). `Debug` redacts the key.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UserDelegationKey {
    /// The object id of the principal the key was issued to.
    pub signed_oid: String,
    /// Its tenant.
    pub signed_tid: String,
    /// When the key becomes valid (RFC 3339).
    pub signed_start: String,
    /// When the key expires (RFC 3339).
    pub signed_expiry: String,
    /// The service it is valid for (`b`).
    pub signed_service: String,
    /// The API version it was issued under.
    pub signed_version: String,
    /// The base64 key. Secret.
    pub value: String,
}

impl UserDelegationKey {
    /// When the key expires, if `signed_expiry` parses.
    pub fn expires_at(&self) -> Option<DateTime<Utc>> {
        DateTime::parse_from_rfc3339(&self.signed_expiry)
            .ok()
            .map(|t| t.with_timezone(&Utc))
    }
}

impl std::fmt::Debug for UserDelegationKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserDelegationKey")
            .field("signed_oid", &self.signed_oid)
            .field("signed_tid", &self.signed_tid)
            .field("signed_start", &self.signed_start)
            .field("signed_expiry", &self.signed_expiry)
            .field("signed_service", &self.signed_service)
            .field("signed_version", &self.signed_version)
            .field("value", &"<redacted>")
            .finish()
    }
}

/// How to reach the Blob service when fetching a user delegation key.
#[derive(Debug, Clone, Default)]
pub struct UserDelegationKeyOptions {
    /// The Blob service endpoint, including the account path for emulators
    /// (`https://127.0.0.1:10000/devstoreaccount1`). Defaults to
    /// `https://<account>.blob.core.windows.net`.
    pub endpoint: Option<String>,
    /// HTTP client options for the request (custom root certificates, timeouts, ...).
    pub client_options: ClientOptions,
}

/// Fetches a user delegation key for `account`, valid from `start` until `expiry` (at
/// most seven days), with an AAD bearer token for storage.
pub async fn fetch_user_delegation_key(
    account: &str,
    bearer_token: &str,
    start: DateTime<Utc>,
    expiry: DateTime<Utc>,
    options: &UserDelegationKeyOptions,
) -> Result<UserDelegationKey> {
    fetch_user_delegation_key_with_retry(
        account,
        bearer_token,
        start,
        expiry,
        options,
        &RetryConfig::default(),
    )
    .await
}

/// Like [`fetch_user_delegation_key`], retrying failed requests as `retry` allows
/// instead of the default (up to three minutes).
pub async fn fetch_user_delegation_key_with_retry(
    account: &str,
    bearer_token: &str,
    start: DateTime<Utc>,
    expiry: DateTime<Utc>,
    options: &UserDelegationKeyOptions,
    retry: &RetryConfig,
) -> Result<UserDelegationKey> {
    let endpoint = match &options.endpoint {
        Some(endpoint) => endpoint.trim_end_matches('/').to_owned(),
        None => format!("https://{account}.blob.core.windows.net"),
    };
    let url = format!("{endpoint}/?restype=service&comp=userdelegationkey");
    let body = format!(
        "<KeyInfo><Start>{}</Start><Expiry>{}</Expiry></KeyInfo>",
        start.to_rfc3339_opts(SecondsFormat::Secs, true),
        expiry.to_rfc3339_opts(SecondsFormat::Secs, true),
    );

    let client = options.client_options.client()?;
    let service: Arc<dyn HttpService> = make_service(client.clone(), None);

    let text = client
        .request(Method::POST, &url)
        .header(AUTHORIZATION, format!("Bearer {bearer_token}"))
        .header("x-ms-version", SAS_VERSION)
        .header(reqwest::header::CONTENT_TYPE, "application/xml")
        .body(body)
        .retryable(retry, service)
        .idempotent(true)
        .send()
        .await
        .map_err(|e| crate::Error::Generic {
            source: Box::new(e),
        })?
        .text()
        .await
        .map_err(|e| crate::Error::Generic {
            source: Box::new(e),
        })?;

    quick_xml::de::from_str::<UserDelegationKey>(&text).map_err(|e| crate::Error::Generic {
        source: e.to_string().into(),
    })
}

// ── Signed resource ──────────────────────────────────────────────────────────

/// Compute the signed directory depth for a blob prefix path.
///
/// Azure ADLS Gen2 SAS tokens scope access to a directory prefix via `sdd=N`
/// where N is the number of `/`-separated, non-empty path segments in the prefix.
///
/// Examples:
/// - `""` → `None` (container-level, no `sdd`)
/// - `"data"` → `Some(1)`
/// - `"data/2024"` → `Some(2)`
/// - `"data/2024/"` → `Some(2)` (trailing slash ignored)
fn signed_directory_depth(prefix: &str) -> Option<usize> {
    let depth = prefix.split('/').filter(|s| !s.is_empty()).count();
    if depth == 0 { None } else { Some(depth) }
}

/// What a SAS is signed for: a whole container (`sr=c`) or, on ADLS Gen2
/// (hierarchical namespace) accounts, a directory and everything below it
/// (`sr=d` with `sdd=<depth>`).
struct SignedResource {
    /// The `sr` value.
    sr: &'static str,
    /// The canonicalized resource of the string-to-sign.
    canonicalized: String,
    /// The `sdd` value, for directory SAS. It is a query parameter only: it is
    /// not part of the string-to-sign.
    depth: Option<usize>,
}

impl SignedResource {
    fn new(account: &str, container: &str, prefix: &str) -> Self {
        match signed_directory_depth(prefix) {
            None => Self {
                sr: "c",
                canonicalized: format!("/blob/{account}/{container}"),
                depth: None,
            },
            Some(depth) => Self {
                sr: "d",
                canonicalized: format!("/blob/{account}/{container}/{}", prefix.trim_matches('/')),
                depth: Some(depth),
            },
        }
    }
}

// ── SAS construction ─────────────────────────────────────────────────────────

/// The string-to-sign of a user delegation SAS (version 2020-12-06 and later).
///
/// <https://learn.microsoft.com/en-us/rest/api/storageservices/create-user-delegation-sas#version-2020-12-06-and-later>
fn user_delegation_string_to_sign(
    permissions: &str,
    start: &str,
    expiry: &str,
    resource: &SignedResource,
    key: &UserDelegationKey,
) -> String {
    [
        permissions,
        start,
        expiry,
        &resource.canonicalized,
        &key.signed_oid,     // signedKeyObjectId
        &key.signed_tid,     // signedKeyTenantId
        &key.signed_start,   // signedKeyStart
        &key.signed_expiry,  // signedKeyExpiry
        &key.signed_service, // signedKeyService
        &key.signed_version, // signedKeyVersion
        "",                  // signedAuthorizedUserObjectId
        "",                  // signedUnauthorizedUserObjectId
        "",                  // signedCorrelationId
        "",                  // signedIP
        "https",             // signedProtocol
        SAS_VERSION,         // signedVersion
        resource.sr,         // signedResource
        "",                  // signedSnapshotTime
        "",                  // signedEncryptionScope
        "",                  // rscc
        "",                  // rscd
        "",                  // rsce
        "",                  // rscl
        "",                  // rsct
    ]
    .join("\n")
}

/// The string-to-sign of a service SAS (version 2020-12-06 and later).
///
/// <https://learn.microsoft.com/en-us/rest/api/storageservices/create-service-sas#version-2020-12-06-and-later>
fn service_string_to_sign(
    permissions: &str,
    start: &str,
    expiry: &str,
    resource: &SignedResource,
    protocol: &str,
) -> String {
    [
        permissions,
        start,
        expiry,
        &resource.canonicalized,
        "",          // signedIdentifier
        "",          // signedIP
        protocol,    // signedProtocol
        SAS_VERSION, // signedVersion
        resource.sr, // signedResource
        "",          // signedSnapshotTime
        "",          // signedEncryptionScope
        "",          // rscc
        "",          // rscd
        "",          // rsce
        "",          // rscl
        "",          // rsct
    ]
    .join("\n")
}

/// Build a SAS token query string signed with a User Delegation Key.
///
/// When `prefix` is non-empty the SAS is a directory SAS (`sr=d`, `sdd=`) for that
/// path, which requires an ADLS Gen2 (hierarchical namespace) account; flat-namespace
/// accounts reject it.
///
/// The SAS has no start time (`st`), so it is valid as soon as it is issued even when
/// the local clock runs ahead of the service's.
///
/// Returns the SAS parameters as a query string (without leading `?`).
pub fn build_user_delegation_sas(
    account: &str,
    container: &str,
    prefix: &str,
    key: &UserDelegationKey,
    expiry: DateTime<Utc>,
    permissions: &str,
) -> Result<String> {
    let expiry_str = expiry.to_rfc3339_opts(SecondsFormat::Secs, true);
    let resource = SignedResource::new(account, container, prefix);
    let string_to_sign =
        user_delegation_string_to_sign(permissions, "", &expiry_str, &resource, key);

    let key_bytes = BASE64
        .decode(&key.value)
        .map_err(|e| crate::Error::Generic { source: e.into() })?;
    let signature = BASE64.encode(hmac_sha256(&key_bytes, string_to_sign.as_bytes()).as_ref());

    let mut sas = format!(
        "sv={version}&se={expiry}&sp={permissions}&spr=https&sr={sr}\
         &skoid={skoid}&sktid={sktid}&skt={skt}&ske={ske}&sks={sks}&skv={skv}",
        version = SAS_VERSION,
        expiry = url_encode(&expiry_str),
        permissions = permissions,
        sr = resource.sr,
        skoid = url_encode(&key.signed_oid),
        sktid = url_encode(&key.signed_tid),
        skt = url_encode(&key.signed_start),
        ske = url_encode(&key.signed_expiry),
        sks = url_encode(&key.signed_service),
        skv = url_encode(&key.signed_version),
    );
    if let Some(d) = resource.depth {
        sas.push_str(&format!("&sdd={d}"));
    }
    sas.push_str(&format!("&sig={}", url_encode(&signature)));
    Ok(sas)
}

/// Build a SAS token signed with a storage account key.
///
/// When `prefix` is non-empty the SAS is a directory SAS (`sr=d`, `sdd=`) for that
/// path, which requires an ADLS Gen2 (hierarchical namespace) account.
///
/// When `emulator` is true the SAS targets the Azurite Blob emulator, which is
/// served over **http** and uses a flat namespace:
/// - `signedProtocol` is `https,http` instead of `https` (Azurite rejects an
///   `https`-only SAS sent over http), and
/// - the SAS is always container-level, since Azurite has no directory SAS. Prefix
///   scoping for the emulator is enforced elsewhere (the object store is rooted at
///   the container + blob prefix).
///
/// Uses the Service SAS signing algorithm (API version 2020-12-06).
/// <https://learn.microsoft.com/en-us/rest/api/storageservices/create-service-sas>
#[allow(clippy::too_many_arguments)]
pub fn build_storage_key_sas(
    account: &str,
    container: &str,
    prefix: &str,
    account_key_b64: &str,
    start: DateTime<Utc>,
    expiry: DateTime<Utc>,
    permissions: &str,
    emulator: bool,
) -> Result<String> {
    let start_str = start.to_rfc3339_opts(SecondsFormat::Secs, true);
    let expiry_str = expiry.to_rfc3339_opts(SecondsFormat::Secs, true);
    let resource = SignedResource::new(account, container, if emulator { "" } else { prefix });

    // Azurite is http; real Azure is https-only. The protocol is part of the
    // string-to-sign, so it must match the emitted `spr=` exactly.
    let protocol = if emulator { "https,http" } else { "https" };
    let string_to_sign =
        service_string_to_sign(permissions, &start_str, &expiry_str, &resource, protocol);

    let key_bytes = BASE64
        .decode(account_key_b64)
        .map_err(|e| crate::Error::Generic { source: e.into() })?;
    let signature = BASE64.encode(hmac_sha256(&key_bytes, string_to_sign.as_bytes()).as_ref());

    // `st` (signedStart) is part of the string-to-sign above, so it MUST also
    // appear in the emitted query — the service recomputes the signature from
    // the query parameters, and an absent `st` is signed as empty → mismatch.
    let mut sas = format!(
        "sv={version}&st={start}&se={expiry}&sp={permissions}&spr={protocol}&sr={sr}",
        version = SAS_VERSION,
        start = url_encode(&start_str),
        expiry = url_encode(&expiry_str),
        permissions = permissions,
        protocol = url_encode(protocol),
        sr = resource.sr,
    );
    if let Some(d) = resource.depth {
        sas.push_str(&format!("&sdd={d}"));
    }
    sas.push_str(&format!("&sig={}", url_encode(&signature)));
    Ok(sas)
}

/// Percent-encode a value for use in a SAS query string.
fn url_encode(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

/// SAS permission string for read-only access (read + list).
pub const SAS_READ: &str = "rl";

/// SAS permission string for read-write access (read, add, create, write, delete, list).
pub const SAS_READ_WRITE: &str = "racwdl";

/// Default SAS TTL in seconds (1 hour).
pub const DEFAULT_TTL_SECS: u64 = 3600;

/// Compute an expiry `DateTime<Utc>` that is `ttl_secs` from now.
pub(crate) fn sas_expiry(ttl_secs: u64) -> DateTime<Utc> {
    let now = SystemTime::now();
    let expiry = now + Duration::from_secs(ttl_secs);
    let secs = expiry
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    DateTime::from_timestamp(secs as i64, 0).unwrap_or_else(Utc::now)
}

// ── Public entry points used by credential_vending ──────────────────────────

/// Generate a User Delegation SAS token scoped to an Azure Blob Storage container
/// and optionally a directory prefix within it.
///
/// Uses the provided AAD bearer token to fetch a User Delegation Key, then signs
/// a SAS. When `prefix` is non-empty the SAS is a directory SAS (`sr=d`, `sdd=`)
/// for that path, which requires an ADLS Gen2 (hierarchical namespace) account.
pub async fn generate_user_delegation_sas(
    account: &str,
    container: &str,
    prefix: &str,
    bearer_token: &str,
    read_only: bool,
    ttl_secs: u64,
) -> Result<String> {
    generate_user_delegation_sas_with_options(
        account,
        container,
        prefix,
        bearer_token,
        read_only,
        ttl_secs,
        &UserDelegationKeyOptions::default(),
    )
    .await
}

/// Like [`generate_user_delegation_sas`], with an explicit Blob service endpoint and
/// HTTP client options for fetching the user delegation key (emulators, custom
/// certificates).
pub async fn generate_user_delegation_sas_with_options(
    account: &str,
    container: &str,
    prefix: &str,
    bearer_token: &str,
    read_only: bool,
    ttl_secs: u64,
    options: &UserDelegationKeyOptions,
) -> Result<String> {
    let expiry = sas_expiry(ttl_secs);
    let start = Utc::now();
    let key = fetch_user_delegation_key(account, bearer_token, start, expiry, options).await?;
    let permissions = if read_only { SAS_READ } else { SAS_READ_WRITE };
    build_user_delegation_sas(account, container, prefix, &key, expiry, permissions)
}

/// Generate a service SAS token scoped to an Azure Blob Storage container and
/// optionally a directory prefix (an ADLS Gen2 directory SAS) using a storage account key.
///
/// This does not require an AAD token. Set `emulator` to true when signing for
/// the Azurite Blob emulator (served over http, flat namespace) — see
/// `build_storage_key_sas` for the differences.
pub fn generate_storage_key_sas(
    account: &str,
    container: &str,
    prefix: &str,
    account_key_b64: &str,
    read_only: bool,
    ttl_secs: u64,
    emulator: bool,
) -> Result<String> {
    let expiry = sas_expiry(ttl_secs);
    let permissions = if read_only { SAS_READ } else { SAS_READ_WRITE };
    build_storage_key_sas(
        account,
        container,
        prefix,
        account_key_b64,
        Utc::now(),
        expiry,
        permissions,
        emulator,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_delegation_key_debug_redacts_the_key() {
        let key = UserDelegationKey {
            signed_oid: "oid".into(),
            signed_tid: "tid".into(),
            signed_start: "2030-01-01T00:00:00Z".into(),
            signed_expiry: "2030-01-02T00:00:00Z".into(),
            signed_service: "b".into(),
            signed_version: "2020-12-06".into(),
            value: "S2VZU0VDUkVU".into(),
        };
        let debug = format!("{key:?}");
        assert!(!debug.contains("S2VZU0VDUkVU"), "{debug}");
        assert_eq!(
            key.expires_at().map(|t| t.to_rfc3339()).as_deref(),
            Some("2030-01-02T00:00:00+00:00")
        );
    }

    #[test]
    fn test_signed_directory_depth_empty() {
        assert_eq!(signed_directory_depth(""), None);
        assert_eq!(signed_directory_depth("/"), None);
    }

    #[test]
    fn test_signed_directory_depth_segments() {
        assert_eq!(signed_directory_depth("data"), Some(1));
        assert_eq!(signed_directory_depth("data/2024"), Some(2));
        assert_eq!(signed_directory_depth("data/2024/"), Some(2));
        assert_eq!(signed_directory_depth("/data/2024/events"), Some(3));
    }

    #[test]
    fn test_sas_permissions_read() {
        assert_eq!(SAS_READ, "rl");
    }

    #[test]
    fn test_sas_permissions_read_write() {
        assert_eq!(SAS_READ_WRITE, "racwdl");
    }

    /// The well-known Azurite account key (also a valid base64 account key for
    /// real-Azure signing tests — these tests don't talk to a server).
    const TEST_KEY: &str =
        "Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==";

    #[test]
    fn test_storage_key_sas_no_prefix_no_sdd() {
        let sas =
            generate_storage_key_sas("devstoreaccount1", "test", "", TEST_KEY, true, 3600, false)
                .expect("SAS generation failed");
        assert!(sas.contains("sv="), "missing sv");
        assert!(sas.contains("se="), "missing se");
        assert!(sas.contains("sp=rl"), "expected read permissions");
        assert!(sas.contains("sig="), "missing sig");
        assert!(sas.contains("spr=https"), "non-emulator SAS is https-only");
        assert!(
            !sas.contains("sdd="),
            "should not have sdd for empty prefix"
        );
    }

    #[test]
    fn test_storage_key_sas_with_prefix_has_sdd() {
        let sas = generate_storage_key_sas(
            "devstoreaccount1",
            "test",
            "data/events",
            TEST_KEY,
            true,
            3600,
            false,
        )
        .expect("SAS generation failed");
        assert!(sas.contains("sdd=2"), "expected sdd=2 for data/events");
    }

    #[test]
    fn test_storage_key_sas_read_write_permissions() {
        let sas =
            generate_storage_key_sas("devstoreaccount1", "test", "", TEST_KEY, false, 3600, false)
                .expect("SAS generation failed");
        assert!(sas.contains("sp=racwdl"), "expected read-write permissions");
    }

    // Signature vectors. The expected signatures were computed independently (Python
    // `hmac`) over string-to-sign layouts assembled from the field lists in the Azure
    // docs (24 fields for user delegation, 16 for service SAS, both ending at `rsct`),
    // which match object_store's `string_to_sign_*_sas`.

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn test_delegation_key() -> UserDelegationKey {
        UserDelegationKey {
            signed_oid: "oid-1".into(),
            signed_tid: "tid-1".into(),
            signed_start: "2030-01-01T00:00:00Z".into(),
            signed_expiry: "2030-01-02T00:00:00Z".into(),
            signed_service: "b".into(),
            signed_version: "2020-12-06".into(),
            // base64("0123456789abcdef0123456789abcdef")
            value: "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=".into(),
        }
    }

    fn query_param<'a>(sas: &'a str, key: &str) -> Option<&'a str> {
        sas.split('&')
            .find_map(|kv| kv.strip_prefix(&format!("{key}=")))
    }

    fn sig(sas: &str) -> String {
        percent_encoding::percent_decode_str(query_param(sas, "sig").unwrap())
            .decode_utf8()
            .unwrap()
            .into_owned()
    }

    #[test]
    fn user_delegation_string_to_sign_matches_the_spec_layout() {
        let resource = SignedResource::new("acct", "ctr", "");
        let s = user_delegation_string_to_sign(
            "rl",
            "",
            "2030-01-01T01:00:00Z",
            &resource,
            &test_delegation_key(),
        );
        let expected = concat!(
            "rl\n",                   // signedPermissions
            "\n",                     // signedStart
            "2030-01-01T01:00:00Z\n", // signedExpiry
            "/blob/acct/ctr\n",       // canonicalizedResource
            "oid-1\ntid-1\n2030-01-01T00:00:00Z\n2030-01-02T00:00:00Z\nb\n2020-12-06\n", // signedKey*
            "\n\n\n\n",               // authorized/unauthorized OID, correlation id, IP
            "https\n2020-12-06\nc\n", // protocol, version, resource
            "\n\n",                   // snapshot time, encryption scope
            "\n\n\n\n",               // rscc, rscd, rsce, rscl; rsct is the empty tail
        );
        assert_eq!(s, expected);
        assert_eq!(s.split('\n').count(), 24);
    }

    #[test]
    fn user_delegation_sas_signature_vectors() {
        let key = test_delegation_key();
        let expiry = at("2030-01-01T01:00:00Z");

        let container = build_user_delegation_sas("acct", "ctr", "", &key, expiry, "rl").unwrap();
        assert_eq!(
            sig(&container),
            "iNzhtSmalfcT3w9YZxzT/Cx2jJtF91nz9/hqzv3MCpU="
        );
        assert_eq!(query_param(&container, "sr"), Some("c"));
        assert_eq!(query_param(&container, "sdd"), None);
        // No start: nothing signed as `st` that the query does not carry.
        assert_eq!(query_param(&container, "st"), None);

        let dir = build_user_delegation_sas("acct", "ctr", "/warehouse/tab/", &key, expiry, "rl")
            .unwrap();
        assert_eq!(sig(&dir), "7Zq1Urkzw7RA0rrvA1cS4mxvqLhXOVr0idg0dv75PW4=");
        assert_eq!(query_param(&dir, "sr"), Some("d"));
        assert_eq!(query_param(&dir, "sdd"), Some("2"));
    }

    #[test]
    fn service_sas_signature_vectors() {
        let (start, expiry) = (at("2030-01-01T00:00:00Z"), at("2030-01-01T01:00:00Z"));

        let container =
            build_storage_key_sas("acct", "ctr", "", TEST_KEY, start, expiry, "rl", false).unwrap();
        assert_eq!(
            sig(&container),
            "GiyaEaEy4mwpc37pguik+xGl/aEBu9Az6SQLmCAQGiU="
        );
        assert_eq!(
            query_param(&container, "st"),
            Some("2030%2D01%2D01T00%3A00%3A00Z")
        );

        let dir = build_storage_key_sas(
            "acct",
            "ctr",
            "warehouse/tab",
            TEST_KEY,
            start,
            expiry,
            "rl",
            false,
        )
        .unwrap();
        assert_eq!(sig(&dir), "jjax8NezhxgYRaMUAIEOF00D8v+fyhgQPH5g8Lac5BY=");
        assert_eq!(query_param(&dir, "sr"), Some("d"));
        assert_eq!(query_param(&dir, "sdd"), Some("2"));

        let emulator = build_storage_key_sas(
            "devstoreaccount1",
            "ctr",
            "warehouse/tab",
            TEST_KEY,
            start,
            expiry,
            "rl",
            true,
        )
        .unwrap();
        assert_eq!(
            sig(&emulator),
            "Se3ET4RJND7f0OhqZK5Cvy74Cwn3brvm/E3XoZUkZCo="
        );
        assert_eq!(query_param(&emulator, "sr"), Some("c"));
    }

    /// The Azurite emulator SAS must allow http (`spr=https,http`) and must not
    /// carry `sdd=` even with a non-empty prefix (flat namespace). The signature
    /// covers the protocol, so `spr=` and the signed protocol must agree — a
    /// regression here surfaces as an Azurite 403 (verified end-to-end against a
    /// live Azurite in the open-lakehouse azurite stack).
    #[test]
    fn test_storage_key_sas_emulator_allows_http_and_omits_sdd() {
        let sas = generate_storage_key_sas(
            "devstoreaccount1",
            "lakehouse",
            "sales/orders",
            TEST_KEY,
            false,
            3600,
            true,
        )
        .expect("SAS generation failed");
        assert!(
            sas.contains("spr=https%2Chttp") || sas.contains("spr=https,http"),
            "emulator SAS must permit http: {sas}"
        );
        assert!(
            !sas.contains("sdd="),
            "emulator SAS must omit sdd (flat namespace): {sas}"
        );
        assert!(sas.contains("sp=racwdl"), "expected read-write permissions");
    }
}
