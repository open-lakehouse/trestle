//! `trestle generate` — proto-driven code generation.
//!
//! Loads the structured [`TrestleConfig`](crate::config::TrestleConfig) (a few
//! CLI flags can override it) and drives [`olai_codegen`] to emit
//! server/client/binding code from a compiled protobuf descriptor.

use std::fs;
use std::path::PathBuf;

use clap::Args;
use olai_codegen::{generate_code, parse_file_descriptor_set};
use protobuf::Message;
use protobuf::descriptor::FileDescriptorSet;

use crate::config::TrestleConfig;
use crate::error::{Error, Result};

#[derive(Args, Clone)]
pub struct GenerateArgs {
    /// Path to a `trestle.yaml` config file. A few CLI flags override values from it.
    #[clap(long, short = 'c', default_value = "trestle.yaml")]
    pub config: PathBuf,

    /// Override the descriptor path from the config.
    #[clap(long, short, env = "UC_BUILD_DESCRIPTORS")]
    pub descriptors: Option<String>,
}

pub fn run(args: GenerateArgs) -> Result<()> {
    let mut cfg = TrestleConfig::load(&args.config)?;

    // CLI override: descriptor path.
    if let Some(d) = args.descriptors {
        cfg.generate.descriptors = d;
    }

    cfg.derive_defaults();
    // `generate` is read-only; never prompt or mutate the file. A buffa-requiring
    // selection on a prost config is a hard error here.
    cfg.validate(false)?;

    let descriptors = cfg.generate.descriptors.clone();
    let descriptor_path =
        fs::canonicalize(PathBuf::from(&descriptors)).map_err(|e| Error::io_at(&descriptors, e))?;
    let descriptor_bytes =
        fs::read(&descriptor_path).map_err(|e| Error::io_at(&descriptor_path, e))?;
    let file_descriptor_set = FileDescriptorSet::parse_from_bytes(&descriptor_bytes)
        .map_err(|e| Error::other(format!("failed to parse descriptor: {e}")))?;

    let metadata = parse_file_descriptor_set(&file_descriptor_set)?;
    let config = cfg.to_codegen_config()?;
    generate_code(&metadata, &config)?;

    Ok(())
}
