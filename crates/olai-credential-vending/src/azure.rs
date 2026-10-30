//! Minting Azure SAS from service principals, managed identities and account keys.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use olai_http::azure::{
    UserDelegationKey, UserDelegationKeyOptions, build_storage_key_sas, build_user_delegation_sas,
    fetch_user_delegation_key_with_retry,
};
use olai_http::bearer::{
    AzureManagedIdentity as MsiIdentity, BearerAuth, BearerCredentialProvider,
};
use olai_http::{ClientOptions, RetryConfig};
use olai_storage_credentials::policy::azure::sas_permissions;
use olai_storage_credentials::{
    StorageCredentials, StorageLocation, StorageProvider, StorageScope, StoreHints,
    VendedCredential,
};

use crate::error::{Error, Result};
use crate::mint::{CredentialMinter, MintRequest};
use crate::registered::{
    AzureClientSecret, AzureManagedIdentity, AzureServicePrincipal, AzureStorageKey,
};

/// The lifetime of a SAS when the request names none.
pub const DEFAULT_TTL: Duration = Duration::from_secs(3600);

/// The longest SAS minted: user delegation keys live at most seven days, less the
/// margin a key must outlive its SAS by and the clock skew allowed.
const MAX_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60 - 10 * 60);

/// The longest a user delegation key may live: Azure's limit, by its own clock.
const MAX_KEY_LIFETIME: TimeDelta = TimeDelta::days(7);

/// How long a fetched user delegation key lives, unless a request needs longer.
const KEY_LIFETIME: TimeDelta = TimeDelta::days(1);

/// A key is replaced this long before it can no longer cover a request.
const KEY_MARGIN: TimeDelta = TimeDelta::minutes(5);

/// Allowance for clocks running ahead of the storage service's.
const CLOCK_SKEW: TimeDelta = TimeDelta::minutes(5);

/// The Azure AD scope of Azure Storage.
const STORAGE_SCOPE: &str = "https://storage.azure.com/.default";

/// Retries for a key fetch: a mint waits on it, so it fails within seconds, not the
/// default three minutes.
fn key_retry() -> RetryConfig {
    RetryConfig {
        max_retries: 3,
        retry_timeout: Duration::from_secs(30),
        ..RetryConfig::default()
    }
}

/// The storage account a Blob service endpoint serves, when it names one:
/// `https://<account>.blob.core.windows.net`, or an emulator's
/// `https://127.0.0.1:10000/<account>`.
fn endpoint_account(endpoint: &str) -> Option<&str> {
    let rest = endpoint
        .split_once("://")
        .map_or(endpoint, |(_, rest)| rest);
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let mut labels = host.split('.');
    if let (Some(account), Some("blob" | "dfs")) = (labels.next(), labels.next())
        && !account.is_empty()
    {
        return Some(account);
    }
    path.split('/').next().filter(|segment| !segment.is_empty())
}

/// The scope `request` asks for, as account, container and directory prefix. Fails if
/// the storage is not Azure, or if `flat` and the scope is narrower than a container
/// (flat-namespace accounts have no directory SAS, and a container SAS would widen the
/// grant).
fn target(request: &MintRequest, flat: bool) -> Result<(&str, &str, &str)> {
    let location = request.scope.location();
    if location.provider() != StorageProvider::Azure {
        return Err(Error::unsupported(
            &request.scope,
            "a SAS covers Azure storage only",
        ));
    }
    let account = location.account().ok_or_else(|| {
        Error::unsupported(&request.scope, "the location names no storage account")
    })?;
    let prefix = location.path().as_ref();
    if flat && !prefix.is_empty() {
        return Err(Error::unsupported(
            &request.scope,
            "flat-namespace accounts cannot scope a SAS below a container",
        ));
    }
    Ok((account, location.bucket(), prefix))
}

/// `request` for the whole container its scope lies in; other storage is left as is,
/// for [`target`] to refuse.
fn container_request(request: &MintRequest) -> MintRequest {
    let location = request.scope.location();
    if location.provider() != StorageProvider::Azure || location.path().as_ref().is_empty() {
        return request.clone();
    }
    let mut url = location.to_url();
    url.set_path("/");
    match StorageLocation::parse(&url) {
        Ok(container) => MintRequest {
            scope: StorageScope::new(container),
            ..request.clone()
        },
        Err(_) => request.clone(),
    }
}

