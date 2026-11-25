//! The environment's **egress routes**: upstreams outside the environment that its services
//! reach through the egress gateway, which adds each route's credential to every request it
//! forwards (see [`InjectedCredential`]).
//!
//! A route is declared in the selection (`[selection.egress]` in `env.toml`) for an upstream no
//! module knows about, such as one route per agent to a work-graph function:
//!
//! ```toml
//! [[selection.egress.routes]]
//! name = "workgraph-planner"
//! function = "workgraph"              # the bound branch's function; or `upstream = "https://…"`
//! path = "/mcp"
//! consumers = ["omnigent"]
//! credential = { kind = "bearer", secret = "WORKGRAPH_PLANNER_API_KEY" }
//! mcp = { title = "Work graph (planner)", tools = ["read", "submit"] }
//! ```
//!
//! `source` chooses where the secret server gets the values ([`SecretSource`]): its launch
//! environment (the default), or a host endpoint the launcher runs, so a rotation reaches
//! the gateway live and the server's only launch value is a per-instance path token:
//!
//! ```toml
//! [selection.egress]
//! source = "host"
//! host_port = 7480        # the default
//! ```

use serde::{Deserialize, Serialize};

use crate::catalog::module::ModuleId;
use crate::model::endpoint::InjectedCredential;

/// Whether the environment runs the egress gateway, and the routes through it (see
/// [`Selection::egress`](crate::Selection::egress)).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EgressSelection {
    /// Run the egress gateway. Implied by any route, so only an explicit choice is written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Where the secret server gets its values. The default, [`SecretSource::Env`], isn't
    /// written.
    #[serde(default, skip_serializing_if = "SecretSource::is_env")]
    pub source: SecretSource,
    /// The host port the endpoint of [`SecretSource::Host`] listens on, on the host's loopback
    /// (or the Docker bridge on Linux). Default [`DEFAULT_HOST_SOURCE_PORT`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_port: Option<u16>,
    /// The routes, in order: each gets its own listener, on consecutive ports.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<EgressRoute>,
}

/// The port of the host endpoint the secret server reaches with [`SecretSource::Host`],
/// unless [`EgressSelection::host_port`] says otherwise.
pub const DEFAULT_HOST_SOURCE_PORT: u16 = 7480;

/// Where the secret server gets the values it serves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretSource {
    /// Its launch environment, read once at start: a rotation recreates the server. Every
    /// launcher supports it.
    #[default]
    Env,
    /// A host endpoint the launcher runs and registers the instance with, followed live: a
    /// rotation reaches the gateway without recreating anything, and the server's only launch
    /// value is the instance's path token. A launcher without an endpoint (`trestle up`)
    /// refuses to start the instance.
    Host,
}

impl SecretSource {
    /// Whether it is the default, [`SecretSource::Env`].
    pub fn is_env(&self) -> bool {
        matches!(self, SecretSource::Env)
    }
}

impl EgressSelection {
    /// Whether the selection says nothing about egress (so serializing it can be skipped).
    pub fn is_empty(&self) -> bool {
        self.enabled.is_none()
            && self.routes.is_empty()
            && self.source.is_env()
            && self.host_port.is_none()
    }

    /// The host endpoint's port, when the source is [`SecretSource::Host`].
    pub fn host_source_port(&self) -> Option<u16> {
        match self.source {
            SecretSource::Env => None,
            SecretSource::Host => Some(self.host_port.unwrap_or(DEFAULT_HOST_SOURCE_PORT)),
        }
    }

    /// Whether the environment runs the egress gateway: as chosen, else when it has a route.
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(!self.routes.is_empty())
    }
}

