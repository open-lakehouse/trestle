//! The **client environment**: the variables an app, notebook, or engine loads to reach the
//! stack, resolved per caller [`Vantage`] from every selected module's
//! [`client_env`](crate::Provides::client_env) and every hosted provider's coordinates.

use super::{Plan, PlanError};
use crate::address::{AddressError, gateway_origin};
use crate::catalog::module::{ClientValue, UrlPart};
use crate::model::placement::Vantage;
use crate::model::role::Role;
use crate::render::InjectedEnv;

/// Resolve every graph module's client-env entries from `from`, in dependency order (a later
/// module overrides an earlier one for the same key).
pub(super) fn resolve_client_env(plan: &Plan, from: Vantage) -> Result<InjectedEnv, PlanError> {
    let mut env = InjectedEnv::new();
    for module in &plan.graph.nodes {
        let knobs = &plan.bindings[module.id()].knobs;
        for (key, value) in module.client_env_in(knobs, &plan.stack_facts).iter() {
            let resolved = match value {
                ClientValue::Literal { value } => Ok(value.clone()),
                ClientValue::GatewayOrigin if !plan.has_gateway() => {
                    Err(AddressError::NoSuchRole(Role::GATEWAY.into()))
                }
                ClientValue::GatewayOrigin => {
                    gateway_origin(from, &plan.gateway_facts).map(|u| origin(&u))
                }
                ClientValue::Endpoint {
                    module: owner,
                    service,
                    endpoint,
                    part,
                    prefix,
                } => {
                    let owner = owner.as_ref().unwrap_or(module.id());
                    plan.service_named(owner, service)
                        .ok_or_else(|| AddressError::UnknownService {
                            module: owner.to_string(),
                            service: service.clone(),
                        })
                        .and_then(|s| s.address(from, endpoint))
                        .map(|url| {
                            let part = match part {
                                UrlPart::Full => url.as_str().trim_end_matches('/').to_string(),
                                UrlPart::Origin => origin(&url),
                                UrlPart::Path => url.path().to_string(),
                                UrlPart::Authority => authority(&url),
                            };
                            format!("{prefix}{part}")
                        })
                }
            }
            .map_err(|source| PlanError::ClientEnv {
                module: module.id().clone(),
                key: key.clone(),
                source,
            })?;
            env.set(key, resolved);
        }
        // A hosted provider's coordinates depend on its configured connection, not the module.
        if let Some(hosted) = plan.hosted.get(module.id()) {
            for (key, value) in hosted.connection.client_env() {
                env.set(key, value);
            }
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
