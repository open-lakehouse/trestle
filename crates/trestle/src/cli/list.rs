//! `trestle list-templates` and `trestle list-components`.

use clap::Args;

use crate::embedded::{embedded_shared_component_names, embedded_template_names};
use crate::error::Result;
use crate::template::{TemplateSource, load_template};

#[derive(Args, Clone)]
pub struct ListComponentsArgs {
    /// Optional template name; if given, also lists the template's private
    /// (local) components.
    #[clap(long, short)]
    pub template: Option<String>,
}

pub fn run_templates() -> Result<()> {
    let names = embedded_template_names();
    if names.is_empty() {
        println!("(no embedded templates)");
        return Ok(());
    }
    println!("Embedded templates:");
    for name in names {
        // Best-effort description load.
        let src = TemplateSource::Embedded(name.clone());
        let desc = match load_template(&src) {
            Ok(t) => t.manifest.description,
            Err(_) => String::new(),
        };
        if desc.is_empty() {
            println!("  {name}");
        } else {
            println!("  {name}  -  {desc}");
        }
    }
    Ok(())
}

pub fn run_components(args: ListComponentsArgs) -> Result<()> {
    println!("Shared components (templates/_components/):");
    for name in embedded_shared_component_names() {
        println!("  {name}");
    }

    if let Some(name) = args.template {
        let src = TemplateSource::Embedded(name.clone());
        let loaded = load_template(&src)?;
        let local: Vec<_> = loaded
            .manifest
            .components
            .iter()
            .filter(|c| matches!(c.kind, crate::template::manifest::ComponentKind::Local))
            .map(|c| c.name.clone())
            .collect();
        println!();
        println!("`{name}` local components:");
        if local.is_empty() {
            println!("  (none)");
        } else {
            for n in local {
                println!("  {n}");
            }
        }
        if !loaded.manifest.profiles.is_empty() {
            println!();
            println!("`{name}` profiles:");
            for (k, v) in &loaded.manifest.profiles {
                println!("  {k}: {}", v.join(", "));
            }
        }
    }
    Ok(())
}