fn ttl(request: &MintRequest) -> TimeDelta {
    TimeDelta::from_std(request.ttl.unwrap_or(DEFAULT_TTL).min(MAX_TTL)).unwrap_or(TimeDelta::MAX)
}

fn vended(
    request: &MintRequest,
    sas: &str,
    expiry: DateTime<Utc>,
    endpoint: &Option<String>,
) -> Result<VendedCredential> {
    Ok(VendedCredential::new(
        StorageCredentials::azure_sas(sas)?,
        request.scope.clone(),
        request.access,
    )
    .with_expiry(Some(expiry))
    .with_hints(StoreHints {
        endpoint: endpoint.clone(),
        ..StoreHints::default()
    }))
}

/// Mints user delegation SAS from an Azure AD identity.
///
/// The identity (a service principal or the host's managed identity) gets a token for
/// Azure Storage, which fetches a user delegation key; the key signs a SAS for the
/// requested directory (or container) and permissions. Keys are cached and reused
/// across mints, per storage account, until they could not cover a request any more,
/// so minting costs one signature, not a round trip to Azure. Each account's key is
/// fetched at most once at a time, and a slow or failing account never delays minting
/// for another.
///
/// Directory SAS need a hierarchical-namespace (ADLS Gen2) account; for flat-namespace
/// accounts, [`with_flat_namespace`](Self::with_flat_namespace) restricts minting to
/// whole containers instead of widening the grant.
pub struct AzureUserDelegationSasMinter {
    token: BearerCredentialProvider,
    endpoint: Option<String>,
    client_options: ClientOptions,
    flat: bool,
    /// User delegation keys by storage account: a key signs for its own account only.
    keys: Mutex<HashMap<String, KeySlot>>,
}

/// One account's cached key. Its async lock is held across that account's fetch only,
/// so concurrent mints for the account share one fetch.
type KeySlot = Arc<tokio::sync::Mutex<Option<UserDelegationKey>>>;

impl AzureUserDelegationSasMinter {
    /// A minter for a service principal (client secret or federated token file).
    pub fn from_service_principal(principal: &AzureServicePrincipal) -> Result<Self> {
        let auth = BearerAuth::new();
        let token = match &principal.secret {
            AzureClientSecret::ClientSecret(secret) => auth.azure_client_secret(
                &principal.directory_id,
                principal.application_id.clone(),
                secret.clone(),
                STORAGE_SCOPE,
                None,
            )?,
            AzureClientSecret::FederatedTokenFile(file) => auth.azure_workload_identity(
                &principal.directory_id,
                principal.application_id.clone(),
                file.clone(),
                STORAGE_SCOPE,
                None,
            )?,
        };
        Ok(Self::with_token_provider(token))
    }

    /// A minter for the host's managed identity: the user-assigned one when
    /// `managed_identity_id` is set, else the system-assigned one.
    pub fn from_managed_identity(identity: &AzureManagedIdentity) -> Result<Self> {
        let token = BearerAuth::new().azure_managed_identity(MsiIdentity {
            resource: "https://storage.azure.com".to_owned(),
            msi_resource_id: identity.managed_identity_id.clone(),
            ..MsiIdentity::default()
        })?;
        Ok(Self::with_token_provider(token))
    }

    /// A minter whose storage tokens come from `token`.
    pub fn with_token_provider(token: BearerCredentialProvider) -> Self {
        Self {
            token,
            endpoint: None,
            client_options: ClientOptions::default(),
            flat: false,
            keys: Mutex::new(HashMap::new()),
        }
    }

    /// The Blob service endpoint, including the account path for emulators
    /// (`https://127.0.0.1:10000/devstoreaccount1`). Also passed to clients as a store
    /// hint.
    ///
    /// An endpoint serves one account: when it names one
    /// (`https://<account>.blob.core.windows.net`, or an emulator path), requests for
    /// any other account are refused instead of minting SAS that would not verify.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    /// HTTP client options for fetching keys.
    pub fn with_client_options(mut self, options: ClientOptions) -> Self {
        self.client_options = options;
        self
    }

