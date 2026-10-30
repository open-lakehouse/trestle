//! Configuring a [`NeonStorage`] from string keys.

use std::fmt;

use olai_http::CloudClient;
use olai_storage_credentials::AccessLevel;

use super::{BranchStorage, DEFAULT_API_URL, Discovery, NeonClient, NeonCredentials, NeonStorage};
use crate::auth::{AuthConfig, AuthDefaults, is_truthy};
use crate::error::{Error, Result};
use crate::options::{StoreKeys, access_key_value, parse_access, parse_base_url};
use crate::store::StoreOptions;

/// Builds a [`NeonStorage`] from configuration keys.
///
/// | Key | Meaning |
/// |---|---|
/// | `neon_api_key` | The Neon API key (also `NEON_API_KEY`); other `neon_*` auth keys work too, see [`crate::auth`] |
/// | `neon_api_url` | The Neon API; default `https://console.neon.tech/api/v2/` |
/// | `neon_project_id`, `neon_branch_id` | The branch whose storage to use |
/// | `neon_access_key_id`, `neon_secret_access_key` | A credential already held |
/// | `neon_token_id` | Reveal the secret of this existing credential |
/// | `neon_create_credentials` | `true` to create a credential when none is configured; off by default |
/// | `neon_credential_name` | A label for a created credential |
/// | `neon_operation` | `read` (the default) or `read_write`: the scopes of a created credential, and the access stores are declared with |
/// | `neon_s3_endpoint`, `neon_region`, `neon_force_path_style` | The branch's storage, skipping discovery when the endpoint and region are both set |
/// | client keys (`allow_http`, `timeout`, ...) | Apply to the stores and the control plane |
///
/// Credentials are taken in that order: keys, then reveal, then create. Without an API
/// key, keys and the branch's storage must both be given.
#[derive(Clone)]
pub struct NeonBuilder {
    api_url: Option<String>,
    project: Option<String>,
    branch: Option<String>,
    access_key_id: Option<String>,
    secret_access_key: Option<String>,
    token_id: Option<String>,
    create: Option<String>,
    credential_name: Option<String>,
    operation: Option<String>,
    s3_endpoint: Option<String>,
    region: Option<String>,
    force_path_style: Option<String>,
    store_keys: StoreKeys,
    auth: AuthConfig,
    api_key: bool,
    store_options: StoreOptions,
    client: Option<CloudClient>,
}

impl Default for NeonBuilder {
    fn default() -> Self {
        Self::with_auth(AuthConfig::new("neon"))
    }
}

impl NeonBuilder {
    /// An empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// A builder configured from the environment: `NEON_*` variables named after a key
    /// (`NEON_API_KEY`, `NEON_PROJECT_ID`, ...). Options set afterwards win.
    pub fn from_env() -> Self {
        let mut builder = Self::with_auth(AuthConfig::from_env("neon"));
        for (name, value) in std::env::vars() {
            if name.starts_with("NEON_") {
                builder = builder.with_option(&name.to_ascii_lowercase(), value);
            }
        }
        builder
    }

    fn with_auth(auth: AuthConfig) -> Self {
        Self {
            api_url: None,
            project: None,
            branch: None,
            access_key_id: None,
            secret_access_key: None,
            token_id: None,
            create: None,
            credential_name: None,
            operation: None,
            s3_endpoint: None,
            region: None,
            force_path_style: None,
            store_keys: StoreKeys::default(),
            auth,
            api_key: false,
            store_options: StoreOptions::default(),
            client: None,
        }
    }

    /// Declares the access stores are built for, and the scopes of a created
    /// credential.
    pub fn with_access(mut self, access: AccessLevel) -> Self {
        self.operation = Some(access_key_value(access).to_owned());
        self
    }

    /// Store options; the client keys set here apply on top.
    pub fn with_store_options(mut self, options: StoreOptions) -> Self {
        self.store_options = options;
        self
    }

    /// Sends every control-plane request through `client`, bypassing the API key.
    pub fn with_client(mut self, client: CloudClient) -> Self {
        self.client = Some(client);
        self
    }

