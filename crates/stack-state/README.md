# olai-stack-state

The state store the [trestle](https://github.com/open-lakehouse/trestle) CLI and the
desktop app share. It is one SQLite database under `$TRESTLE_HOME`, defaulting to the
platform data directory, and it holds:

- **workspaces** with their vars
- **environments**: an editable draft (the placement-free `env.toml`, the `module.yaml`
  texts it lists, vars) and a head
- **revisions**: numbered, immutable snapshots of a draft, with a digest, the time they
  were committed and an optional message
- **instances**: a revision to run, its desired state, overrides and overlay sources
- **slot leases**: host-wide, so two instances never share a slot, whatever their
  workspace

Secrets are stored as references (`SecretRef`); their values stay with their provider.
Several processes can use the store at once. Writes are serialized, and a `Watcher`
notices commits made elsewhere. `trestle-workspace.toml` and environment directories
can be imported, and exported again with the instances' slots.

```rust,no_run
use olai_stack_state::{Draft, Store};
use olai_stack_topology::{EnvManifest, PlanCtx, Selection};

# fn run() -> olai_stack_state::Result<()> {
let store = Store::open_default()?;
let ws = store.create_workspace("demo", false)?;
let manifest = EnvManifest::new(Selection::modules(["unity-catalog-oss"]), PlanCtx::default());
let env = store.create_environment(&ws.id, "dev", Draft::new(manifest))?;
let rev = store.commit_draft(&env.id)?;
let instance = store.create_instance(&env.id, rev.number())?;
println!("slot {}", store.lease_slot(&instance.id)?.slot);
# Ok(())
# }
```
