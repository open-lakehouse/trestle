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
    /// The routes, in order: each gets its own listener, on consecutive ports.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<EgressRoute>,
}

impl EgressSelection {
    /// Whether the selection says nothing about egress (so serializing it can be skipped).
    pub fn is_empty(&self) -> bool {
        self.enabled.is_none() && self.routes.is_empty()
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
