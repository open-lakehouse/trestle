# olai-stack-runtime

Runs [trestle](https://github.com/open-lakehouse/trestle) environments on Docker
Compose. `olai-stack-topology` plans and renders an environment; this crate starts,
stops and watches it. The `trestle` CLI and the desktop app share it.

- **`ComposeEngine`**: up, down, ls, ps, logs, events and network removal, as an
  async trait.
  - `DockerCli` runs the `docker` CLI and tells "Docker is not available" apart
    from "nothing is running".
  - `FakeEngine` (feature `testing`) is an in-memory stand-in for tests.
- **Typed status**: `ProjectStatus`, `ServiceStatus` with `State` and `Health`,
  `EngineEvent` and `LogLine`, parsed from the CLI's JSON output. The test fixtures
  are captured from real engines.
- **`probe_host_ports`**: whether a plan's host ports are free before starting it.
- **`managed_projects`, `remove_mesh_if_idle`**: match what Docker runs against
  what the caller manages.

- **`Instances`**: runs the revisions in an `olai-stack-state` store, each in its own
  compose project, slot and host label, so two revisions of one environment run side by
  side.
  - It starts, stops and removes instances, reports their health and events, and
    reconciles with Docker after a restart.
  - The rendered tree is a cache under the state directory and holds no credential.
- **`EffectiveEnv`**: an instance's variables from every layer (catalog defaults,
  revision, placement, client env, workspace/environment/revision vars, source
  overlays, instance overrides), with provenance and a masked view.
- **`SecretProvider`**: `EnvProvider`, plus `KeychainProvider` behind feature
  `keychain`. A reference to a provider that isn't configured is an error, never a
  fallback.
- **`Source`**: overlays of variables, endpoints and expiring credentials from outside
  the store. `UserVars` is built in.

Credentials go to Compose as process environment (`UpOptions::env`), never through a
file.

```rust,no_run
use olai_stack_runtime::{ComposeEngine, DockerCli, UpOptions};

# async fn run() -> olai_stack_runtime::Result<()> {
let docker = DockerCli::new();
let opts = UpOptions { wait: true, ..UpOptions::default() };
docker.up(std::path::Path::new("demo/dev"), &opts).await?;
for project in docker.ls().await? {
    println!("{}: {}", project.name, project.status);
}
# Ok(())
# }
```

See [`docs/runtime.md`](https://github.com/open-lakehouse/trestle/blob/main/docs/runtime.md)
for the captured engine behaviour and how credentials are handled.
