//! Command-line entry point for the `trestle` binary.

pub mod config;
pub mod enrich_openapi;
pub mod env;
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
    /// Create and render composed Docker-Compose lakehouse environments.
    #[command(subcommand)]
    Env(env::EnvCommand),
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
        Commands::Env(cmd) => env::run(cmd),
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

    fn env_new(args: &[&str]) -> env::EnvNewArgs {
        let mut full = vec!["trestle", "env", "new"];
        full.extend_from_slice(args);
        let Commands::Env(env::EnvCommand::New(a)) = parse(&full).command else {
            panic!("expected env new");
        };
        *a
    }

    #[test]
    fn env_new_parses_select_and_set() {
        let args = env_new(&[
            "lakehouse",
            "--select",
            "envoy,postgres",
            "--set",
            "envoy.auth=true",
        ]);
        assert_eq!(args.name, "lakehouse");
        assert_eq!(
            args.select,
            vec!["envoy".to_string(), "postgres".to_string()]
        );
        assert_eq!(
            args.knobs,
            vec![(
                ("envoy".to_string(), "auth".to_string()),
                "true".to_string()
            )]
        );
    }

    #[test]
    fn env_new_set_value_may_contain_equals() {
        // The value is split on the *first* `=`, so a value that itself contains
        // `=` (e.g. a query string or base64 padding) survives intact.
        let args = env_new(&["lh", "--set", "svc.opts=a=1&b=2"]);
        assert_eq!(
            args.knobs,
            vec![(
                ("svc".to_string(), "opts".to_string()),
                "a=1&b=2".to_string()
            )]
        );
    }

    #[test]
    fn env_new_rejects_malformed_set() {
        // No `.` in the module.key half.
        assert!(
            Cli::try_parse_from(["trestle", "env", "new", "lh", "--set", "auth=true"]).is_err()
        );
        // No `=` at all.
        assert!(
            Cli::try_parse_from(["trestle", "env", "new", "lh", "--set", "envoy.auth"]).is_err()
        );
    }

    #[test]
    fn env_new_parses_prefer() {
        let args = env_new(&["lh", "--prefer", "object_store=azurite,seaweedfs"]);
        assert_eq!(
            args.prefer,
            vec![(
                "object_store".to_string(),
                vec!["azurite".to_string(), "seaweedfs".to_string()]
            )]
        );
    }

    #[test]
    fn env_new_requires_a_name() {
        assert!(Cli::try_parse_from(["trestle", "env", "new"]).is_err());
    }

    #[test]
    fn env_render_defaults_dir_to_dot() {
        let Commands::Env(env::EnvCommand::Render(args)) =
            parse(&["trestle", "env", "render"]).command
        else {
            panic!("expected env render");
        };
        assert_eq!(args.dir, std::path::PathBuf::from("."));
    }
}
