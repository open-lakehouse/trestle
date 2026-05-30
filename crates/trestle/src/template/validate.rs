//! Cross-component validation that runs after resolution and before render.
//!
//! Two classes of consistency check live here:
//!
//! - **`conflicts_with`** — a component cannot coexist with anything in its
//!   `conflicts_with:` list. Catches mutually-exclusive picks (e.g. two
//!   incompatible storage backends) before they produce a broken compose file.
//! - **Port collisions** — two components must not claim the same host port via
//!   `provides.ports[].default`. Catches accidental dup ports introduced by
//!   adding a new component.

use std::collections::BTreeMap;

use crate::error::{Error, Result};
use crate::template::resolve::ResolvedComponent;

/// Run every cross-component consistency check and surface the first failure.
pub fn validate_resolved(components: &[ResolvedComponent]) -> Result<()> {
    check_conflicts(components)?;
    check_port_collisions(components)?;
    Ok(())
}

fn check_conflicts(components: &[ResolvedComponent]) -> Result<()> {
    let names: Vec<&str> = components
        .iter()
        .map(|c| c.loaded.manifest.name.as_str())
        .collect();
    for c in components {
        for forbidden in &c.loaded.manifest.conflicts_with {
            if names.iter().any(|n| *n == forbidden) {
                return Err(Error::Manifest(format!(
                    "component `{}` conflicts with `{forbidden}`; they cannot be enabled together",
                    c.loaded.manifest.name
                )));
            }
        }
    }
    Ok(())
}

fn check_port_collisions(components: &[ResolvedComponent]) -> Result<()> {
    let mut by_port: BTreeMap<u16, Vec<(String, String)>> = BTreeMap::new();
    for c in components {
        for p in &c.loaded.manifest.provides.ports {
            if p.internal_only {
                continue;
            }
            by_port
                .entry(p.default)
                .or_default()
                .push((c.loaded.manifest.name.clone(), p.name.clone()));
        }
    }
    for (port, claimants) in &by_port {
        if claimants.len() > 1 {
            let pretty: Vec<String> = claimants
                .iter()
                .map(|(comp, port_name)| format!("{comp}::{port_name}"))
                .collect();
            return Err(Error::Manifest(format!(
                "port {port} is claimed by multiple components: {}",
                pretty.join(", ")
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::loader::LoadedComponent;
    use crate::template::manifest::{ComponentManifest, PortDecl, Provides};

    fn synth(name: &str, conflicts: Vec<&str>, ports: Vec<(&str, u16)>) -> ResolvedComponent {
        let m = ComponentManifest {
            name: name.to_string(),
            conflicts_with: conflicts.into_iter().map(String::from).collect(),
            provides: Provides {
                ports: ports
                    .into_iter()
                    .map(|(n, p)| PortDecl {
                        name: n.to_string(),
                        default: p,
                        internal_only: false,
                    })
                    .collect(),
                ..Default::default()
            },
            ..Default::default()
        };
        ResolvedComponent {
            kind: crate::template::manifest::ComponentKind::Shared,
            loaded: LoadedComponent::synthetic(std::path::PathBuf::from("/tmp/synthetic"), m),
        }
    }

    #[test]
    fn conflicts_with_is_caught() {
        let cs = vec![
            synth("storage-a", vec!["storage-b"], vec![]),
            synth("storage-b", vec![], vec![]),
        ];
        let err = validate_resolved(&cs).unwrap_err();
        assert!(format!("{err}").contains("storage-a"));
        assert!(format!("{err}").contains("storage-b"));
    }

    #[test]
    fn port_collisions_are_caught() {
        let cs = vec![
            synth("a", vec![], vec![("api", 9080)]),
            synth("b", vec![], vec![("api", 9080)]),
        ];
        let err = validate_resolved(&cs).unwrap_err();
        assert!(format!("{err}").contains("port 9080"));
    }

    #[test]
    fn distinct_ports_are_fine() {
        let cs = vec![
            synth("a", vec![], vec![("api", 9080)]),
            synth("b", vec![], vec![("api", 9081)]),
        ];
        validate_resolved(&cs).expect("no collision");
    }
}