    /// The account has a flat namespace: mint only for whole containers.
    pub fn with_flat_namespace(mut self) -> Self {
        self.flat = true;
        self
    }

    /// A user delegation key for `account` valid until at least `until`, from the
    /// cache when it still is.
    async fn key(&self, account: &str, until: DateTime<Utc>) -> Result<UserDelegationKey> {
        let slot = Arc::clone(
            self.keys
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entry(account.to_owned())
                .or_default(),
        );
        let mut cached = slot.lock().await;
        if let Some(key) = cached.as_ref()
            && key
                .expires_at()
                .is_some_and(|expiry| expiry - KEY_MARGIN >= until)
        {
            return Ok(key.clone());
        }
        let token = self.token.get_credential().await?;
        let now = Utc::now();
        // Azure refuses keys living past seven days by its own clock, which may be behind.
        let expiry = (now + KEY_LIFETIME)
            .max(until + KEY_MARGIN)
            .min(now + MAX_KEY_LIFETIME - CLOCK_SKEW);
        let options = UserDelegationKeyOptions {
            endpoint: self.endpoint.clone(),
            client_options: self.client_options.clone(),
        };
        let key = fetch_user_delegation_key_with_retry(
            account,
            &token.token,
            now - CLOCK_SKEW,
            expiry,
            &options,
            &key_retry(),
        )
        .await?;
        *cached = Some(key.clone());
        Ok(key)
    }
}

#[async_trait]
impl CredentialMinter for AzureUserDelegationSasMinter {
    async fn mint(&self, request: &MintRequest) -> Result<VendedCredential> {
        let (account, container, prefix) = target(request, self.flat)?;
        if let Some(served) = self.endpoint.as_deref().and_then(endpoint_account)
            && !served.eq_ignore_ascii_case(account)
        {
            return Err(Error::unsupported(
                &request.scope,
                format!("the endpoint serves storage account `{served}`"),
            ));
        }
        let now = Utc::now();
        let requested = now + ttl(request);
        let key = self.key(account, requested).await?;
        // A SAS stops verifying when its key expires, so it never outlives the key Azure
        // issued, which may live shorter than asked for.
        let expiry = match key.expires_at() {
            Some(key_expiry) => requested.min(key_expiry - KEY_MARGIN),
            None => requested,
        };
        if expiry <= now {
            return Err(Error::unsupported(
                &request.scope,
                "the user delegation key expires too soon to sign for",
            ));
        }
        let sas = build_user_delegation_sas(
            account,
            container,
            prefix,
            &key,
            expiry,
            sas_permissions(request.access),
        )?;
        vended(request, &sas, expiry, &self.endpoint)
    }
}

impl fmt::Debug for AzureUserDelegationSasMinter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AzureUserDelegationSasMinter")
            .field("endpoint", &self.endpoint)
            .field("flat", &self.flat)
            .finish_non_exhaustive()
    }
}

/// Mints service SAS signed with a storage account key.
///
/// The key belongs to one account, so only that account's storage is minted for.
pub struct AzureAccountKeySasMinter {
    account: String,
    key: String,
    endpoint: Option<String>,
    flat: bool,
    emulator: bool,
    container_grants: bool,
}

impl AzureAccountKeySasMinter {
    /// A minter for `key`'s account.
    pub fn new(key: &AzureStorageKey) -> Self {
        Self {
            account: key.account_name.clone(),
            key: key.account_key.clone(),
            endpoint: None,
            flat: false,
            emulator: false,
            container_grants: false,
        }
    }

    /// The Blob service endpoint, passed to clients as a store hint.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    /// The account has a flat namespace: mint only for whole containers.
    pub fn with_flat_namespace(mut self) -> Self {
        self.flat = true;
        self
    }

    /// Sign for the Azurite emulator: plain `http` is allowed and, as Azurite has a flat
    /// namespace, only whole containers are minted for.
    pub fn with_emulator(mut self) -> Self {
        self.emulator = true;
        self.with_flat_namespace()
    }

