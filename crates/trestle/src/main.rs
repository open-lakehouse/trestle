use std::io::IsTerminal;

use olai_trestle::{Error, cli};

fn main() {
    init_logging();

    if let Err(err) = cli::run() {
        report_error(&err);
        std::process::exit(1);
    }
}

/// Render an error to stderr: a red `error:` headline, the `caused by:` chain,
/// and — when present — a `try:` suggestion. Colors are emitted only when stderr
/// is a terminal so piped/CI output stays clean.
fn report_error(err: &Error) {
    let color = std::io::stderr().is_terminal();
    let (red, yellow, dim, reset) = if color {
        ("\x1b[31m", "\x1b[33m", "\x1b[2m", "\x1b[0m")
    } else {
        ("", "", "", "")
    };

    // Peel the hint wrapper so the real cause is the headline, not a
    // transparent duplicate, and the hint is shown last.
    let (head, hint): (&dyn std::error::Error, Option<&str>) = match err {
        Error::WithHint { source, hint } => (source.as_ref(), Some(hint.as_str())),
        other => (other, other.hint()),
    };

    eprintln!("{red}error:{reset} {head}");
    let mut src = std::error::Error::source(head);
    while let Some(s) = src {
        eprintln!("  {dim}caused by:{reset} {s}");
        src = s.source();
    }
    if let Some(hint) = hint {
        eprintln!("{yellow}try:{reset} {hint}");
    }
}

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("TRESTLE_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,olai_trestle=info")),
        )
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();
}
