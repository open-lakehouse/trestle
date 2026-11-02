//! The crate's [`Error`].

use std::process::ExitStatus;

/// What can go wrong driving a [`ComposeEngine`](crate::ComposeEngine).
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The engine can't be reached: the `docker` binary is missing, or its daemon isn't
    /// running. Distinct from "nothing is running", which is an empty listing.
    #[error("Docker is not available: {reason}")]
    Unavailable {
        /// Why, as the CLI or the OS put it.
        reason: String,
    },
    /// A command ran and failed.
    #[error("`{command}` failed ({status}){}", stderr_suffix(.stderr))]
    Failed {
        /// The command line, without the program (e.g. `compose up -d --wait`).
        command: String,
        /// Its exit status.
        status: ExitStatus,
        /// What it printed on stderr, when it was captured.
        stderr: String,
    },
    /// A command's output didn't parse as expected.
    #[error("parsing the output of `{command}`: {source}")]
    Parse {
        /// The command line, without the program.
        command: String,
        /// The JSON error.
        #[source]
        source: serde_json::Error,
    },
    /// Spawning or talking to a command failed for another reason.
    #[error("running `{command}`: {source}")]
    Io {
        /// The command line, without the program.
        command: String,
        /// The I/O error.
        #[source]
        source: std::io::Error,
    },
}

fn stderr_suffix(stderr: &str) -> String {
    let stderr = stderr.trim();
    if stderr.is_empty() {
        String::new()
    } else {
        format!(": {stderr}")
    }
}

impl Error {
    /// Whether this is [`Error::Unavailable`].
    pub fn is_unavailable(&self) -> bool {
        matches!(self, Error::Unavailable { .. })
    }
}

/// The crate's result type.
pub type Result<T, E = Error> = std::result::Result<T, E>;
