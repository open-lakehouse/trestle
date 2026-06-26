//! Emit `buf.gen.yaml` from a [`GenerateConfig`].
//!
//! `buf.gen.yaml` is a *derived output* of the structured config — the single
//! source of truth is `trestle.yaml`. The model plugin depends on the proto
//! library (prost vs buffa); the Connect RPC facade adds the remote
//! `connect-rust` plugin (plus the packaging plugin that stitches the per-service
//! `mod.rs` tree).

use super::{GenerateConfig, MODELS_GEN_SUBDIR, ProtoLib};

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
}
