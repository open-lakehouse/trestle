//! Emit `buf.gen.yaml` from a [`GenerateConfig`].
//!
//! `buf.gen.yaml` is a *derived output* of the structured config — the single
//! source of truth is `trestle.yaml`. The model plugin depends on the proto
//! library (prost vs buffa); the Connect RPC facade adds the remote
//! `connect-rust` plugin (plus the packaging plugin that stitches the per-service
//! `mod.rs` tree).
//!
//! Two write modes exist. [`emit_buf_gen`] renders the **whole** file from
//! scratch — used by `trestle new` (fresh project) and `trestle config` (explicit
//! author/update). [`merge_buf_gen`] reconciles trestle's *managed* plugins into
//! an **existing** file while preserving any plugins an adopter added — used by
//! `trestle generate`, which regenerates on every run and must not clobber
//! adopter customization.

use super::{GenerateConfig, MODELS_GEN_SUBDIR, ProtoLib};
use crate::error::{Error, Result};

/// Version pin shared by the buffa model plugin and the connect-rust plugin so
/// they bump in lockstep.
const BUFFA_PLUGIN_VERSION: &str = "v0.7.0";

/// Render a `buf.gen.yaml` for the given config.
pub fn emit_buf_gen(cfg: &GenerateConfig) -> String {
    let mut lines: Vec<String> = vec![
        "version: v2".into(),
        "inputs:".into(),
        "  - directory: proto".into(),
        "plugins:".into(),
    ];

    // The buf model plugin co-locates the compiled model files with trestle's
    // generated `mod.rs`/`labels.rs` in the models `_gen` subdir, so the includes
    // are plain siblings.
    let models_out = format!("{}/{MODELS_GEN_SUBDIR}", cfg.models.dir);

    match cfg.proto_lib {
        ProtoLib::Buffa => {
            lines.push(format!(
                "  - remote: buf.build/anthropics/buffa:{BUFFA_PLUGIN_VERSION}"
            ));
            lines.push(format!("    out: {models_out}"));
            lines.push("    opt:".into());
            lines.push("      - json=true".into());
            lines.push("      - file_per_package=true".into());
        }
        ProtoLib::Prost => {
            lines.push("  - remote: buf.build/community/neoeinstein-prost:v0.4.0".into());
            lines.push(format!("    out: {models_out}"));
            lines.push("    opt:".into());
            lines.push("      - bytes=.".into());
            lines.push("      - file_descriptor_set".into());
            lines.push("      - compile_well_known_types".into());
            // Flat `<pkg>.rs` filenames so the co-located `mod.rs` can
            // `include!("./<pkg>.rs")` (prost v0.4 otherwise nests by package path).
            lines.push("      - flat_output_dir=true".into());
            lines.push("  - remote: buf.build/community/neoeinstein-prost-serde:v0.3.1".into());
            lines.push(format!("    out: {models_out}"));
            lines.push("    opt:".into());
            lines.push("      - flat_output_dir=true".into());
        }
    }

    if cfg.servers.connect {
        // ConnectRPC server facade via the remote `connect-rust` plugin (no local
        // toolchain install needed). It references the buffa models via the
        // absolute `buffa_module` path. The facade lands in the server crate's
        // `connect/` subdir (mounted `mod connect;`).
        let crate_name = cfg
            .models
            .crate_name
            .clone()
            .unwrap_or_else(|| "common".to_string());
        let connect_out = connect_out_dir(cfg);
        lines.push(format!(
            "  - remote: buf.build/anthropics/connect-rust:{BUFFA_PLUGIN_VERSION}"
        ));
        lines.push(format!("    out: {connect_out}"));
        lines.push("    opt:".into());
        lines.push(format!("      - buffa_module=::{crate_name}::models"));
        // TODO: confirm whether connect-rust:v0.7.0 subsumes the packaging plugin
        // (the per-service mod.rs stitcher). Until verified, keep it as a separate
        // local entry, matching prior generation.
        lines.push("  - local: protoc-gen-buffa-packaging".into());
        lines.push(format!("    out: {connect_out}"));
        lines.push("    strategy: all".into());
        lines.push("    opt:".into());
        lines.push("      - filter=services".into());
    }

    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// The `connect/` output dir for the Connect facade: `<server src>/connect`,
/// falling back to the conventional `crates/server/src/connect` when no server
/// output is configured.
fn connect_out_dir(cfg: &GenerateConfig) -> String {
    let server_src = cfg.server.output.as_deref().unwrap_or("crates/server/src");
    format!("{server_src}/connect")
}

/// The plugin refs `emit_buf_gen` can produce, regardless of `cfg`. An entry in
/// an existing `buf.gen.yaml` is "trestle-managed" iff its ref is in this set;
/// everything else is adopter-owned and preserved verbatim by [`merge_buf_gen`].
///
/// Kept exhaustive by construction: the buffa/prost/serde/connect refs mirror the
/// literals in [`emit_buf_gen`]. If a new managed plugin is added there, add its
/// ref here too (the roundtrip test guards this).
const MANAGED_PLUGIN_REFS: &[&str] = &[
    "buf.build/anthropics/buffa",
    "buf.build/community/neoeinstein-prost",
    "buf.build/community/neoeinstein-prost-serde",
    "buf.build/anthropics/connect-rust",
    "protoc-gen-buffa-packaging",
];

/// Extract the plugin ref (`remote:`/`local:` value) from a plugin entry,
/// stripping any `:vX.Y.Z` version suffix on remote refs so version bumps don't
/// change managed-vs-adopter identity.
fn plugin_ref(entry: &serde_yaml::Value) -> Option<String> {
    let map = entry.as_mapping()?;
    for key in ["remote", "local"] {
        if let Some(v) = map
            .get(serde_yaml::Value::from(key))
            .and_then(|v| v.as_str())
        {
            // Remote refs carry a `:vX` version; strip it for identity. Local
            // plugin names have no version suffix.
            let base = v.rsplit_once(':').map(|(name, _)| name).unwrap_or(v);
            return Some(base.to_string());
        }
    }
    None
}

fn is_managed(entry: &serde_yaml::Value) -> bool {
    plugin_ref(entry).is_some_and(|r| MANAGED_PLUGIN_REFS.contains(&r.as_str()))
}

/// Reconcile trestle's managed plugins into an existing `buf.gen.yaml`, preserving
/// adopter-added plugins and any top-level keys trestle doesn't own.
///
/// - Managed plugin entries (identified by [`plugin_ref`] against
///   [`MANAGED_PLUGIN_REFS`]) are **replaced** by the current set derived from
///   `cfg`, so a config change (e.g. prost→buffa) is reflected.
/// - Non-managed plugins keep their relative order and are appended after the
///   managed block.
/// - The existing file's `version`, `inputs`, and any extra top-level keys are
///   left untouched.
///
/// Returns the serialized merged document. Idempotent: merging an
/// already-merged file yields the same managed set.
pub fn merge_buf_gen(cfg: &GenerateConfig, existing: &str) -> Result<String> {
    let mut doc: serde_yaml::Value = serde_yaml::from_str(existing).map_err(Error::PlainYaml)?;

    let map = doc
        .as_mapping_mut()
        .ok_or_else(|| Error::other("buf.gen.yaml is not a YAML mapping"))?;

    // Split the existing plugins into adopter-owned (preserved) and managed
    // (dropped — re-added from the canonical set below, deduped by ref so an
    // adopter copy of a managed plugin doesn't double up).
    let existing_plugins: Vec<serde_yaml::Value> = map
        .get(serde_yaml::Value::from("plugins"))
        .and_then(|p| p.as_sequence())
        .cloned()
        .unwrap_or_default();
    let adopter_plugins: Vec<serde_yaml::Value> = existing_plugins
        .into_iter()
        .filter(|e| !is_managed(e))
        .collect();

    // Fast path: no adopter customization, so there's nothing to preserve.
    // Return the canonical hand-formatted output so `generate` doesn't reformat
    // the file (and matches what `new` / `config` write).
    if adopter_plugins.is_empty() {
        return Ok(emit_buf_gen(cfg));
    }

    // The managed plugins we want present, in canonical order.
    let managed_doc: serde_yaml::Value =
        serde_yaml::from_str(&emit_buf_gen(cfg)).map_err(Error::PlainYaml)?;
    let managed_plugins: Vec<serde_yaml::Value> = managed_doc
        .get("plugins")
        .and_then(|p| p.as_sequence())
        .cloned()
        .unwrap_or_default();

    let mut merged = managed_plugins;
    merged.extend(adopter_plugins);
    map.insert(
        serde_yaml::Value::from("plugins"),
        serde_yaml::Value::Sequence(merged),
    );

    serde_yaml::to_string(&doc).map_err(Error::PlainYaml)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Clients, Models, Server, Servers};

    fn mk(proto_lib: ProtoLib, connect: bool) -> GenerateConfig {
        GenerateConfig {
            proto_lib,
            descriptors: "api.bin".into(),
            servers: Servers {
                rest: true,
                connect,
            },
            clients: Clients::default(),
            bindings: None,
            models: Models {
                dir: "crates/common/src/models".into(),
                crate_name: Some("golden_path_app_common".into()),
                path_template: None,
                path_crate_template: None,
            },
            server: Server::default(),
        }
    }

    #[test]
    fn prost_emits_neoeinstein() {
        let s = emit_buf_gen(&mk(ProtoLib::Prost, false));
        assert!(s.contains("neoeinstein-prost:v0.4.0"));
        assert!(s.contains("neoeinstein-prost-serde:v0.3.1"));
        assert!(!s.contains("connect-rust"));
        // Models are co-located in the `_gen` subdir with flat filenames.
        assert!(s.contains("out: crates/common/src/models/_gen"));
        assert!(s.contains("flat_output_dir=true"));
    }

    #[test]
    fn connect_out_is_under_server_src() {
        let s = emit_buf_gen(&mk(ProtoLib::Buffa, true));
        assert!(s.contains("out: crates/server/src/connect"));
        assert!(!s.contains("connect_gen"));
    }

    #[test]
    fn buffa_emits_buffa_plugin() {
        let s = emit_buf_gen(&mk(ProtoLib::Buffa, false));
        assert!(s.contains("buf.build/anthropics/buffa:v0.7.0"));
        assert!(!s.contains("connect-rust"));
    }

    #[test]
    fn connect_emits_remote_plugin() {
        let s = emit_buf_gen(&mk(ProtoLib::Buffa, true));
        assert!(s.contains("remote: buf.build/anthropics/connect-rust:v0.7.0"));
        assert!(s.contains("buffa_module=::golden_path_app_common::models"));
        assert!(s.contains("protoc-gen-buffa-packaging"));
        assert!(!s.contains("local: protoc-gen-connect-rust"));
    }

    fn plugin_refs(yaml: &str) -> Vec<String> {
        let doc: serde_yaml::Value = serde_yaml::from_str(yaml).unwrap();
        doc.get("plugins")
            .and_then(|p| p.as_sequence())
            .unwrap()
            .iter()
            .filter_map(plugin_ref)
            .collect()
    }

    #[test]
    fn merge_preserves_adopter_plugin() {
        let existing = "\
version: v2
inputs:
  - directory: proto
plugins:
  - remote: buf.build/community/neoeinstein-prost:v0.4.0
    out: crates/common/src/models/_gen
  - remote: buf.build/grpc/python:v1.0.0
    out: gen/python
";
        let merged = merge_buf_gen(&mk(ProtoLib::Prost, false), existing).unwrap();
        let refs = plugin_refs(&merged);
        // Adopter's python plugin survives...
        assert!(refs.contains(&"buf.build/grpc/python".to_string()));
        // ...and trestle's managed prost/serde are present.
        assert!(refs.contains(&"buf.build/community/neoeinstein-prost".to_string()));
        assert!(refs.contains(&"buf.build/community/neoeinstein-prost-serde".to_string()));
    }

    #[test]
    fn merge_reflects_prost_to_buffa_switch() {
        let existing = emit_buf_gen(&mk(ProtoLib::Prost, false));
        let merged = merge_buf_gen(&mk(ProtoLib::Buffa, false), &existing).unwrap();
        let refs = plugin_refs(&merged);
        assert!(refs.contains(&"buf.build/anthropics/buffa".to_string()));
        // The old prost/serde managed entries are gone after the switch.
        assert!(!refs.contains(&"buf.build/community/neoeinstein-prost".to_string()));
        assert!(!refs.contains(&"buf.build/community/neoeinstein-prost-serde".to_string()));
    }

    #[test]
    fn merge_without_adopter_plugins_keeps_canonical_formatting() {
        // No adopter plugins → identical to emit_buf_gen, so `generate` doesn't
        // reformat a file that `new`/`config` wrote.
        let cfg = mk(ProtoLib::Buffa, true);
        let canonical = emit_buf_gen(&cfg);
        assert_eq!(merge_buf_gen(&cfg, &canonical).unwrap(), canonical);
    }

    #[test]
    fn merge_is_idempotent() {
        let cfg = mk(ProtoLib::Buffa, true);
        let once = merge_buf_gen(&cfg, &emit_buf_gen(&cfg)).unwrap();
        let twice = merge_buf_gen(&cfg, &once).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn merge_dedupes_adopter_copy_of_managed_plugin() {
        // An adopter who pasted the exact managed prost plugin shouldn't end up
        // with it twice after a merge.
        let existing = "\
version: v2
inputs:
  - directory: proto
plugins:
  - remote: buf.build/community/neoeinstein-prost:v0.4.0
    out: somewhere/else
";
        let merged = merge_buf_gen(&mk(ProtoLib::Prost, false), existing).unwrap();
        let count = plugin_refs(&merged)
            .iter()
            .filter(|r| *r == "buf.build/community/neoeinstein-prost")
            .count();
        assert_eq!(count, 1);
    }
}
