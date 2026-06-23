# olai-http-wasm

Browser/WASM HTTP transport for [`olai-codegen`](https://crates.io/crates/olai-codegen)-generated clients.

This is the WASM counterpart to [`olai-http`](https://crates.io/crates/olai-http)'s `CloudClient`. It exposes the exact
surface a generated client body uses — per-verb builders (`get`/`post`/`put`/`patch`/`delete`)
returning a builder with `.json(..)`/`.query(..)`/`.send()`, whose response has
`.status()`/`.bytes()` — so the **same** generated code compiles against either transport. Select
it at generation time via `CodeGenConfig::transport_type_path` (or `transport: wasm` in
`trestle.yaml`).

## How it differs from `CloudClient`

- **No request signing**, no `ring`/`tokio`/`hyper`, no cloud-credential discovery. The wasm
  dependency tree is just `reqwest` (its browser/Fetch backend) + `web-sys`.
- **The browser manages the session.** On `wasm32`, `send()` asks `fetch` to include credentials
  (`RequestCredentials::Include`), so cookies / auth headers are attached automatically for
  same-origin (or CORS-with-`Access-Control-Allow-Credentials`) requests.
- On a **native** target the credential step is a no-op, so the crate builds and unit-tests
  off-wasm.

## Building for the browser

```bash
cargo build -p olai-http-wasm --target wasm32-unknown-unknown
# downstream generated client crate -> package with wasm-pack / wasm-bindgen
```

## JS/TS bindings

`olai-codegen` can emit a `#[wasm_bindgen]` wrapper layer + `client.d.ts` over a client built on
this transport (set `output.wasm` / `transport: wasm` + `runtime: buffa`). The generated bindings
exchange plain JS objects via `serde-wasm-bindgen`. See the olai-codegen README, "JS/TS bindings".
