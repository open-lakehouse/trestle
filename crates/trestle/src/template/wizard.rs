//! Interactive `trestle new` wizard, built on [`cliclack`].
//!
//! The wizard's job is to translate "the user wants to scaffold something" into
//! the same structured inputs the non-interactive code path takes:
//!
//! - which apps to layer on top of the base (`apps: Vec<String>`),
//! - which components to enable per lakehouse category and per app-private
//!   category (`selections: BTreeMap<String, Vec<String>>`),
//! - variable values for prompts the manifest still declares (`overrides`).
//!
//! Anything already supplied via CLI flags or `--values` is honoured: the
//! wizard only asks about choices the caller hasn't made yet, so power users
//! can fully script `trestle new` while interactive users get a guided flow.

use std::collections::BTreeMap;

use crate::error::{Error, Result};
use crate::template::catalog::ComponentCatalog;
use crate::template::loader::LoadedTemplate;
use crate::template::manifest::Category;

/// Result of running the wizard.
pub struct WizardOutput {
    /// App names the user picked (may be empty).
    pub apps: Vec<String>,
    /// Selection map keyed by category id (`storage`, `catalog`, …) or
    /// app-private namespace (`app.<app-name>.<category>`).
    pub selections: BTreeMap<String, Vec<String>>,
    /// Variable values to feed into the existing `collect_variables` pipeline.
    pub overrides: BTreeMap<String, String>,
    /// Whether the user confirmed the preview. If false the caller should
    /// abort cleanly without scaffolding.
    pub proceed: bool,
}

/// Inputs to the wizard. Anything already provided by the caller acts as a
/// pre-fill so the user isn't asked twice.
pub struct WizardInput<'a> {
    pub project_name: &'a str,
    pub base: &'a LoadedTemplate,
    /// App names *available* to layer on top of the base (catalog ordering).
    pub available_apps: &'a [&'a LoadedTemplate],
    /// Pre-selected app names (e.g. `--app databricks-app-rust`).
    pub preselected_apps: &'a [String],
    /// Pre-supplied selections (e.g. `--select storage=local-stack-seaweedfs`).
    pub preselected_selections: &'a BTreeMap<String, Vec<String>>,
    /// Pre-supplied variable values (e.g. `--set gh_owner=acme`).
    pub preselected_overrides: &'a BTreeMap<String, String>,
    /// Component metadata index used to build the multiselect items.
    pub catalog: &'a ComponentCatalog,
}

/// Run the wizard. Returns the user's choices.
pub fn run_wizard(input: WizardInput<'_>) -> Result<WizardOutput> {
    cliclack::intro(format!("trestle new {}", input.project_name)).map_err(io_err)?;
    cliclack::log::info(
        "Baseline: Envoy gateway with Databricks-shaped URLs + forwarded-user headers",
    )
    .map_err(io_err)?;

    let mut selections: BTreeMap<String, Vec<String>> = input.preselected_selections.clone();

    // 1. Walk every base category that the user hasn't already pinned via --select.
    cliclack::log::step("Configure lakehouse environment").map_err(io_err)?;
    for cat in &input.base.manifest.categories {
        if selections.contains_key(&cat.id) {
            // CLI already chose for this category; show what it picked.
            let chosen = selections.get(&cat.id).cloned().unwrap_or_default();
            cliclack::log::info(format!(
                "  {} → {} (pre-selected)",
                category_display(cat),
                fmt_list(&chosen),
            ))
            .map_err(io_err)?;
            continue;
        }
        let chosen = ask_category(cat, input.catalog, None)?;
        selections.insert(cat.id.clone(), chosen);
    }

    // 2. Multi-select apps.
    let app_picks = if input.available_apps.is_empty() {
        cliclack::log::info("No app templates available to layer on top.").map_err(io_err)?;
        Vec::new()
    } else if !input.preselected_apps.is_empty() {
        cliclack::log::info(format!(
            "Apps (pre-selected): {}",
            input.preselected_apps.join(", "),
        ))
        .map_err(io_err)?;
        input.preselected_apps.to_vec()
    } else {
        let items: Vec<(String, String, String)> = input
            .available_apps
            .iter()
            .map(|a| {
                let label = a
                    .manifest
                    .display_name
                    .clone()
                    .unwrap_or_else(|| a.manifest.name.clone());
                let hint = a.manifest.summary.clone().unwrap_or_else(|| {
                    a.manifest
                        .description
                        .lines()
                        .next()
                        .unwrap_or("")
                        .to_string()
                });
                (a.manifest.name.clone(), label, hint)
            })
            .collect();
        cliclack::multiselect("Add apps on top? (space to toggle, enter to confirm)")
            .items(&items)
            .required(false)
            .interact()
            .map_err(io_err)?
    };

    // 3. For each picked app, apply lakehouse_requires nudges and walk its
    //    private categories.
    let picked_app_manifests: Vec<&LoadedTemplate> = input
        .available_apps
        .iter()
        .copied()
        .filter(|a| app_picks.iter().any(|n| n == &a.manifest.name))
        .collect();

    for app in &picked_app_manifests {
        cliclack::log::step(format!("Configure app: {}", app.manifest.name)).map_err(io_err)?;

        // hard requirements: silently auto-enable. Note them for clarity.
        for name in &app.manifest.lakehouse_requires.hard {
            cliclack::log::info(format!("  pulls in: {name} (required)")).map_err(io_err)?;
        }
        // soft requirements: nudge the relevant category default so the user
        // sees them pre-checked the next time around. (We can't retroactively
        // amend an already-asked category, but if the user pre-selected via
        // CLI we honour their choice.)
        for name in &app.manifest.lakehouse_requires.soft {
            if let Some(cat_id) = input
                .catalog
                .get(name)
                .and_then(|c| c.category.clone())
                .and_then(|c| {
                    input
                        .base
                        .manifest
                        .categories
                        .iter()
                        .find(|cat| cat.id == c)
                        .map(|cat| cat.id.clone())
                })
            {
                cliclack::log::info(format!(
                    "  recommended: {name} (under {cat_id} — already chosen above if defaulted)"
                ))
                .map_err(io_err)?;
            }
        }

        for cat in &app.manifest.categories {
            let key = format!("app.{}.{}", app.manifest.name, cat.id);
            if selections.contains_key(&key) {
                let chosen = selections.get(&key).cloned().unwrap_or_default();
                cliclack::log::info(format!(
                    "  {} → {} (pre-selected)",
                    category_display(cat),
                    fmt_list(&chosen),
                ))
                .map_err(io_err)?;
                continue;
            }
            let chosen = ask_category(cat, input.catalog, Some(&app.manifest.name))?;
            selections.insert(key, chosen);
        }
    }

    // 4. Variable collection happens in the existing `collect_variables`
    //    pipeline; the wizard just contributes its overrides map. Defaults from
    //    the manifest cover everything; the CLI's `--set` already pre-fills the
    //    map.
    let overrides = input.preselected_overrides.clone();

    // 5. Confirmation step. The actual wiring preview is rendered by the
    //    caller (which has the resolved component set + stack context); the
    //    wizard just gates the proceed boolean.
    let proceed = cliclack::confirm("Continue and render the project?")
        .initial_value(true)
        .interact()
        .map_err(io_err)?;

    if !proceed {
        cliclack::outro_cancel("Cancelled.").map_err(io_err)?;
    }

    Ok(WizardOutput {
        apps: app_picks,
        selections,
        overrides,
        proceed,
    })
}