    /// On the emulator, answer a request below a container with a SAS for the whole
    /// container, rather than refusing it.
    ///
    /// This **widens the grant** to the container: the credential's scope says so, and
    /// covers the request. It exists because Azurite has no directory SAS, so local
    /// development against it could otherwise only use container-root locations. It has
    /// no effect without [`with_emulator`](Self::with_emulator); never use it to serve
    /// real storage.
    pub fn with_container_grants(mut self) -> Self {
        self.container_grants = true;
        self
    }
}

#[async_trait]
impl CredentialMinter for AzureAccountKeySasMinter {
    async fn mint(&self, request: &MintRequest) -> Result<VendedCredential> {
        let widened;
        let request = if self.emulator && self.container_grants {
            widened = container_request(request);
            &widened
        } else {
            request
        };
        let (account, container, prefix) = target(request, self.flat)?;
        if account != self.account {
            return Err(Error::unsupported(
                &request.scope,
                format!("the key is for storage account `{}`", self.account),
            ));
        }
        let now = Utc::now();
        let expiry = now + ttl(request);
        let sas = build_storage_key_sas(
            account,
            container,
            prefix,
            &self.key,
            now - CLOCK_SKEW,
            expiry,
            sas_permissions(request.access),
            self.emulator,
        )?;
        vended(request, &sas, expiry, &self.endpoint)
    }
}

