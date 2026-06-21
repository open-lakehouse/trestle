# olai-http-wasm

Browser/WASM HTTP transport for [`olai-codegen`](../olai-codegen)-generated clients.

This is the WASM counterpart to [`olai_http::CloudClient`](../olai-http). It exposes the exact
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

## Status

Transports request/response only. `#[wasm_bindgen]` JS wrappers and `.d.ts` emission for the
generated client are a future addition (the Rust-level wasm client is independently usable today).
