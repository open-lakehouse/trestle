# olai-auth

Bearer-token authentication and an authorization port for [axum](https://docs.rs/axum)
services. It verifies the `Authorization: Bearer` header of each request and hands
handlers a `Principal`, and an `Authorizer` decides what that principal may do.

- **JWT verification**: EdDSA (Ed25519) only, with the algorithm pinned. Checks an
  exact `iss`, an `aud` allow-list, and `exp`/`nbf` with a small leeway.
- **Key sources**: keys are cached by `kid`, and an unknown `kid` triggers a
  rate-limited refresh.
  - `PushKeySource` takes a JWKS document the host fetched itself, so no HTTP client
    is linked.
  - `HttpKeySource` (feature `http`) fetches a JWKS URL.
- **API keys**: `olk_<id>_<secret>` keys authenticate service accounts. An
  `ApiKeyStore` looks the key up by id, and the SHA-256 of the whole key is compared
  in constant time. Live keys are cached for a short time (10 s by default), so a
  revoked key stops working within that time. `PgApiKeyStore` (feature `postgres`)
  reads them from Postgres.
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
    .api_keys(PgApiKeyStore::new(pool))
    .resource_metadata_url("https://api.example.com/.well-known/oauth-protected-resource")
    .build()?;

let app = Router::new()
    .route("/health", get(health))
    .merge(ProtectedResourceMetadata::new("https://api.example.com", "https://auth.example.com").router())
    .merge(auth_layer(Router::new().route("/items", get(list_items)), authenticator));
```

## Authorization

An `Authorizer` answers "may this principal perform this action on this resource?"
without the handler knowing where the Cedar policies run:

- `HttpAuthorizer` (feature `pdp-client`) posts to a remote policy decision point's
  `/v1/authorize` and `/v1/authorize/batch` with a per-service bearer secret. The
  request and response types are in `olai_auth::wire`, so the PDP can share them.
- `CedarAuthorizer` (feature `cedar`) evaluates the policies in-process. The policies
  are validated against the schema in strict mode at construction. The host's
  `EntityLoader` supplies the entities (users, groups, grants) per request. The caller
  can pass the entities it owns the data of (the resource and its ancestors) in
  `entities`; they are merged with the loaded ones. The caller may not supply the
  principal, nor set the attributes the loader owns (`privileges` and
  `inherited_privileges` by default), so it can't forge grants.
- `CachedAuthorizer` reuses decisions for a few seconds (5 s by default).

All of them fail closed: a timeout (2 s by default for the PDP), an unreachable PDP,
a loader error or a request that doesn't match the schema is a deny, with the reason
in `errors`. `EntityTypes` maps principals and actions to the service's Cedar entity
types, and `AuthContext` builds the standard context: `auth_method`, `client_id`,
`scopes`, `token_issued_at`, `source_ip`, `request_time` and `branch`.

```rust,ignore
let types = EntityTypes::new("Docs::User", "Docs::ServiceAccount", "Docs::Action");
let authz = CachedAuthorizer::new(HttpAuthorizer::new(pdp_url, pdp_secret));

let request = types
    .request(&principal, "read", EntityRef::new("Docs::Doc", doc_id))
    .with_context(AuthContext::for_principal(&principal).request_time(SystemTime::now()));
if !authz.authorize(&request).await.is_allowed() {
    return Err(StatusCode::FORBIDDEN);
}
```

## Grants

A `Grant` is a `(subject, privilege, object)` tuple: a user, service account or group
holds a privilege (`READ`, `OWNER`, …, names the Cedar policies give meaning to) on a
Cedar entity. Groups contain users, service accounts and other groups. A `GrantStore`
lists, applies (add and remove, atomically), deletes and renames them (with the
descendants of a hierarchical id, `a` → `a.b`), and resolves a subject's groups and
effective grants. `MemoryGrantStore` keeps them in memory; `PgGrantStore` (feature
`postgres`) in the tables the `authz` migrations create.

`GrantsEntityLoader` (feature `cedar`) is an `EntityLoader` over a `GrantStore`. For a
request it loads the principal with its groups as parents, every group it is in, and,
on the resource and every caller-supplied entity, the grants the principal holds there:
`privileges` (on that entity itself) and `inherited_privileges` (on it or any ancestor
the caller supplied). So a policy can tell a privilege on a container from one on the
resource:

```cedar
permit (principal, action == Ns::Action::"GetDoc", resource is Ns::Doc)
when {
  resource.inherited_privileges.contains("READ") &&
  resource.folder.privileges.contains("USE_FOLDER")
};
```

A host adds its own facts about the principal (a role, a ban) with `PrincipalAttrs`. A
batch asks the store once per principal.

## Migrations

`migrate(&pool, "authz")` (feature `postgres`) applies the `authz` migrations
(`MIGRATOR`): `service_accounts`, `api_keys`, `groups`, `memberships` and `grants`, and
an `authz_reader` role with `SELECT` on them. They and their ledger live in the given
schema, apart from the host's own migrations.

## Responses

| Response | When |
|---|---|
| 401, `WWW-Authenticate: Bearer resource_metadata="…"` | no bearer token |
| 401, `WWW-Authenticate: Bearer error="invalid_token", error_description="…", resource_metadata="…"` | the token or API key was rejected, or its user is unknown |
| 403 | the user is banned |
| 503 | the key source, the user status store or the API key store failed |

## Features

| Feature | Adds |
|---|---|
| `http` | `HttpKeySource`, using `reqwest` with rustls |
| `postgres` | `PgApiKeyStore`, `PgGrantStore` and the `authz` migrations (`migrate`, `MIGRATOR`), using `sqlx` with its `postgres`, `migrate` and `macros` features (the host's sqlx brings the runtime and TLS) |
| `pdp-client` | `HttpAuthorizer`, using `reqwest` with rustls (shared with `http`) |
| `cedar` | `CedarAuthorizer` and `GrantsEntityLoader`, using `cedar-policy` with the `datetime`, `decimal` and `ipaddr` extensions. Needs Rust 1.89; the rest of the crate builds on 1.88. Adds about 4.8 MB to a release binary (1.9 MB zipped). |

## License

Apache-2.0
