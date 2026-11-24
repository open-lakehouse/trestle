# olai-key-service

A KMS-style key service, so that a service never holds a key encryption key (KEK). The
service sends its data encryption keys (DEKs) to a key server to be wrapped and unwrapped,
and stores only the wrapped bytes. The server is `olai-secret-server`, which serves this API
on the same per-consumer Unix sockets as SDS (see
[`docs/egress-gateway.md`](../../docs/egress-gateway.md#key-material)).

This crate holds:

- `v1`: the `olai.keys.v1` gRPC API, generated from
  [`proto/olai/keys/v1/keys.proto`](proto/olai/keys/v1/keys.proto) with prost and tonic
  0.14. It has client and server stubs, and needs only tonic's `codegen` feature.
- `KeyServiceProvider` (feature `client`): an `olai_store::KeyProvider` over a
  consumer's socket, for `olai_store::EnvelopeEncryptor`.

## The API

| RPC | What it does |
|---|---|
| `Wrap(key, dek, version?, alias?)` → `{version, wrapped}` | Wraps a DEK under the active version, or under a named version between `min_decrypt` and the active version. |
| `Unwrap(key, version, wrapped, alias?)` → `dek` | Unwraps it. Refused with `FAILED_PRECONDITION` below `min_decrypt`, and with `NOT_FOUND` for an unknown version or alias. |
| `GetKey(key)` → `{active_version, min_decrypt_version, aliases}` | The key's versions. Never any key material. |
| `WatchKey(key)` → stream of the same | The versions now, then again each time they change. |

A consumer may only use the keys its listener lists. A key another consumer holds and a key
nobody holds both answer `NOT_FOUND`.

## KEK ids and the wrap format

Every sealed value records the id of the KEK that wrapped its DEK. Through the key service
that id is `<key>@v<version>` (`kek_id`), or an alias the keyring maps to a version.

A wrapped DEK is AES-256-GCM: a random 96-bit nonce, then the ciphertext and tag. The KEK id
is the associated data. That is exactly what `olai_store::LocalKeyProvider` produces for a
KEK of that id, so keys move between the two in both directions:

- **Import.** A service that used `LocalKeyProvider::single("kek-1", key)` moves to the key
  service by importing `key` as version 1 with the alias `kek-1`. Its existing blobs keep
  opening, and `EnvelopeEncryptor::rewrap` moves them to the active version.
- **Export.** A `LocalKeyProvider` holding version `n` of key `k` under the id `k@vn`
  opens what the service wrapped under that version.

## The client

```rust,no_run
# async fn example() -> olai_store::Result<()> {
use std::sync::Arc;
use std::time::Duration;

use olai_key_service::KeyServiceProvider;
use olai_store::EnvelopeEncryptor;

let keys = KeyServiceProvider::builder("/run/trestle-sds/sds.sock", "server-kek")
    .connect_timeout(Duration::from_secs(30))
    .connect()
    .await?;
let encryptor = EnvelopeEncryptor::new(Arc::new(keys));
let blob = encryptor.seal("row-id", b"value").await?;
assert_eq!(encryptor.open("row-id", &blob).await?, b"value");
# Ok(())
# }
```

- `connect` fetches the key's versions with `GetKey`, and retries while the server is
  unreachable. A key the socket doesn't serve fails at once.
- `active_kek_id()` is answered from a cache that a background `WatchKey` stream keeps
  current, so it does no I/O, and a rotation reaches the running process without a
  restart. When the server goes away (it's being recreated, say), the provider keeps the
  last versions it saw and reconnects with backoff. The watch stops when the provider is
  dropped.
- Wraps and unwraps retry an unreachable server for up to `call_timeout` (10 s by
  default).
- **Unwrapped DEKs are not cached by default.** `dek_cache(ttl, max_entries)` turns on a
  bounded cache that expires its entries. A cached DEK opens its values without a round trip,
  including while the server is away. A raised `min_decrypt` drops the affected entries as
  soon as the watch sees it. Wraps are never cached, so new values need the server.
- `Debug` shows the socket, key, versions and the cache's size, never a DEK.

## Regenerating the stubs

`src/gen/` is generated and checked in, so building this crate needs no `protoc`. After
changing the proto, regenerate it with protox (no `protoc` here either):

```bash
BLESS=1 cargo test -p olai-key-service --test generated
```

Without `BLESS`, that test fails when the checked-in copy is out of date.
`WrapRequest` and `UnwrapResponse` carry plaintext DEKs, so they are generated without
`Debug`. `src/lib.rs` implements a redacting one.