    /// Sets one option; unknown keys are ignored.
    pub fn with_option(mut self, key: &str, value: impl Into<String>) -> Self {
        let value = value.into();
        match key {
            "neon_api_key" => {
                self.auth.set("neon_token", value);
                self.api_key = true;
            }
            "neon_api_url" => self.api_url = Some(value),
            "neon_project_id" => self.project = Some(value),
            "neon_branch_id" => self.branch = Some(value),
            "neon_access_key_id" => self.access_key_id = Some(value),
            "neon_secret_access_key" => self.secret_access_key = Some(value),
            "neon_token_id" => self.token_id = Some(value),
            "neon_create_credentials" => self.create = Some(value),
            "neon_credential_name" => self.credential_name = Some(value),
            "neon_operation" => self.operation = Some(value),
            "neon_s3_endpoint" => self.s3_endpoint = Some(value),
            "neon_region" => self.region = Some(value),
            "neon_force_path_style" => self.force_path_style = Some(value),
            _ => {
                if !self.store_keys.set(key, &value)
                    && self.auth.set(key, value)
                    && key.starts_with("neon_")
                {
                    self.api_key = true;
                }
            }
        }
        self
    }

    /// Sets options from a map, ignoring keys that are not Neon, auth or client keys,
    /// so a whole storage-options map can be passed.
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

    /// Builds the storage. No request is sent until a store is asked for.
    pub fn build(self) -> Result<NeonStorage> {
        let required = |value: &Option<String>, key: &str| {
            value
                .clone()
                .ok_or_else(|| Error::Config(format!("missing `{key}`")))
        };
        let project = required(&self.project, "neon_project_id")?;
        let branch = required(&self.branch, "neon_branch_id")?;
        let access =
            parse_access("neon_operation", self.operation.as_deref())?.unwrap_or(AccessLevel::Read);
        let credentials = self.credentials()?;
        let discovery = match (&self.s3_endpoint, &self.region) {
            (Some(endpoint), Some(region)) => Discovery::Given(BranchStorage {
                enabled: true,
                s3_endpoint: endpoint.clone(),
                region: region.clone(),
                force_path_style: self.force_path_style.as_deref().is_none_or(is_truthy),
            }),
            _ => Discovery::Api,
        };
        let needs_api = matches!(discovery, Discovery::Api)
            || !matches!(credentials, NeonCredentials::Keys { .. });
        let client = if needs_api || self.api_key || self.client.is_some() {
            let url = parse_base_url(
                "neon_api_url",
                self.api_url.as_deref().unwrap_or(DEFAULT_API_URL),
            )?;
            let http = match &self.client {
                Some(client) => client.clone(),
                None => self.auth.client(&url, &AuthDefaults::default())?,
            };
            Some(NeonClient::new(http, url))
        } else {
            None
        };
        Ok(
            NeonStorage::with_parts(client, project, branch, discovery, credentials, access)
                .with_store_options(self.store_keys.apply(self.store_options.clone())),
        )
    }

    /// The credential source the keys select: keys, else reveal, else (opted in)
    /// create.
    fn credentials(&self) -> Result<NeonCredentials> {
        if let (Some(access_key_id), Some(secret_access_key)) =
            (&self.access_key_id, &self.secret_access_key)
        {
            return Ok(NeonCredentials::Keys {
                access_key_id: access_key_id.clone(),
                secret_access_key: secret_access_key.clone(),
            });
        }
        if self.access_key_id.is_some() || self.secret_access_key.is_some() {
            return Err(Error::Config(
                "`neon_access_key_id` and `neon_secret_access_key` go together".to_owned(),
            ));
        }
        if let Some(token_id) = &self.token_id {
            return Ok(NeonCredentials::Reveal {
                token_id: token_id.clone(),
            });
        }
        if self.create.as_deref().is_some_and(is_truthy) {
            return Ok(NeonCredentials::Create {
                name: self.credential_name.clone(),
            });
        }
        Err(Error::Config(
            "no Neon credential configured: set `neon_access_key_id` + \
             `neon_secret_access_key`, `neon_token_id`, or `neon_create_credentials=true`"
                .to_owned(),
        ))
    }
}

impl fmt::Debug for NeonBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NeonBuilder")
            .field("api_url", &self.api_url)
            .field("project", &self.project)
            .field("branch", &self.branch)
            .field("access_key_id", &self.access_key_id)
            .field(
                "secret_access_key",
                &self.secret_access_key.as_ref().map(|_| "<redacted>"),
            )
            .field("token_id", &self.token_id)
            .field("create", &self.create)
            .field("operation", &self.operation)
            .field("auth", &self.auth)
            .field("client", &self.client.as_ref().map(|_| "CloudClient"))
            .finish_non_exhaustive()
    }
}
