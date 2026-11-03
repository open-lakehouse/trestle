//! [`NeonBranchSource`]: a Neon branch's overlay on an instance's environment.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use olai_stack_state::SecretRef;
use serde::{Deserialize, Serialize};

use super::{NeonControl, NeonError};
use crate::secrets::{SecretProviders, SecretValue};
use crate::source::{Overlay, Source, SourceCtx, SourceError};

/// The name instances bind the Neon source by.
pub const NEON_SOURCE: &str = "neon";

/// What the storage variables the planner reads start with (`NEON_S3_ENDPOINT`, …).
pub(super) const S3_VAR_PREFIX: &str = "NEON_S3_";

/// An instance's binding to a Neon branch: its [`SourceBinding`] config for
/// [`NEON_SOURCE`], written by [`bind`](super::bind). It holds no secret, only where the
/// storage credential's secret is kept.
///
/// [`SourceBinding`]: olai_stack_state::SourceBinding
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeonBinding {
    /// The Neon project.
    pub project_id: String,
    /// The branch's id.
    pub branch: String,
    /// The branch's name, for showing.
    #[serde(default)]
    pub branch_name: String,
    /// The functions the plan runs on the branch: slug → the variable its URL goes in.
    #[serde(default)]
    pub functions: BTreeMap<String, String>,
    /// The storage credential minted for the instance, when the plan uses object storage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_credential: Option<StorageCredentialRef>,
}

/// A storage credential as a binding refers to it: its id (the S3 access key id) and where its
/// secret lives.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageCredentialRef {
    /// The token id.
    pub token_id: String,
    /// The secret access key's reference.
    pub secret: SecretRef,
}

/// The overlay of the Neon branch an instance is bound to (its source config is a
/// [`NeonBinding`]):
///
/// - Postgres: `PGHOST` (the direct host), `PGUSER`, `PGDATABASE` and `PGPASSWORD`, which the
///   plan's `hosted-postgres` reads, and `DATABASE_URL` (through the pooler) and
///   `DATABASE_URL_UNPOOLED` for whatever reads those;
/// - object storage, when the binding has a credential: `NEON_S3_ENDPOINT`,
///   `NEON_S3_REGION`, `NEON_S3_ACCESS_KEY_ID` and `NEON_S3_SECRET_ACCESS_KEY`;
/// - every bound function's URL, in its variable (`UNITYCATALOG_FUNCTION_URL`). A function
///   gone from the branch since binding is an error.
///
/// Neon's credentials don't expire, so the overlay never needs a refresh.
pub struct NeonBranchSource {
    neon: Arc<dyn NeonControl>,
    secrets: SecretProviders,
}

impl NeonBranchSource {
    /// The source, asking `neon`, with the storage credential's secret resolved by `secrets`.
    pub fn new(neon: Arc<dyn NeonControl>, secrets: SecretProviders) -> Self {
        NeonBranchSource { neon, secrets }
    }

    async fn overlay_for(&self, binding: &NeonBinding) -> Result<Overlay, NeonError> {
        let (project, branch) = (&binding.project_id, &binding.branch);
        let mut overlay = Overlay::default();

        let direct = self.neon.connection(project, branch, None, false).await?;
        let pooled_url = pooled(direct.url.expose(), &direct.host);
        overlay.vars.extend([
            ("PGHOST".to_owned(), direct.host.clone()),
            ("PGUSER".to_owned(), direct.role.clone()),
            ("PGDATABASE".to_owned(), direct.database.clone()),
        ]);
        overlay.credentials.extend([
            ("PGPASSWORD".to_owned(), direct.password.clone()),
            ("DATABASE_URL".to_owned(), SecretValue::new(pooled_url)),
            ("DATABASE_URL_UNPOOLED".to_owned(), direct.url.clone()),
        ]);

        if let Some(credential) = &binding.storage_credential {
            let storage = self.neon.storage(project, branch).await?;
            if !storage.enabled {
                return Err(NeonError::NoStorage(binding.branch_name.clone()));
            }
            overlay.endpoints.insert(
                format!("{S3_VAR_PREFIX}ENDPOINT"),
                storage.s3_endpoint.trim_end_matches('/').to_owned(),
            );
            overlay
                .vars
                .insert(format!("{S3_VAR_PREFIX}REGION"), storage.region);
            let secret =
                self.secrets
                    .resolve(&credential.secret)
                    .map_err(|e| NeonError::Failed {
                        command: "credentials (stored secret)".into(),
                        stderr: e.to_string(),
                    })?;
            overlay.credentials.extend([
                (
                    format!("{S3_VAR_PREFIX}ACCESS_KEY_ID"),
                    SecretValue::new(credential.token_id.clone()),
                ),
                (format!("{S3_VAR_PREFIX}SECRET_ACCESS_KEY"), secret),
            ]);
        }

        if !binding.functions.is_empty() {
            let deployed = self.neon.functions(project, branch).await?;
            let mut missing = Vec::new();
            for (slug, var) in &binding.functions {
                match deployed.iter().find(|f| &f.slug == slug) {
                    Some(f) => {
                        overlay.endpoints.insert(
                            var.clone(),
                            f.invocation_url.trim_end_matches('/').to_owned(),
                        );
                    }
                    None => missing.push(slug.clone()),
                }
            }
            if !missing.is_empty() {
                return Err(NeonError::MissingFunctions {
                    branch: binding.branch_name.clone(),
                    slugs: missing,
                });
            }
        }
        Ok(overlay)
    }
}

/// `url` through the connection pooler: Neon's pooled host is the direct one with `-pooler`
/// after the endpoint id.
fn pooled(url: &str, host: &str) -> String {
    match host.split_once('.') {
        Some((endpoint, rest)) if !endpoint.ends_with("-pooler") => {
            url.replacen(host, &format!("{endpoint}-pooler.{rest}"), 1)
        }
        _ => url.to_owned(),
    }
}

#[async_trait]
impl Source for NeonBranchSource {
    fn name(&self) -> &str {
        NEON_SOURCE
    }

    async fn overlay(&self, ctx: &SourceCtx) -> Result<Overlay, SourceError> {
        let fail = |reason: String| SourceError {
            source_name: NEON_SOURCE.into(),
            reason,
        };
        let binding: NeonBinding = serde_json::from_value(ctx.config.clone())
            .map_err(|e| fail(format!("the instance's binding: {e}")))?;
        if binding.project_id.is_empty() || binding.branch.is_empty() {
            return Err(fail("the instance is bound to no Neon branch".into()));
        }
        self.overlay_for(&binding)
            .await
            .map_err(|e| fail(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pooled_host_adds_pooler_to_the_endpoint() {
        let host = "ep-snowy-mud-bar7hnwg.c-8.eu-central-1.aws.neon.tech";
        let url = format!("postgresql://owner@{host}/neondb?sslmode=require");
        assert_eq!(
            pooled(&url, host),
            "postgresql://owner@ep-snowy-mud-bar7hnwg-pooler.c-8.eu-central-1.aws.neon.tech/neondb?sslmode=require"
        );
        let already = "ep-x-pooler.example.neon.tech";
        assert_eq!(pooled(already, already), already);
    }
}
