# syntax=docker/dockerfile:1
#
# A test-only image of the `key_probe` example: a module process that seals values through the
# secret server's key service. The live key test builds it from the workspace root:
#
#   docker build -f crates/stack-runtime/tests/key-probe.Dockerfile -t trestle-key-probe .
#
# The base images and the optional `cargo-config` build secret match
# crates/secret-server/Dockerfile.

FROM rust:1.89-slim-bookworm@sha256:d7fc7de78bb8c1469933aeecbf801314d30d7d6e9f0578bba4cfa285bfa37fe6 AS build
WORKDIR /src
COPY . .
RUN --mount=type=secret,id=cargo-config,target=/usr/local/cargo/config.toml,required=false \
    --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --locked --release -p olai-stack-runtime --example key_probe \
    && cp target/release/examples/key_probe /key_probe

FROM gcr.io/distroless/cc-debian12@sha256:e5d81ddde149641e2a9ba55be4545bc125c67de07508b03ba4c22e6eb0ded5aa
COPY --from=build /key_probe /key_probe
ENTRYPOINT ["/key_probe"]
CMD ["serve"]
