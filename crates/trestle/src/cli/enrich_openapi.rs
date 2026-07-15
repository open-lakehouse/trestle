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

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileEnrichOpenApiConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jsonschema_dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camel_case: Option<bool>,
    /// Rename emitted OpenAPI component-schema keys: `<current key>: <desired key>`.
    /// gnostic derives the schema key from the proto message name with no override;
    /// this decouples them so the spec can present the official `*Info` naming
    /// (e.g. `Catalog: CatalogInfo`) without renaming the proto/Rust model types.
    /// Source keys absent from the spec are ignored. Config-file only.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub schema_renames: std::collections::BTreeMap<String, String>,
    /// Component-schema keys whose body gnostic clobbers via a name collision with
    /// one of its own OpenAPI meta-model types (e.g. a proto message named `Schema`
    /// is overwritten by gnostic's meta `Schema`). For each listed key, the enrich
    /// pass rebuilds the component from the authoritative JSON Schema instead of
    /// merge-enriching the clobbered one. Config-file only.
    #[serde(default, skip_serializing_if = "std::collections::HashSet::is_empty")]
    pub schema_overrides: std::collections::HashSet<String>,
}

pub fn run(mut args: EnrichOpenApiArgs) -> Result<()> {
    let mut schema_renames = std::collections::BTreeMap::new();
    let mut schema_overrides = std::collections::HashSet::new();

    if let Some(config_path) = args.config.clone() {
        let file = crate::config::TrestleConfig::load(&config_path)?;

        if args.descriptors.is_none() {
            args.descriptors = Some(PathBuf::from(file.generate.descriptors));
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
        schema_renames = cfg.schema_renames;
        schema_overrides = cfg.schema_overrides;
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
        &schema_renames,
        &schema_overrides,
    )
    .map_err(Error::from)?;

    Ok(())
}
