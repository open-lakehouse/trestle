//! RustFS: S3 with an STS endpoint that enforces session policies.
//!
//! RustFS implements STS `AssumeRole` with an inline `Policy`, and evaluates the
//! intersection on every request, `s3:prefix` list conditions included. That makes it
//! the emulator for proving an S3 session policy actually denies what it should.

use std::time::Duration;

use testcontainers::core::{CmdWaitFor, ExecCommand, IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, Healthcheck, ImageExt};

/// RustFS image tag; the same tag stack-topology deploys.
pub const RUSTFS_TAG: &str = "1.0.1";

/// Root access key of the emulator.
pub const ROOT_ACCESS_KEY: &str = "vendingroot";

/// Root secret key of the emulator.
pub const ROOT_SECRET_KEY: &str = "vendingroot-secret";

/// The region RustFS signs and validates requests for.
pub const REGION: &str = "us-east-1";

const PORT: u16 = 9000;

/// A running RustFS container. Stopped and removed on drop.
pub struct RustFs {
    container: ContainerAsync<GenericImage>,
    endpoint: String,
}

impl RustFs {
    /// Starts RustFS and waits until it serves requests.
    ///
    /// # Panics
    ///
    /// If Docker is unavailable or the container does not become healthy.
    pub async fn start() -> Self {
        let container = GenericImage::new("rustfs/rustfs", RUSTFS_TAG)
            .with_exposed_port(PORT.tcp())
            .with_wait_for(WaitFor::healthcheck())
            .with_env_var("RUSTFS_ACCESS_KEY", ROOT_ACCESS_KEY)
            .with_env_var("RUSTFS_SECRET_KEY", ROOT_SECRET_KEY)
            .with_env_var("RUSTFS_VOLUMES", "/data")
            .with_env_var("RUSTFS_ADDRESS", format!(":{PORT}"))
            .with_health_check(
                Healthcheck::cmd_shell(format!("curl -s -o /dev/null http://localhost:{PORT}/"))
                    .with_interval(Duration::from_millis(500))
                    .with_retries(60),
            )
            .with_startup_timeout(Duration::from_secs(120))
            .start()
            .await
            .expect("start RustFS (is Docker running?)");
        let host = container.get_host().await.expect("RustFS host");
        let port = container
            .get_host_port_ipv4(PORT.tcp())
            .await
            .expect("RustFS port");
        Self {
            container,
            endpoint: format!("http://{host}:{port}"),
        }
    }

    /// The S3 and STS endpoint, as reachable from the host (`http://host:port`).
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Creates `bucket`, signing the request with the root credentials.
    ///
    /// Runs `curl --aws-sigv4` inside the container, so the harness needs no S3 client.
    ///
    /// # Panics
    ///
    /// If the bucket cannot be created.
    pub async fn create_bucket(&self, bucket: &str) {
        let mut result = self
            .container
            .exec(
                ExecCommand::new([
                    "curl",
                    "-sS",
                    "--fail-with-body",
                    "--aws-sigv4",
                    &format!("aws:amz:{REGION}:s3"),
                    "--user",
                    &format!("{ROOT_ACCESS_KEY}:{ROOT_SECRET_KEY}"),
                    "-X",
                    "PUT",
                    &format!("http://localhost:{PORT}/{bucket}"),
                ])
                .with_cmd_ready_condition(CmdWaitFor::exit()),
            )
            .await
            .expect("exec curl in RustFS");
        let code = result.exit_code().await.expect("curl exit code");
        if code != Some(0) {
            let stdout = result.stdout_to_vec().await.unwrap_or_default();
            let stderr = result.stderr_to_vec().await.unwrap_or_default();
            panic!(
                "creating bucket {bucket} failed ({code:?}): {}{}",
                String::from_utf8_lossy(&stdout),
                String::from_utf8_lossy(&stderr),
            );
        }
    }
}
