# syntax=docker/dockerfile:1
#
# ghcr.io/open-lakehouse/trestle-secret-server. Build from the workspace root:
#
#   docker build -f crates/secret-server/Dockerfile -t trestle-secret-server .
#
# Base images are pinned by digest (multi-arch indexes); bump them together with the
# Rust toolchain in .github/workflows.
#
# Where crates.io is only reachable through a registry mirror, pass a Cargo config with
# the source replacement as the optional `cargo-config` build secret:
#
#   docker build --secret id=cargo-config,src=$HOME/.cargo/config.toml -f crates/secret-server/Dockerfile .

FROM rust:1.89-slim-bookworm@sha256:d7fc7de78bb8c1469933aeecbf801314d30d7d6e9f0578bba4cfa285bfa37fe6 AS build
WORKDIR /src
COPY . .
RUN --mount=type=secret,id=cargo-config,target=/usr/local/cargo/config.toml,required=false \
    --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --locked --release -p olai-secret-server \
    && cp target/release/olai-secret-server /olai-secret-server

# Runs as root on purpose: each consumer's socket lives on a volume mounted per consumer,
# which the image can't pre-own for a non-root user, and the mount (not the socket's
# owner) is what identifies a consumer. Nothing else runs in the image.
FROM gcr.io/distroless/cc-debian12@sha256:e5d81ddde149641e2a9ba55be4545bc125c67de07508b03ba4c22e6eb0ded5aa
LABEL org.opencontainers.image.title="trestle-secret-server" \
      org.opencontainers.image.description="SDS secret server for trestle's egress gateway" \
      org.opencontainers.image.source="https://github.com/open-lakehouse/trestle" \
      org.opencontainers.image.licenses="Apache-2.0"
COPY --from=build /olai-secret-server /olai-secret-server
ENTRYPOINT ["/olai-secret-server"]
