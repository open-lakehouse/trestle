//! Configuring an [`OpenSharing`] from string keys.

use std::fmt;

use chrono::Utc;
use olai_http::CloudClient;

use super::{OpenSharing, SharingClient, SharingProfile};
use crate::auth::{AuthConfig, AuthDefaults};
use crate::error::{Error, Result};
use crate::options::{StoreKeys, parse_base_url};
use crate::store::StoreOptions;

/// Builds an [`OpenSharing`] from configuration keys.
///
/// | Key | Meaning |
/// |---|---|
/// | `sharing_profile` | The path of the sharing profile file: the server endpoint and bearer token |
/// | `sharing_endpoint` | The server endpoint, overriding the profile's: for servers whose profiles name another API |
/// | `aws_region` | The region of vended S3 locations, overriding hints and discovery |
/// | `aws_endpoint`, `aws_endpoint_url` | The S3 endpoint, for servers backed by an S3-compatible store |
/// | `sharing_*` auth keys, client keys | Authentication, overriding the profile's token; see [`crate::auth`] |
///
/// Client keys (`allow_http`, `timeout`, ...) apply to the stores too. A profile whose
/// token has expired is refused.
#[derive(Clone)]
pub struct SharingBuilder {
    profile_path: Option<String>,
    profile: Option<SharingProfile>,
    endpoint: Option<String>,
    store_keys: StoreKeys,
    auth: AuthConfig,
    /// Whether a `sharing_*` auth key was set, which replaces the profile's token.
    auth_configured: bool,
    store_options: StoreOptions,
    client: Option<CloudClient>,
}

impl Default for SharingBuilder {
    fn default() -> Self {
        Self {
            profile_path: None,
            profile: None,
            endpoint: None,
            store_keys: StoreKeys::default(),
            auth: AuthConfig::new("sharing"),
            auth_configured: false,
            store_options: StoreOptions::default(),
            client: None,
        }
    }
}

impl SharingBuilder {
    /// An empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Uses `profile` for the endpoint and token.
    pub fn with_profile(mut self, profile: SharingProfile) -> Self {
        self.profile = Some(profile);
        self
    }

    /// Store options; the region, endpoint and client keys set here apply on top.
    pub fn with_store_options(mut self, options: StoreOptions) -> Self {
        self.store_options = options;
        self
    }

    /// Sends every sharing request through `client`, bypassing the profile's token
    /// and the authentication keys.
    pub fn with_client(mut self, client: CloudClient) -> Self {
        self.client = Some(client);
        self
    }

    /// Sets one option; unknown keys are ignored.
    pub fn with_option(mut self, key: &str, value: impl Into<String>) -> Self {
        let value = value.into();
        match key {
            "sharing_profile" => self.profile_path = Some(value),
            "sharing_endpoint" => self.endpoint = Some(value),
            _ => {
                if !self.store_keys.set(key, &value)
                    && self.auth.set(key, value)
                    && key.starts_with("sharing_")
                {
                    self.auth_configured = true;
                }
            }
        }
        self
    }

    /// Sets options from a map, ignoring keys that are not sharing, auth, store or
    /// client keys, so a whole storage-options map can be passed.
    pub fn with_options<I, K, V>(self, options: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: Into<String>,
    {
        options.into_iter().fold(self, |builder, (key, value)| {
            builder.with_option(key.as_ref(), value)
        })
    }

    /// Builds the client. No request is sent until a store is asked for.
    pub fn build(self) -> Result<OpenSharing> {
        let profile = match (&self.profile, &self.profile_path) {
            (Some(profile), _) => Some(profile.clone()),
            (None, Some(path)) => Some(SharingProfile::from_file(path)?),
            (None, None) => None,
        };
        if let Some(profile) = &profile
            && profile.is_expired(Utc::now())
        {
            return Err(Error::Config(format!(
                "the sharing profile for `{}` expired at {}",
                profile.endpoint,
                profile.expiration_time.unwrap_or_default()
            )));
        }
        let endpoint = self
            .endpoint
            .as_deref()
            .or(profile.as_ref().map(|p| p.endpoint.as_str()))
            .ok_or_else(|| {
                Error::Config("missing `sharing_profile` or `sharing_endpoint`".to_owned())
            })?;
        let endpoint = parse_base_url("sharing_endpoint", endpoint)?;
        let http = match &self.client {
            Some(client) => client.clone(),
            None => {
                let mut auth = self.auth.clone();
                if !self.auth_configured
                    && let Some(profile) = &profile
                {
                    auth.set("sharing_token", profile.bearer_token.clone());
                }
                auth.client(&endpoint, &AuthDefaults::default())?
            }
        };
        Ok(OpenSharing::new(SharingClient::new(http, endpoint))
            .with_store_options(self.store_keys.apply(self.store_options.clone())))
    }
}

impl fmt::Debug for SharingBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharingBuilder")
            .field("profile_path", &self.profile_path)
            .field("profile", &self.profile)
            .field("endpoint", &self.endpoint)
            .field("store_keys", &self.store_keys)
            .field("auth", &self.auth)
            .field("client", &self.client.as_ref().map(|_| "CloudClient"))
            .finish_non_exhaustive()
    }
}
