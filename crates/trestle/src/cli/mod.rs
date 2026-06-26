//! Command-line entry point for the `trestle` binary.

pub mod config;
pub mod enrich_openapi;
pub mod generate;
pub mod list;
pub mod new;

use clap::{Parser, Subcommand};

use crate::error::Result;

/// `trestle` — unified CLI for proto codegen and project scaffolding.
#[derive(Parser)]
#[command(
    name = "trestle",
    version,
    about = "Proto-driven codegen and project scaffolding for open lakehouse architectures"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Scaffold a new project from a base template + zero or more apps.
    New(Box<new::NewArgs>),
    /// Author or update the structured project config (`trestle.yaml` + `buf.gen.yaml`).
    Config(Box<config::ConfigArgs>),
    /// Generate Rust/Python/Node.js code from a proto descriptor.
    Generate(Box<generate::GenerateArgs>),
    /// Enrich an OpenAPI YAML spec with validation rules from buf JSON Schema files.
    EnrichOpenapi(enrich_openapi::EnrichOpenApiArgs),
    /// List embedded bases + apps.
    ListTemplates,
    /// Alias for `list-templates` filtered to apps only.
    ListApps,
    /// List embedded shared components (and optionally a template's local components).
    ///
    /// Pass `--by-category` to group by category id.
    ListComponents(list::ListComponentsArgs),
    /// List all categories declared on the base or a given app's manifest.
    ListCategories(list::ListCategoriesArgs),
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::New(args) => new::run(*args),
        Commands::Config(args) => config::run(*args),
        Commands::Generate(args) => generate::run(*args),
        Commands::EnrichOpenapi(args) => enrich_openapi::run(args),
        Commands::ListTemplates => list::run_templates(),
        Commands::ListApps => list::run_apps(),
        Commands::ListComponents(args) => list::run_components(args),
        Commands::ListCategories(args) => list::run_categories(args),
    }
}
