//! `trestle list-templates` and `list-apps`.

use crate::embedded::{embedded_app_names, embedded_base_names};
use crate::error::Result;
use crate::template::loader::{load_embedded_app, load_embedded_base};

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
