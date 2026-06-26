//! `trestle config` — author or update the structured project config.
//!
//! Produces `trestle.yaml` (the single source of truth) and its derived
//! `buf.gen.yaml`. Flags-first: the full parameter surface lets automation drive
//! it non-interactively; when a TTY is present and `--non-interactive` is not
//! set, prompts fill any gaps. An existing `trestle.yaml` is loaded as the base
//! so the command reconfigures rather than clobbers (preserving `project.id`).

use std::path::PathBuf;

use clap::{Args, ValueEnum};

use crate::config::{
    Bindings, Clients, GenerateConfig, Models, NapiBindings, NodeClient, ProjectMeta, ProtoLib,
    PythonClient, RustClient, Server, Servers, Transport, TrestleConfig, TsBindings, WasmBindings,
    emit_buf_gen,
};
use crate::error::{Error, Result};

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
pub enum ProtoLibArg {
    Prost,
    Buffa,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
pub enum ServerArg {
    Rest,
    Connect,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
pub enum ClientArg {
    Rust,
    Python,
    Node,
}

#[derive(Args, Clone)]
pub struct ConfigArgs {
    /// Project root name (most crate / binding names derive from it).
    #[clap(long)]
    pub name: Option<String>,

    /// Optional project description.
    #[clap(long)]
    pub description: Option<String>,

    /// Protobuf library the generated code consumes.
    #[clap(long, value_enum)]
    pub proto_lib: Option<ProtoLibArg>,

    /// Servers to emit (repeatable): `rest`, `connect`.
    #[clap(long = "server", value_enum)]
    pub servers: Vec<ServerArg>,

    /// Clients to emit (repeatable): `rust`, `python`, `node`.
    #[clap(long = "client", value_enum)]
    pub clients: Vec<ClientArg>,

    /// Rust client transport: `cloud` (default) or `wasm`.
    #[clap(long, value_enum)]
    pub rust_transport: Option<TransportArg>,

    /// Emit the NAPI native bindings for the Node client.
    #[clap(long)]
    pub node_napi: bool,

    /// Emit the WASM browser bindings for the Node client.
    #[clap(long)]
    pub node_wasm: bool,

    /// Emit the NAPI TypeScript client for the Node client.
    #[clap(long)]
    pub node_ts: bool,

    /// Output path for the config file.
    #[clap(long, short = 'o', default_value = "trestle.yaml")]
    pub out: PathBuf,

    /// Disable interactive prompts; rely solely on flags + an existing config.
    #[clap(long)]
    pub non_interactive: bool,

