//! The **client environment**: the variables an app, notebook, or engine loads to reach the
//! stack, resolved per caller [`Vantage`] from every selected module's
//! [`client_env`](crate::Provides::client_env).

use super::{Plan, PlanError};
use crate::address::{AddressError, gateway_origin};
use crate::catalog::module::{ClientValue, UrlPart};
use crate::model::placement::Vantage;
use crate::render::InjectedEnv;

/// Resolve every graph module's client-env entries from `from`, in dependency order (a later
/// module overrides an earlier one for the same key).
pub(super) fn resolve_client_env(plan: &Plan, from: Vantage) -> Result<InjectedEnv, PlanError> {
    let mut env = InjectedEnv::new();
    for module in &plan.graph.nodes {
        for (key, value) in &module.provides().client_env {
            let resolved = match value {
                ClientValue::Literal { value } => Ok(value.clone()),
                ClientValue::GatewayOrigin => {
                    gateway_origin(from, &plan.gateway_facts).map(|u| origin(&u))
                }
                ClientValue::Endpoint {
                    service,
                    endpoint,
                    part,
                } => plan
                    .service_named(module.id(), service)
                    .ok_or_else(|| AddressError::UnknownService {
                        module: module.id().to_string(),
                        service: service.clone(),
                    })
                    .and_then(|s| s.address(from, endpoint))
                    .map(|url| match part {
                        UrlPart::Full => url.as_str().trim_end_matches('/').to_string(),
                        UrlPart::Origin => origin(&url),
                        UrlPart::Path => url.path().to_string(),
                        UrlPart::Authority => authority(&url),
                    }),
            }
            .map_err(|source| PlanError::ClientEnv {
                module: module.id().clone(),
                key: key.clone(),
                source,
            })?;
            env.set(key, resolved);
        }
    }
    Ok(env)
}

/// `host[:port]` of `url`.
fn authority(url: &url::Url) -> String {
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    }
}

/// `scheme://host[:port]` of `url`, without a trailing slash. Built by hand rather than via
/// [`Url::origin`](url::Url::origin), which is opaque (`null`) for non-special schemes.
fn origin(url: &url::Url) -> String {
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    }
}
