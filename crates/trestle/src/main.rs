use trestle::cli;

fn main() {
    init_logging();

    if let Err(err) = cli::run() {
        eprintln!("error: {err}");
        // Walk the error chain so users see the underlying cause.
        let mut src = std::error::Error::source(&err);
        while let Some(s) = src {
            eprintln!("  caused by: {s}");
            src = s.source();
        }
        std::process::exit(1);
    }
}

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("TRESTLE_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,trestle=info")),
        )
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();
}