impl fmt::Debug for AzureAccountKeySasMinter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AzureAccountKeySasMinter")
            .field("account", &self.account)
            .field("key", &"<redacted>")
            .field("endpoint", &self.endpoint)
            .field("flat", &self.flat)
            .field("emulator", &self.emulator)
            .field("container_grants", &self.container_grants)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use olai_storage_credentials::AccessLevel;

    use super::*;

    fn key_minter() -> AzureAccountKeySasMinter {
        AzureAccountKeySasMinter::new(&AzureStorageKey {
            account_name: "acct".into(),
            account_key: "a2V5LWJ5dGVzLWZvci10ZXN0aW5n".into(),
        })
    }

    fn request(url: &str, access: AccessLevel) -> MintRequest {
        MintRequest::new(url.parse().unwrap(), access)
    }

    fn query(vended: &VendedCredential) -> Vec<(String, String)> {
        match &vended.credentials {
            StorageCredentials::AzureSas(pairs) => pairs.clone(),
            other => panic!("expected a SAS, got {other:?}"),
        }
    }

    fn get<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
        pairs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    #[tokio::test]
    async fn account_key_sas_is_scoped_to_the_directory() {
        let vended = key_minter()
            .mint(&request(
                "abfss://ctr@acct.dfs.core.windows.net/wh/tab",
                AccessLevel::Read,
            ))
            .await
            .unwrap();
        let sas = query(&vended);
        assert_eq!(get(&sas, "sr"), Some("d"));
        assert_eq!(get(&sas, "sdd"), Some("2"));
        assert_eq!(get(&sas, "sp"), Some("rl"));
        assert!(vended.expires_at.is_some());

        let vended = key_minter()
            .mint(&request(
                "abfss://ctr@acct.dfs.core.windows.net",
                AccessLevel::ReadWrite,
            ))
            .await
            .unwrap();
        let sas = query(&vended);
        assert_eq!(get(&sas, "sr"), Some("c"));
        assert_eq!(get(&sas, "sp"), Some("racwdl"));
    }

    #[tokio::test]
    async fn account_keys_mint_for_their_account_only() {
        let err = key_minter()
            .mint(&request(
                "abfss://ctr@other.dfs.core.windows.net/t",
                AccessLevel::Read,
            ))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }), "{err}");
    }

    #[tokio::test]
    async fn flat_namespaces_never_widen_to_the_container() {
        let err = key_minter()
            .with_emulator()
            .mint(&request(
                "abfss://ctr@acct.dfs.core.windows.net/tab",
                AccessLevel::Read,
            ))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }), "{err}");
        let minter =
            AzureUserDelegationSasMinter::with_token_provider(BearerAuth::static_token("t"))
                .with_flat_namespace();
        let err = minter
            .mint(&request(
                "abfss://ctr@acct.dfs.core.windows.net/tab",
                AccessLevel::Read,
            ))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }), "{err}");
    }

    #[tokio::test]
    async fn emulator_container_grants_widen_to_the_container() {
        let vended = key_minter()
            .with_emulator()
            .with_container_grants()
            .mint(&request(
                "abfss://ctr@acct.dfs.core.windows.net/tab/part",
                AccessLevel::Read,
            ))
            .await
            .unwrap();
        let sas = query(&vended);
        assert_eq!(get(&sas, "sr"), Some("c"));
        assert_eq!(get(&sas, "spr"), Some("https,http"));
        // The scope states the real, wider grant.
        let container: StorageScope = "abfss://ctr@acct.dfs.core.windows.net".parse().unwrap();
        assert_eq!(vended.scope, container);

        // Without the emulator the opt-in changes nothing: real storage never widens.
        let err = key_minter()
            .with_flat_namespace()
            .with_container_grants()
            .mint(&request(
                "abfss://ctr@acct.dfs.core.windows.net/tab",
                AccessLevel::Read,
            ))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }), "{err}");
    }

    #[tokio::test]
    async fn only_azure_scopes_are_minted() {
        let err = key_minter()
            .mint(&request("s3://bucket/t", AccessLevel::Read))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }), "{err}");
    }

    #[tokio::test]
    async fn delegation_keys_are_cached_per_account() {
        let mut server = mockito::Server::new_async().await;
        let expiry = (Utc::now() + TimeDelta::days(1)).to_rfc3339();
        let key = server
            .mock("POST", "/")
            .match_query(mockito::Matcher::Any)
            .with_body(format!(
                "<UserDelegationKey><SignedOid>oid</SignedOid><SignedTid>tid</SignedTid>\
                 <SignedStart>2020-01-01T00:00:00Z</SignedStart><SignedExpiry>{expiry}</SignedExpiry>\
                 <SignedService>b</SignedService><SignedVersion>2020-12-06</SignedVersion>\
                 <Value>a2V5</Value></UserDelegationKey>"
            ))
            .expect(2)
            .create_async()
            .await;
        let minter =
            AzureUserDelegationSasMinter::with_token_provider(BearerAuth::static_token("t"))
                .with_endpoint(server.url())
                .with_client_options(ClientOptions::new().with_allow_http(true));
        for account in ["one", "two", "one"] {
            let url = format!("abfss://ctr@{account}.dfs.core.windows.net/tab");
            minter
                .mint(&request(&url, AccessLevel::Read))
                .await
                .unwrap();
        }
        key.assert_async().await;
    }

    /// A `Get User Delegation Key` response for a key expiring at `expiry`.
    fn key_body(expiry: DateTime<Utc>) -> String {
        format!(
            "<UserDelegationKey><SignedOid>oid</SignedOid><SignedTid>tid</SignedTid>\
             <SignedStart>2020-01-01T00:00:00Z</SignedStart><SignedExpiry>{}</SignedExpiry>\
             <SignedService>b</SignedService><SignedVersion>2020-12-06</SignedVersion>\
             <Value>a2V5</Value></UserDelegationKey>",
            expiry.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        )
    }

    /// Hands out a token, except that the first call waits until `release` is notified.
    #[derive(Debug, Default)]
    struct StallingToken {
        calls: std::sync::atomic::AtomicUsize,
        release: tokio::sync::Notify,
    }

    #[async_trait]
    impl olai_http::CredentialProvider for StallingToken {
        type Credential = olai_http::bearer::BearerCredential;

        async fn get_credential(&self) -> olai_http::Result<Arc<Self::Credential>> {
            if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                self.release.notified().await;
            }
            Ok(Arc::new(olai_http::bearer::BearerCredential {
                token: "t".into(),
            }))
        }
    }

    #[tokio::test]
    async fn a_stalled_account_does_not_block_another() {
        let mut server = mockito::Server::new_async().await;
        let fetches = server
            .mock("POST", "/")
            .match_query(mockito::Matcher::Any)
            .with_body(key_body(Utc::now() + TimeDelta::days(1)))
            .expect(2)
            .create_async()
            .await;
        let token = Arc::new(StallingToken::default());
        let minter = Arc::new(
            AzureUserDelegationSasMinter::with_token_provider(token.clone())
                .with_endpoint(server.url())
                .with_client_options(ClientOptions::new().with_allow_http(true)),
        );
        let mint = |account: &'static str| {
            let minter = Arc::clone(&minter);
            tokio::spawn(async move {
                let url = format!("abfss://ctr@{account}.dfs.core.windows.net/tab");
                minter.mint(&request(&url, AccessLevel::Read)).await
            })
        };

        // The first fetch for `one` stalls in its token request...
        let stalled = mint("one");
        while token.calls.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        // ...yet `two` mints, and a second mint for `one` waits to share its fetch.
        let shared = mint("one");
        tokio::time::timeout(Duration::from_secs(10), mint("two"))
            .await
            .expect("minting for another account must not wait")
            .unwrap()
            .unwrap();
        token.release.notify_one();
        stalled.await.unwrap().unwrap();
        shared.await.unwrap().unwrap();
        fetches.assert_async().await;
    }

    #[tokio::test]
    async fn a_sas_never_outlives_its_delegation_key() {
        let mut server = mockito::Server::new_async().await;
        // Azure may issue a key shorter-lived than asked for.
        let key_expiry = Utc::now() + TimeDelta::hours(1);
        server
            .mock("POST", "/")
            .match_query(mockito::Matcher::Any)
            .with_body(key_body(key_expiry))
            .create_async()
            .await;
        let minter =
            AzureUserDelegationSasMinter::with_token_provider(BearerAuth::static_token("t"))
                .with_endpoint(server.url())
                .with_client_options(ClientOptions::new().with_allow_http(true));
        let vended = minter
            .mint(
                &request(
                    "abfss://ctr@acct.dfs.core.windows.net/tab",
                    AccessLevel::Read,
                )
                .with_ttl(Duration::from_secs(3 * 3600)),
            )
            .await
            .unwrap();
        let expiry = vended.expires_at.unwrap();
        assert!(
            expiry <= key_expiry - KEY_MARGIN,
            "{expiry} vs {key_expiry}"
        );
        let se = expiry.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        assert_eq!(get(&query(&vended), "se"), Some(se.as_str()));
    }

    #[test]
    fn the_longest_sas_leaves_room_for_the_key_margin_and_skew() {
        let longest = TimeDelta::from_std(MAX_TTL).unwrap();
        assert!(longest + KEY_MARGIN + CLOCK_SKEW <= MAX_KEY_LIFETIME);
    }

    #[test]
    fn endpoints_name_their_account() {
        for (endpoint, account) in [
            ("https://acct.blob.core.windows.net", Some("acct")),
            ("https://acct.dfs.core.windows.net/", Some("acct")),
            (
                "https://127.0.0.1:10000/devstoreaccount1",
                Some("devstoreaccount1"),
            ),
            ("http://127.0.0.1:10000", None),
            ("https://storage.example.com", None),
        ] {
            assert_eq!(endpoint_account(endpoint), account, "{endpoint}");
        }
    }

    #[tokio::test]
    async fn an_endpoint_mints_for_its_own_account_only() {
        let minter =
            AzureUserDelegationSasMinter::with_token_provider(BearerAuth::static_token("t"))
                .with_endpoint("https://127.0.0.1:10000/devstoreaccount1");
        let err = minter
            .mint(&request(
                "abfss://ctr@other.dfs.core.windows.net/tab",
                AccessLevel::Read,
            ))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }), "{err}");
    }

    #[tokio::test]
    async fn account_keys_can_be_flat_without_emulator_signing() {
        let minter = key_minter().with_flat_namespace();
        let err = minter
            .mint(&request(
                "abfss://ctr@acct.dfs.core.windows.net/tab",
                AccessLevel::Read,
            ))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }), "{err}");
        let vended = minter
            .mint(&request(
                "abfss://ctr@acct.dfs.core.windows.net",
                AccessLevel::Read,
            ))
            .await
            .unwrap();
        assert_eq!(get(&query(&vended), "sr"), Some("c"));
    }

    #[test]
    fn debug_hides_the_key() {
        let debug = format!("{:?}", key_minter());
        assert!(!debug.contains("a2V5"), "{debug}");
    }
}
