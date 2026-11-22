//! `olai-secret-server`: serves SDS on the sockets `secret-server.yaml` lists, with values
//! from the process environment.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use olai_secret_server::{Config, serve};
use tracing_subscriber::EnvFilter;

/// Serve Envoy's secret discovery service on one Unix socket per consumer.
#[derive(Debug, Parser)]
#[command(name = "olai-secret-server", version)]
struct Args {
    /// The configuration file. It names variables, never values.
    #[arg(
        long,
        env = "SECRET_SERVER_CONFIG",
        default_value = "/etc/trestle/secret-server.yaml"
    )]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
    let args = Args::parse();

    let config = match Config::load(&args.config) {
        Ok(config) => config,
        Err(e) => return fail(&e),
    };
    let policy = match config.resolve(|name| std::env::var_os(name)) {
        Ok(policy) => policy,
        Err(e) => return fail(&e),
    };
    match serve(&config, policy, shutdown_signal()).await {
        Ok(()) => {
            tracing::info!("shut down");
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e),
    }
}

fn fail(error: &dyn std::error::Error) -> ExitCode {
    tracing::error!("{error}");
    ExitCode::FAILURE
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending().await,
        }
    };
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
