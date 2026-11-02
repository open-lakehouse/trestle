# olai-auth

Bearer-token authentication for [axum](https://docs.rs/axum) services. It verifies
the `Authorization: Bearer` header of each request and hands handlers a `Principal`.

- **JWT verification**: EdDSA (Ed25519) only, with the algorithm pinned. Checks an
  exact `iss`, an `aud` allow-list, and `exp`/`nbf` with a small leeway.
- **Key sources**: keys are cached by `kid`, and an unknown `kid` triggers a
  rate-limited refresh.
  - `PushKeySource` takes a JWKS document the host fetched itself, so no HTTP client
    is linked.
  - `HttpKeySource` (feature `http`) fetches a JWKS URL.
- **User status**: a required, cached lookup of whether a user is banned or an admin,
  implemented by the host against its identity provider's user records. Token claims
  are never used for either.
- **`auth_layer`**: puts the `Principal` into the request extensions. A rejected
  request gets a 401 with an RFC 6750 `WWW-Authenticate` challenge, carrying
  `resource_metadata` (RFC 9728) so OAuth clients can discover the authorization
  server.
- **`ProtectedResourceMetadata`**: serves the
  `/.well-known/oauth-protected-resource` document.

```rust,ignore
let keys = PushKeySource::new();
keys.set_jwks(&jwks_json)?;

let verifier = JwtVerifier::new("https://auth.example.com", keys)
    .with_audience("https://api.example.com");
let authenticator = Authenticator::builder(MyUserStatusStore)
    .issuer(verifier)
    .resource_metadata_url("https://api.example.com/.well-known/oauth-protected-resource")
    .build()?;

let app = Router::new()
    .route("/health", get(health))
    .merge(ProtectedResourceMetadata::new("https://api.example.com", "https://auth.example.com").router())
    .merge(auth_layer(Router::new().route("/items", get(list_items)), authenticator));
```

| Response | When |
|---|---|
| 401, `WWW-Authenticate: Bearer resource_metadata="…"` | no bearer token |
| 401, `WWW-Authenticate: Bearer error="invalid_token", error_description="…", resource_metadata="…"` | the token was rejected, or its user is unknown |
| 403 | the user is banned |
| 503 | the key source or the user status store failed |

## Features

| Feature | Adds |
|---|---|
| `http` | `HttpKeySource`, using `reqwest` with rustls |

## License

Apache-2.0