/// Ask the user about one category. Auto-discovers eligible components by
/// category id when `cat.options` is empty.
fn ask_category(
    cat: &Category,
    catalog: &ComponentCatalog,
    _app_namespace: Option<&str>,
) -> Result<Vec<String>> {
    let prompt = category_display(cat);

    if !cat.options.is_empty() {
        // Explicit option list — values aren't necessarily component names
        // (e.g. `frontend: [react, none]`).
        return ask_options(&prompt, cat);
    }

    let eligible = catalog.components_for_category(&cat.id);
    if eligible.is_empty() {
        // Nothing to pick. Still print the category for clarity.
        cliclack::log::info(format!("  {prompt} → (no components available)")).map_err(io_err)?;
        return Ok(Vec::new());
    }

    let items: Vec<(String, String, String)> = eligible
        .iter()
        .map(|c| {
            let label = c.display_name.clone().unwrap_or_else(|| c.name.clone());
            let hint = c.summary.clone().unwrap_or_default();
            (c.name.clone(), label, hint)
        })
        .collect();

    let initial: Vec<String> = cat.default.0.clone();

    if cat.multi {
        let chosen: Vec<String> = cliclack::multiselect(prompt.clone())
            .items(&items)
            .initial_values(initial)
            .required(!cat.optional)
            .interact()
            .map_err(io_err)?;
        Ok(chosen)
    } else {
        let initial_one = initial
            .into_iter()
            .next()
            .unwrap_or_else(|| items.first().map(|i| i.0.clone()).unwrap_or_default());
        let mut select = cliclack::select(prompt.clone()).items(&items);
        if !initial_one.is_empty() {
            select = select.initial_value(initial_one);
        }
        let chosen = select.interact().map_err(io_err)?;
        if chosen == "none" {
            Ok(Vec::new())
        } else {
            Ok(vec![chosen])
        }
    }
}

/// Ask about a category whose `options:` is explicit (e.g. `frontend: [react, none]`).
fn ask_options(prompt: &str, cat: &Category) -> Result<Vec<String>> {
    let items: Vec<(String, String, String)> = cat
        .options
        .iter()
        .map(|o| (o.clone(), o.clone(), String::new()))
        .collect();

    let initial: Vec<String> = cat.default.0.clone();

    if cat.multi {
        let chosen: Vec<String> = cliclack::multiselect(prompt.to_string())
            .items(&items)
            .initial_values(initial)
            .required(!cat.optional)
            .interact()
            .map_err(io_err)?;
        Ok(chosen)
    } else {
        let initial_one = initial
            .into_iter()
            .next()
            .unwrap_or_else(|| items.first().map(|i| i.0.clone()).unwrap_or_default());
        let mut select = cliclack::select(prompt.to_string()).items(&items);
        if !initial_one.is_empty() {
            select = select.initial_value(initial_one);
        }
        let chosen = select.interact().map_err(io_err)?;
        if chosen == "none" {
            Ok(Vec::new())
        } else {
            Ok(vec![chosen])
        }
    }
}

fn category_display(cat: &Category) -> String {
    cat.display_name.clone().unwrap_or_else(|| cat.id.clone())
}

fn fmt_list(values: &[String]) -> String {
    if values.is_empty() {
        "(none)".to_string()
    } else {
        values.join(", ")
    }
}

fn io_err(e: std::io::Error) -> Error {
    Error::other(format!("wizard prompt failed: {e}"))
}
