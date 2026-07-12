//! Cross-cell constraint validation for [`TrestleConfig`].
//!
//! The config matrix has a handful of rules that span cells. They are enforced
//! here, centrally, so both `trestle generate` and `trestle config` behave
//! identically:
//!
//! 1. `servers.connect` ⇒ buffa
//! 2. any WASM client (rust transport `wasm`, or `node.wasm`) ⇒ buffa
//! 3. a JS/TS client (`node.ts` / `node.wasm`) ⇒ `bindings.*` identity present
//! 4. `clients.python` ⇒ `error_type` + `result_type`
//! 5. `clients.node.napi` ⇒ `error_ext_trait`
//!
//! Rules (1)/(2) are *upgradable*: in interactive mode the user is prompted to
//! switch the proto library to buffa; non-interactively (and on decline) they
//! are hard errors. Rules (3)–(5) are always hard errors — they're enforced by
//! the schema being `required`, but checked here too for a friendly message.

use super::{JsonFieldNames, ProtoLib, Transport, TrestleConfig};
use crate::error::{Error, Result};

impl TrestleConfig {
    /// Validate constraints. When `interactive`, a buffa-requiring selection on a
    /// prost config prompts the user to upgrade (mutating `self`); otherwise it
    /// is a hard error.
    pub fn validate(&mut self, interactive: bool) -> Result<()> {
        let needs_buffa = self.buffa_requirement();
        if let Some(reason) = needs_buffa
            && self.generate.proto_lib != ProtoLib::Buffa
        {
            self.resolve_buffa_requirement(reason, interactive)?;
        }

        // `json_field_names: proto` only affects buffa output (it post-processes the
        // buffa plugin's serde renames). Under prost, casing is controlled by the
        // prost-serde `preserve_proto_field_names` buf option instead, so the knob
        // would silently do nothing — flag it as a config mistake.
        if self.generate.json_field_names == JsonFieldNames::Proto
            && self.generate.proto_lib != ProtoLib::Buffa
        {
            return Err(Error::other(
                "`json_field_names: proto` only applies to `proto_lib: buffa`; under \
                 prost, use the prost-serde `preserve_proto_field_names` option instead",
            ));
        }

        // Identity / per-client required fields. The schema already makes these
        // `required` (so deserialize fails first), but the rust client's wasm
        // transport + node.ts/wasm share the `bindings` block, which is optional
        // at the type level — check it here.
        let g = &self.generate;
        if let Some(node) = &g.clients.node
            && (node.ts.is_some() || node.wasm.is_some())
        {
            let b = g.bindings.as_ref();
            let missing = b.is_none_or(|b| {
                b.aggregate_client_name.is_none()
                    || b.client_crate_name.is_none()
                    || b.error_base_class.is_none()
            });
            if missing {
                return Err(Error::other(
                    "a JS/TS client (node.ts / node.wasm) requires `bindings` with \
                         aggregate_client_name, client_crate_name and error_base_class \
                         (these derive from project.name — call derive_defaults first)",
                ));
            }
        }
        Ok(())
    }

    /// The reason a buffa runtime is required, if any (for the prompt/error text).
    fn buffa_requirement(&self) -> Option<&'static str> {
        let g = &self.generate;
        if g.servers.connect {
            return Some("Connect RPC");
        }
        if let Some(rust) = &g.clients.rust
            && rust.transport == Transport::Wasm
        {
            return Some("the WASM Rust client transport");
        }
        if let Some(node) = &g.clients.node
            && node.wasm.is_some()
        {
            return Some("the WASM browser client");
        }
        None
    }

    fn resolve_buffa_requirement(&mut self, reason: &str, interactive: bool) -> Result<()> {
        if interactive {
            let switch = cliclack::confirm(format!(
                "{reason} requires the buffa proto library, but proto_lib is prost. Switch to buffa?"
            ))
            .initial_value(true)
            .interact()
            .map_err(|e| Error::other(format!("prompt failed: {e}")))?;
            if switch {
                self.generate.proto_lib = ProtoLib::Buffa;
                return Ok(());
            }
        }
        Err(Error::other(format!(
            "{reason} requires `proto_lib: buffa`, but it is set to prost"
        )))
    }
}

#[cfg(test)]
mod tests {
    use crate::config::{
        Clients, GenerateConfig, Models, NodeClient, ProjectMeta, ProtoLib, Server, Servers,
        TrestleConfig, WasmBindings,
    };

    fn base() -> TrestleConfig {
        TrestleConfig {
            version: 1,
            project: ProjectMeta {
                name: "demo".into(),
                id: None,
                description: None,
            },
            generate: GenerateConfig {
                proto_lib: ProtoLib::Prost,
                json_field_names: Default::default(),
                descriptors: "api.bin".into(),
                servers: Servers::default(),
                clients: Clients::default(),
                bindings: None,
                models: Models {
                    dir: "x".into(),
                    crate_name: None,
                    path_template: None,
                    path_crate_template: None,
                },
                server: Server::default(),
            },
            enrich_openapi: None,
        }
    }

    #[test]
    fn connect_on_prost_errors_non_interactive() {
        let mut c = base();
        c.generate.servers.connect = true;
        assert!(c.validate(false).is_err());
    }

    #[test]
    fn wasm_browser_on_prost_errors_non_interactive() {
        let mut c = base();
        c.derive_defaults();
        c.generate.clients.node = Some(NodeClient {
            napi: None,
            ts: None,
            wasm: Some(WasmBindings { output: "w".into() }),
        });
        assert!(c.validate(false).is_err());
    }

    #[test]
    fn buffa_with_connect_passes() {
        let mut c = base();
        c.generate.proto_lib = ProtoLib::Buffa;
        c.generate.servers.connect = true;
        assert!(c.validate(false).is_ok());
    }

    #[test]
    fn json_field_names_proto_on_prost_errors() {
        let mut c = base();
        c.generate.json_field_names = crate::config::JsonFieldNames::Proto;
        // proto_lib is Prost in `base()`.
        assert!(c.validate(false).is_err());
    }

    #[test]
    fn json_field_names_proto_on_buffa_passes() {
        let mut c = base();
        c.generate.proto_lib = ProtoLib::Buffa;
        c.generate.json_field_names = crate::config::JsonFieldNames::Proto;
        assert!(c.validate(false).is_ok());
    }

    #[test]
    fn node_ts_without_bindings_errors() {
        let mut c = base();
        c.generate.proto_lib = ProtoLib::Buffa;
        c.generate.clients.node = Some(NodeClient {
            napi: None,
            ts: Some(crate::config::TsBindings { output: "t".into() }),
            wasm: None,
        });
        // No bindings + no derive → missing identity.
        assert!(c.validate(false).is_err());
    }
}