    /// Overwrite existing files.
    #[clap(long)]
    pub force: bool,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
pub enum TransportArg {
    Cloud,
    Wasm,
}

pub fn run(args: ConfigArgs) -> Result<()> {
    let interactive = !args.non_interactive;

    // Start from an existing config when present (preserves project.id + overrides).
    let mut cfg = if args.out.exists() {
        TrestleConfig::load(&args.out)?
    } else {
        default_config()
    };

    apply_flags(&mut cfg, &args);

    if interactive {
        prompt_missing(&mut cfg)?;
    }

    if cfg.project.name.trim().is_empty() {
        return Err(Error::other(
            "project name is required (pass --name or run interactively)",
        ));
    }

    cfg.derive_defaults();
    cfg.validate(interactive)?;
    cfg.ensure_id();

    cfg.write(&args.out, args.force)?;

    let buf_gen_path = args
        .out
        .parent()
        .map(|p| p.join("buf.gen.yaml"))
        .unwrap_or_else(|| PathBuf::from("buf.gen.yaml"));
    write_buf_gen(&cfg.generate, &buf_gen_path, args.force)?;

    println!(
        "Wrote {} and {}",
        args.out.display(),
        buf_gen_path.display()
    );
    Ok(())
}

fn default_config() -> TrestleConfig {
    TrestleConfig {
        version: crate::config::CONFIG_VERSION,
        project: ProjectMeta {
            name: String::new(),
            id: None,
            description: None,
        },
        generate: GenerateConfig {
            proto_lib: ProtoLib::default(),
            descriptors: "api.bin".to_string(),
            servers: Servers::default(),
            clients: Clients::default(),
            bindings: None,
            models: Models {
                dir: "crates/common/src/models".to_string(),
                crate_name: None,
                path_template: None,
                path_crate_template: None,
            },
            server: Server::default(),
        },
        enrich_openapi: None,
    }
}

fn apply_flags(cfg: &mut TrestleConfig, args: &ConfigArgs) {
    if let Some(name) = &args.name {
        cfg.project.name = name.clone();
    }
    if let Some(desc) = &args.description {
        cfg.project.description = Some(desc.clone());
    }
    if let Some(pl) = args.proto_lib {
        cfg.generate.proto_lib = match pl {
            ProtoLibArg::Prost => ProtoLib::Prost,
            ProtoLibArg::Buffa => ProtoLib::Buffa,
        };
    }

    if !args.servers.is_empty() {
        cfg.generate.servers = Servers {
            rest: args.servers.contains(&ServerArg::Rest),
            connect: args.servers.contains(&ServerArg::Connect),
        };
    }

    if !args.clients.is_empty() {
        let g = &mut cfg.generate;
        if args.clients.contains(&ClientArg::Rust) {
            let transport = match args.rust_transport {
                Some(TransportArg::Wasm) => Transport::Wasm,
                _ => Transport::Cloud,
            };
            g.clients.rust = Some(RustClient {
                output: "crates/client/src".to_string(),
                transport,
                transport_type_path: None,
            });
        }
        if args.clients.contains(&ClientArg::Python) {
            g.clients.python = Some(PythonClient {
                output: "crates/client/python/src".to_string(),
                error_type: String::new(),
                result_type: String::new(),
                typings_package_filter: None,
            });
        }
        if args.clients.contains(&ClientArg::Node) {
            // Default to NAPI when no specific node variant flag is given, so
            // `--client node` alone still produces a usable native client.
            let any_variant = args.node_napi || args.node_ts || args.node_wasm;
            let napi = args.node_napi || !any_variant;
            g.clients.node = Some(NodeClient {
                napi: napi.then(|| NapiBindings {
                    output: "crates/client/node/src".to_string(),
                    error_ext_trait: String::new(),
                }),
                ts: args.node_ts.then(|| TsBindings {
                    output: "crates/client/ts/src".to_string(),
                }),
                wasm: args.node_wasm.then(|| WasmBindings {
                    output: "crates/client/src".to_string(),
                }),
            });
        }
    } else if let Some(rt) = args.rust_transport {
        // No --client given but a transport was: apply it to an existing rust client.
        if let Some(rust) = &mut cfg.generate.clients.rust {
            rust.transport = match rt {
                TransportArg::Wasm => Transport::Wasm,
                TransportArg::Cloud => Transport::Cloud,
            };
        }
    }

    // Ensure a Bindings block exists so derive_defaults can fill it.
    if cfg.generate.bindings.is_none() {
        cfg.generate.bindings = Some(Bindings::default());
    }
}

/// Fill anything still missing via cliclack prompts.
fn prompt_missing(cfg: &mut TrestleConfig) -> Result<()> {
    if cfg.project.name.trim().is_empty() {
        let name: String = cliclack::input("Project name?")
            .interact()
            .map_err(|e| Error::other(format!("prompt failed: {e}")))?;
        cfg.project.name = name;
    }

    // proto_lib
    let pl = cliclack::select("Proto library?")
        .initial_value(cfg.generate.proto_lib)
        .item(ProtoLib::Buffa, "buffa", "required for Connect RPC / WASM")
        .item(ProtoLib::Prost, "prost", "historical default")
        .interact()
        .map_err(|e| Error::other(format!("prompt failed: {e}")))?;
    cfg.generate.proto_lib = pl;

    // servers
    let servers = cliclack::multiselect("Which servers to emit?")
        .item(ServerSel::Rest, "REST (Axum)", "")
        .item(ServerSel::Connect, "Connect RPC", "requires buffa")
        .required(false)
        .interact()
        .map_err(|e| Error::other(format!("prompt failed: {e}")))?;
    cfg.generate.servers = Servers {
        rest: servers.contains(&ServerSel::Rest),
        connect: servers.contains(&ServerSel::Connect),
    };

    // clients
    let clients = cliclack::multiselect("Which clients to emit?")
        .item(ClientSel::Rust, "Rust", "")
        .item(ClientSel::Python, "Python", "")
        .item(ClientSel::Node, "Node", "NAPI / TS / WASM")
        .required(false)
        .interact()
        .map_err(|e| Error::other(format!("prompt failed: {e}")))?;

    if clients.contains(&ClientSel::Rust) && cfg.generate.clients.rust.is_none() {
        cfg.generate.clients.rust = Some(RustClient {
            output: "crates/client/src/gen".to_string(),
            transport: Transport::Cloud,
            transport_type_path: None,
        });
    }
    if clients.contains(&ClientSel::Python) && cfg.generate.clients.python.is_none() {
        cfg.generate.clients.python = Some(PythonClient {
            output: "crates/client/python/src".to_string(),
            error_type: String::new(),
            result_type: String::new(),
            typings_package_filter: None,
        });
    }
    if clients.contains(&ClientSel::Node) && cfg.generate.clients.node.is_none() {
        let wasm = cliclack::confirm("Emit the WASM browser client?")
            .initial_value(false)
            .interact()
            .map_err(|e| Error::other(format!("prompt failed: {e}")))?;
        cfg.generate.clients.node = Some(NodeClient {
            napi: Some(NapiBindings {
                output: "crates/client/node/src".to_string(),
                error_ext_trait: String::new(),
            }),
            ts: None,
            wasm: wasm.then(|| WasmBindings {
                output: "crates/client/src".to_string(),
            }),
        });
    }

    Ok(())
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum ServerSel {
    Rest,
    Connect,
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum ClientSel {
    Rust,
    Python,
    Node,
}

fn write_buf_gen(generate: &GenerateConfig, path: &std::path::Path, force: bool) -> Result<()> {
    if path.exists() && !force {
        return Err(Error::other(format!(
            "{} already exists (use --force to overwrite)",
            path.display()
        )));
    }
    std::fs::write(path, emit_buf_gen(generate)).map_err(|e| Error::io_at(path, e))
}