/// One upstream outside the environment, reached through the egress gateway with an injected
/// credential.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EgressRoute {
    /// The route's name, unique in the environment: the name of its secret on the secret
    /// server, and (upper-cased) of the variable its consumers read its URL from
    /// (`EGRESS_<NAME>_URL`).
    pub name: String,
    /// The function on the Neon branch the environment is bound to (its slug). Its URL is read
    /// from `${<SLUG>_FUNCTION_URL}`, and binding a branch without it fails. Exactly one of
    /// `function` and [`upstream`](Self::upstream).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    /// The upstream's origin (`https://host[:port]`). Exactly one of
    /// [`function`](Self::function) and `upstream`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    /// The path prefix the route forwards, unchanged: consumers call
    /// `http://egress-gateway:<port><path>`, and nothing outside it is forwarded.
    #[serde(default = "root_path")]
    pub path: String,
    /// The modules that may call the route; each gets its URL.
    #[serde(default)]
    pub consumers: Vec<ModuleId>,
    /// The credential the gateway adds to every request.
    pub credential: InjectedCredential,
    /// The route serves an MCP server, for consumers that keep an MCP registry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<McpRoute>,
}

fn root_path() -> String {
    "/".into()
}

/// What an MCP-serving [`EgressRoute`] offers (see [`EgressRoute::mcp`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpRoute {
    /// The server's display title.
    pub title: String,
    /// The tools a consumer may call; every tool when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
}

/// The variable a consumer reads route `name`'s URL from: `EGRESS_<NAME>_URL`, upper-cased with
/// every character that isn't a letter or digit as `_` (`EGRESS_WORKGRAPH_PLANNER_URL`).
pub fn egress_url_var(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("EGRESS_{slug}_URL")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_route_reads_from_toml_with_its_defaults() {
        let s: EgressSelection = toml::from_str(
            r#"
            [[routes]]
            name = "workgraph-planner"
            function = "workgraph"
            consumers = ["omnigent"]
            credential = { kind = "bearer", secret = "WORKGRAPH_PLANNER_API_KEY" }
            "#,
        )
        .unwrap();
        assert!(s.is_enabled());
        let route = &s.routes[0];
        assert_eq!(route.path, "/");
        assert_eq!(route.credential.secret_var(), "WORKGRAPH_PLANNER_API_KEY");
        assert_eq!(
            route.credential.header(),
            Some(("Authorization", "Bearer "))
        );
    }

    #[test]
    fn an_explicit_choice_wins_over_the_routes() {
        let off = EgressSelection {
            enabled: Some(false),
            ..EgressSelection::default()
        };
        assert!(!off.is_enabled());
        assert!(!EgressSelection::default().is_enabled());
        assert!(EgressSelection::default().is_empty());
    }

    #[test]
    fn the_source_defaults_to_env_and_reads_host() {
        let default = EgressSelection::default();
        assert_eq!(default.source, SecretSource::Env);
        assert_eq!(default.host_source_port(), None);
        let host: EgressSelection = toml::from_str("source = \"host\"").unwrap();
        assert_eq!(host.host_source_port(), Some(DEFAULT_HOST_SOURCE_PORT));
        assert!(!host.is_empty());
        assert!(!host.is_enabled(), "a source alone doesn't run the gateway");
        let port: EgressSelection = toml::from_str("source = \"host\"\nhost_port = 7999").unwrap();
        assert_eq!(port.host_source_port(), Some(7999));
        assert_eq!(toml::to_string(&default).unwrap(), "");
        assert!(toml::from_str::<EgressSelection>("source = \"vault\"").is_err());
    }

    #[test]
    fn credential_kinds_read_in_snake_case() {
        let header: InjectedCredential = toml::from_str(
            r#"kind = "header"
name = "x-api-key"
secret = "KEY""#,
        )
        .unwrap();
        assert_eq!(header.header(), Some(("x-api-key", "")));
        let oauth: InjectedCredential = toml::from_str(
            r#"kind = "oauth2_client_credentials"
token_url = "https://login.example.com/token"
client_id = "agent"
client_secret = "AGENT_CLIENT_SECRET"
scopes = ["read"]"#,
        )
        .unwrap();
        assert_eq!(oauth.secret_var(), "AGENT_CLIENT_SECRET");
        assert_eq!(oauth.header(), None);
    }

    #[test]
    fn the_url_variable_is_upper_snake() {
        assert_eq!(
            egress_url_var("workgraph-planner"),
            "EGRESS_WORKGRAPH_PLANNER_URL"
        );
        assert_eq!(egress_url_var("a.b"), "EGRESS_A_B_URL");
    }
}
