//! `trestle list-templates`, `list-apps`, and `list-components`.

use clap::Args;

use crate::embedded::{embedded_app_names, embedded_base_names, embedded_shared_component_names};
use crate::error::Result;
use crate::template::loader::{load_embedded_app, load_embedded_base};
use crate::template::{ComponentCatalog, TemplateSource, load_template};

#[derive(Args, Clone)]
pub struct ListComponentsArgs {
    /// Optional template name (base or app); if given, also lists the
    /// template's private (local) components.
    #[clap(long, short)]
    pub template: Option<String>,

    /// Group the shared component listing by category.
    #[clap(long)]
    pub by_category: bool,
}

#[derive(Args, Clone)]
pub struct ListCategoriesArgs {
    /// Optional template name (base or app). Defaults to the embedded
    /// `lakehouse` base, which is the canonical home of lakehouse-wide
    /// categories.
    #[clap(long, short, default_value = "lakehouse")]
    pub template: String,
}

pub fn run_templates() -> Result<()> {
    let bases = embedded_base_names();
    let apps = embedded_app_names();

    if bases.is_empty() && apps.is_empty() {
        println!("(no embedded templates)");
        return Ok(());
    }

    if !bases.is_empty() {
        println!("Bases (always-rendered scaffold foundations):");
        for name in &bases {
            let blurb = blurb_for_base(name);
            print_entry(name, &blurb);
        }
    }
    if !apps.is_empty() {
        if !bases.is_empty() {
            println!();
        }
        println!("Apps (opt-in templates layered on top of a base):");
        for name in &apps {
            let blurb = blurb_for_app(name);
            print_entry(name, &blurb);
        }
    }

    Ok(())
}

pub fn run_apps() -> Result<()> {
    let apps = embedded_app_names();
    if apps.is_empty() {
        println!("(no embedded apps)");
        return Ok(());
    }
    println!("Embedded apps:");
    for name in &apps {
        let blurb = blurb_for_app(name);
        print_entry(name, &blurb);
    }
    Ok(())
}

pub fn run_components(args: ListComponentsArgs) -> Result<()> {
    if args.by_category {
        print_components_by_category()?;
    } else {
        println!("Shared components (templates/_components/):");
        for name in embedded_shared_component_names() {
            println!("  {name}");
        }
    }

    if let Some(name) = args.template {
        let loaded = match TemplateSource::detect(&name) {
            TemplateSource::Embedded(n) => {
                // Try base, then app.
                load_embedded_base(&n).or_else(|_| load_embedded_app(&n))?
            }
            other => load_template(&other)?,
        };
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
        if !loaded.manifest.categories.is_empty() {
            println!();
            println!("`{name}` categories:");
            for cat in &loaded.manifest.categories {
                println!(
                    "  {} ({})",
                    cat.id,
                    if cat.multi { "multi" } else { "single" }
                );
            }
        }
        if !loaded.manifest.profiles.is_empty() {
            println!();
            println!("`{name}` profiles (legacy):");
            for (k, v) in &loaded.manifest.profiles {
                println!("  {k}: {}", v.join(", "));
            }
        }
    }
    Ok(())
}

pub fn run_categories(args: ListCategoriesArgs) -> Result<()> {
    let loaded = match TemplateSource::detect(&args.template) {
        TemplateSource::Embedded(n) => load_embedded_base(&n).or_else(|_| load_embedded_app(&n))?,
        other => load_template(&other)?,
    };
    if loaded.manifest.categories.is_empty() {
        println!("`{}` declares no categories.", args.template);
        return Ok(());
    }
    let catalog = ComponentCatalog::from_embedded()?;
    println!("Categories for `{}`:", args.template);
    for cat in &loaded.manifest.categories {
        let label = cat.display_name.clone().unwrap_or_else(|| cat.id.clone());
        let kind = if cat.multi { "multi" } else { "single" };
        let required = if cat.optional { "optional" } else { "required" };
        println!("  [{}] {} ({}, {})", cat.id, label, kind, required);
        if let Some(help) = &cat.help {
            println!("      {help}");
        }
        if !cat.options.is_empty() {
            println!("      options: {}", cat.options.join(", "));
        } else {
            let eligible = catalog.components_for_category(&cat.id);
            if eligible.is_empty() {
                println!("      providers: (none yet)");
            } else {
                let names: Vec<&str> = eligible.iter().map(|c| c.name.as_str()).collect();
                println!("      providers: {}", names.join(", "));
            }
        }
        if !cat.default.0.is_empty() {
            println!("      default: {}", cat.default.0.join(", "));
        }
    }
    Ok(())
}

fn print_components_by_category() -> Result<()> {
    let cat = ComponentCatalog::from_embedded()?;
    let mut categories = cat.categories();
    categories.sort();
    println!("Shared components, grouped by category:");
    for category in &categories {
        println!("  [{category}]");
        for component in cat.components_for_category(category) {
            let label = component.display_name.as_deref().unwrap_or(&component.name);
            let summary = component.summary.as_deref().unwrap_or("");
            if summary.is_empty() {
                println!("    {} ({label})", component.name);
            } else {
                println!("    {} - {summary}", component.name);
            }
        }
    }
    // Components without a category (rare; intentionally omitted from pickers).
    let uncategorised: Vec<&crate::template::ComponentSummary> =
        cat.all().filter(|c| c.category.is_none()).collect();
    if !uncategorised.is_empty() {
        println!("  [uncategorised]");
        for c in uncategorised {
            println!("    {}", c.name);
        }
    }
    Ok(())
}

fn blurb_for_base(name: &str) -> String {
    match load_embedded_base(name) {
        Ok(t) => format_blurb(&t.manifest),
        Err(_) => String::new(),
    }
}

fn blurb_for_app(name: &str) -> String {
    match load_embedded_app(name) {
        Ok(t) => format_blurb(&t.manifest),
        Err(_) => String::new(),
    }
}

fn format_blurb(m: &crate::template::Manifest) -> String {
    if let Some(s) = &m.summary {
        return s.clone();
    }
    if let Some(d) = &m.display_name {
        return d.clone();
    }
    m.description.lines().next().unwrap_or("").to_string()
}

fn print_entry(name: &str, blurb: &str) {
    if blurb.is_empty() {
        println!("  {name}");
    } else {
        println!("  {name}  -  {blurb}");
    }
}
