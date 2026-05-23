//! `trestle enrich-openapi` — merge proto-derived validation rules into an
//! OpenAPI YAML spec.

use std::path::PathBuf;

use clap::Args;

use crate::error::{Error, Result};

#[derive(Args, Clone)]
pub struct EnrichOpenApiArgs {
    /// Path to a YAML config file; CLI flags override values from the file.
    #[arg(long, short = 'c')]
    pub config: Option<PathBuf>,

    /// Path to openapi.yaml
    #[arg(long)]
    pub spec: Option<PathBuf>,
    /// Directory containing *.schema.strict.bundle.json files
    #[arg(long)]
    pub jsonschema_dir: Option<PathBuf>,
    /// Proto descriptor binary for path/body deduplication (Pass 2); omit to skip
    #[arg(long)]
    pub descriptors: Option<PathBuf>,
    /// Translate snake_case JSON Schema property names to camelCase in OpenAPI
    #[arg(long)]
    pub camel_case: Option<bool>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileEnrichOpenApiConfig {
    pub spec: Option<PathBuf>,
    pub jsonschema_dir: Option<PathBuf>,
    pub camel_case: Option<bool>,
}

pub fn run(mut args: EnrichOpenApiArgs) -> Result<()> {
    if let Some(config_path) = args.config.clone() {
        let file = super::generate::load_trestle_config(&config_path)?;

        if args.descriptors.is_none() {
            args.descriptors = file.descriptors.map(PathBuf::from);
        }

        let cfg = file.enrich_openapi.unwrap_or_default();

        macro_rules! fill {
            ($field:ident) => {
                if args.$field.is_none() {
                    args.$field = cfg.$field;
                }
            };
        }

        fill!(spec);
        fill!(jsonschema_dir);
        fill!(camel_case);
    }

    let spec = args
        .spec
        .unwrap_or_else(|| PathBuf::from("openapi/openapi.yaml"));
    let jsonschema_dir = args
        .jsonschema_dir
        .unwrap_or_else(|| PathBuf::from("openapi/jsonschema"));
    let camel_case = args.camel_case.unwrap_or(false);

    olai_codegen::enrich_openapi(
        &spec,
        &jsonschema_dir,
        camel_case,
        args.descriptors.as_deref(),
    )
    .map_err(Error::from)?;

    Ok(())
}
