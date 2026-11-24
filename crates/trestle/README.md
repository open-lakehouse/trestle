# olai-trestle (`trestle`)

The unified CLI for the [Trestle](https://github.com/open-lakehouse/trestle)
framework. Published as `olai-trestle`; installs a `trestle` binary.

1. **Project scaffolding**: `trestle new <name>` renders a base template plus
   optional apps (`--app`).
2. **Config bootstrap**: `trestle init` runs a guided interview and writes
   `trestle.yaml` (+ derived `buf.gen.yaml`); `trestle config` reconfigures an
   existing project (both accept the same flags for non-interactive use).
3. **Code generation**: `trestle generate -c trestle.yaml` and
   `trestle enrich-openapi -c trestle.yaml` (see
   [`olai-codegen`](https://crates.io/crates/olai-codegen)).
4. **Dev environments**: `trestle env new <name>` renders a Docker Compose stack
   from selected modules. `trestle ws`, `trestle env create`/`commit` and
   `trestle up`/`down`/`ps` keep workspaces of environments, with numbered
   revisions, in the trestle state store and run them side by side (see
   [multi-env.md](https://github.com/open-lakehouse/trestle/blob/main/docs/multi-env.md)).

## Install

Prebuilt binaries (no Rust toolchain, no compile) via
[`cargo binstall`](https://github.com/cargo-bins/cargo-binstall):

```bash
cargo binstall olai-trestle
```

This downloads the release archive for your platform and installs the `trestle`
command. Prebuilt targets: Linux (x86_64 and aarch64) and macOS (Apple Silicon
and Intel). Other platforms fall back to a source build. Each archive ships with
a `.sha256` checksum and a GitHub build-provenance attestation you can verify:

```bash
gh attestation verify trestle-<target>.tar.gz --repo open-lakehouse/trestle
```

Or build from source:

```bash
cargo install olai-trestle   # or: cargo install --git https://github.com/open-lakehouse/trestle --bin trestle
```

## Scaffolding

| Template | Kind | Description |
|----------|------|-------------|
| `lakehouse` | base (default) | Project skeleton for an open-lakehouse app; the composed dev environment is created separately with `trestle env`. |
| `databricks-app-rust` | app (`--app`) | Service + optional frontend + proto codegen; deployable to Databricks Apps. |

```bash
trestle list-templates
trestle new my-app --app databricks-app-rust
cd my-app
trestle env new my-app --out-dir env   # the composed dev environment
```

`--template` picks the base and accepts an embedded name, a git URL, or a local
path. `--app` layers an app on top and can be repeated; it takes the same forms.
Run `trestle new --help` for all options.

## Categories and components

Each app declares **categories**: named choices with explicit `options:`. Pick
them with `--select app.<app>.<category>=<value>`, or answer the wizard. A
category you don't pick takes its default. `databricks-app-rust` declares:

| Category | Options | Default |
|----------|---------|---------|
| `frontend` | `react`, `none` | `react` |
| `ci` | `github`, `none` | `github` |
| `connect` | `on`, `none` | `none` (`on` forces the buffa runtime) |

The picks turn on the app's **components** (`components:`): subtrees under the
app's `components/` directory, each enabled by a `when:` expression
(`frontend=react` sets `with_frontend`, and so on).

## Advanced usage

### Non-interactive scaffolding (CI)

`--non-interactive` skips every prompt. Each variable must then be resolvable
from a `--set` override, a `--values` file, or its manifest default, or the run
fails with a "missing variable" error. `post_init` hooks marked `confirm: true`
are skipped in this mode (they can't be confirmed without a prompt).

```bash
trestle new my-app \
  --app databricks-app-rust \
  --non-interactive \
  --select app.databricks-app-rust.frontend=react \
  --set gh_owner=acme
```

### Selecting options

`--select app.<app-name>.<category>=<value>[,<value>...]` picks a category's
option instead of answering the wizard. Repeat the flag for multiple categories;
comma-separate values for a multi-select category. Values must be among the
category's options.

Individual variables are set with `--set <name>=<value>` (short form `-D`).

### Values file (`--values <file>`)

For repeatable runs, supply a YAML file with up to three top-level keys:

```yaml
# values.yaml
apps:               # apps to layer on the base (same as repeating --app)
  - databricks-app-rust
selections:         # same shape as --select; value may be a string or a list
  app.databricks-app-rust.frontend: react
  app.databricks-app-rust.ci: github
variables:          # variable overrides (same as --set); only declared variables apply
  gh_owner: acme
  runtime: buffa
```

```bash
trestle new my-app --values values.yaml --non-interactive
```

CLI flags win over the file. A `--set` overrides the same `variables:` key, a
`--select` replaces the same category in `selections:`, and `--app` adds to
`apps:`.

## Running environments

The store-backed commands (`ws`, `env create`/`commit`, `var`, `up`, `down`, `ps`,
`logs`, `health`, `neon`, `state`) are listed in
[`docs/multi-env.md`](../../docs/multi-env.md#cli). Running instances are covered in
[`docs/runtime.md`](../../docs/runtime.md).

### `trestle secret rotate <ws>/<env>[@<n>] <NAME>`

Stores a new value for a secret the secret server holds and hands it to the
running instances without restarting the egress gateway or the consumers. That
secret is an egress route's credential (the variable its `credential.secret`
names) or a secret a module reads over SDS (its `sds_secrets` entry's `env`):

```bash
printf %s "$NEW_KEY" | trestle secret rotate demo/dev SEARCH_API_KEY
# stored SEARCH_API_KEY on demo/dev
# recreated secret-server in demo_dev_r1
```

- The value comes from stdin, or a prompt on a terminal, and never from the command line.
- It is stored the way `trestle var set <ws>/<env> --secret <NAME>` stores a secret: in the
  keychain, referenced from the environment's vars. On a team's state (`--state`) it goes to
  this machine's own vars unless you pass `--shared`, and `--local` always puts it there.
- Each running instance of the environment then gets its `secret-server` recreated with
  the new value. `@<n>` limits that to revision n's instance. If no instance is running,
  the value is stored for the next `trestle up`.
- It refuses a name the secret server doesn't hold (use `trestle var set` for other
  secrets). It also refuses one that a higher layer (revision vars, this machine's vars, or the
  instance's overrides) sets over where it would be stored.

See [`docs/egress-gateway.md`](../../docs/egress-gateway.md) for the egress gateway and
its secret server.

### `trestle key import|rotate|retire|show <ws>/<env>[@<n>] <module>/<key>`

Manages the keyrings of the keys modules seal their data under through the secret
server's key service (`Provides::keys`). The secret server alone holds a keyring; the
module only asks it to wrap and unwrap its data keys.

```bash
trestle key import demo/dev unity-catalog-rs/server-kek --from UC_SERVER_KEK --alias kek-1
trestle key rotate demo/dev unity-catalog-rs/server-kek
# recreated secret-server in demo_dev_r1
trestle key retire demo/dev unity-catalog-rs/server-kek --below 2 --confirm-swept
trestle key show demo/dev unity-catalog-rs/server-kek [--json]
```

- `import` makes version 1 from an existing 32-byte key (standard base64). It reads
  the key from stdin, or from an existing secret with `--from <VAR>`: the
  environment's or workspace's vars, else this shell. `--alias` keeps the id the key
  had before, so what it sealed still opens.
- `rotate` adds a random version and makes it active. If the key has no keyring yet,
  it mints version 1.
- `retire --below <n>` raises `min_decrypt`. It refuses an `n` above the active
  version, and refuses without `--confirm-swept`: the consumer's sweep (olai-store's
  `rewrap_all`, then `kek_usage`) must report zero blobs below `n` first.
- `show` prints versions, the active version, `min_decrypt` and aliases, never key
  material.
- Writes are **add-only**. None removes a version's material, and a write that would
  drop a version is refused. Each one reads the stored keyring back to check it, then
  recreates `secret-server` in the running instances. `var set`, `var unset` and
  `secret rotate` refuse keyring variables, and `env rm` / `ws rm` keep their
  keychain entries.
- The keychain holds the only copy of each keyring, so back it up. On a team's state
  (`--state`), writes are refused because each machine has its own keychain.

## Troubleshooting

- **`git ... failed` when using a git template.** Git templates are cloned with
  the system `git` binary, so `git` must be on `PATH`. Install it (or use an
  embedded name / local path with `--template`) and retry.
- **"missing variable" under `--non-interactive`.** A variable had no override,
  no `--values` entry, and no manifest default. Supply it with `--set` or in the
  `--values` file.
- **Output directory already exists and is non-empty.** Pass `--force` to render
  into it anyway; existing files at generated paths are overwritten, others are
  left untouched.

## Authoring a template

```text
my-template/
├── template.yaml     # manifest: variables, categories, components, post_init
├── template/         # files copied + MiniJinja-rendered (.tmpl suffix stripped)
├── components/       # local components (one directory each, with a template.yaml)
└── hooks/            # optional post-init scripts
```

Filenames *and* contents render with [MiniJinja](https://docs.rs/minijinja),
with extra case filters (`snake_case`, `kebab_case`, `pascal_case`, `camel_case`,
`screaming_snake_case`, `upper_case`, `lower_case`). A `.trestle-ignore` file
(gitignore syntax) at the template root excludes paths from copying.

```yaml
# template.yaml
name: my-template
variables:
  - { name: project_name, prompt: "Name", validate: "^[a-z][a-z0-9-]*$" }
  - { name: with_x, type: bool, default: true }
components:
  - { name: x, when: "with_x" }   # path defaults to components/x
post_init:
  - { run: "git init && git add -A", confirm: true, description: "Initialise git" }

# component template.yaml (components/x/template.yaml)
name: x
depends_on: []   # other components to enable with this one
```

## Tests

```bash
cargo test -p olai-trestle --test scaffold       # fast: render + structure checks
TRESTLE_TEST_SLOW=1 cargo test -p olai-trestle --test scaffold   # also cargo check + compose config
```

## License

Apache-2.0
