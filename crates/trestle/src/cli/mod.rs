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
    /// Initialize `trestle.yaml` (+ `buf.gen.yaml`) via a guided interview.
    ///
    /// A discoverable alias for `config` aimed at bootstrapping a fresh project;
    /// `config` remains for scripted / non-interactive updates. Same flags.
    Init(Box<config::ConfigArgs>),
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
        Commands::Init(args) | Commands::Config(args) => config::run(*args),
        Commands::Generate(args) => generate::run(*args),
        Commands::EnrichOpenapi(args) => enrich_openapi::run(args),
        Commands::ListTemplates => list::run_templates(),
        Commands::ListApps => list::run_apps(),
        Commands::ListComponents(args) => list::run_components(args),
        Commands::ListCategories(args) => list::run_categories(args),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// clap's own lint pass — catches conflicting flags, duplicate names, bad
    /// value-parser wiring, etc. across the whole command tree at test time.
    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).expect("args should parse")
    }

    #[test]
    fn generate_defaults() {
        let cli = parse(&["trestle", "generate"]);
        let Commands::Generate(args) = cli.command else {
            panic!("expected generate");
        };
        assert_eq!(args.config, std::path::PathBuf::from("trestle.yaml"));
        assert!(args.descriptors.is_none());
    }

    #[test]
    fn generate_descriptors_is_a_path_flag() {
        // The escape hatch: `--descriptors <path>` (no longer env-backed).
        let cli = parse(&["trestle", "generate", "--descriptors", "api.bin"]);
        let Commands::Generate(args) = cli.command else {
            panic!("expected generate");
        };
        assert_eq!(
            args.descriptors.as_deref(),
            Some(std::path::Path::new("api.bin"))
        );
    }

    #[test]
    fn init_and_config_are_distinct_variants() {
        assert!(matches!(
            parse(&["trestle", "init"]).command,
            Commands::Init(_)
        ));
        assert!(matches!(
            parse(&["trestle", "config"]).command,
            Commands::Config(_)
        ));
    }

    #[test]
    fn new_parses_repeatable_apps_and_selections() {
        let cli = parse(&[
            "trestle",
            "new",
            "my-app",
            "--app",
            "databricks-app-rust",
            "--select",
            "storage=seaweedfs,minio",
        ]);
        let Commands::New(args) = cli.command else {
            panic!("expected new");
        };
        assert_eq!(args.name, "my-app");
        assert_eq!(args.apps, vec!["databricks-app-rust".to_string()]);
        assert_eq!(
            args.selections,
            vec![(
                "storage".to_string(),
                vec!["seaweedfs".to_string(), "minio".to_string()]
            )]
        );
    }

    #[test]
    fn new_runtime_rejects_unknown_value() {
        assert!(Cli::try_parse_from(["trestle", "new", "x", "--runtime", "capnp"]).is_err());
        assert!(Cli::try_parse_from(["trestle", "new", "x", "--runtime", "buffa"]).is_ok());
    }

    #[test]
    fn new_requires_a_name() {
        assert!(Cli::try_parse_from(["trestle", "new"]).is_err());
    }

    #[test]
    fn unknown_subcommand_errors() {
        assert!(Cli::try_parse_from(["trestle", "frobnicate"]).is_err());
    }
}
