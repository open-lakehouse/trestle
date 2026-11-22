# syntax=docker/dockerfile:1
#
# A test-only image of the `sds_probe` example: a consumer that reads its bearer token over
# SDS and logs each new value's SHA-256. The Docker tests build it from the workspace root:
#
#   docker build -f crates/olai-http/tests/sds-probe.Dockerfile -t trestle-sds-probe .
#
# The base images and the optional `cargo-config` build secret match
# crates/secret-server/Dockerfile.

FROM rust:1.89-slim-bookworm@sha256:d7fc7de78bb8c1469933aeecbf801314d30d7d6e9f0578bba4cfa285bfa37fe6 AS build
WORKDIR /src
COPY . .
RUN --mount=type=secret,id=cargo-config,target=/usr/local/cargo/config.toml,required=false \
    --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --locked --release -p olai-http --features sds --example sds_probe \
    && cp target/release/examples/sds_probe /sds_probe

FROM gcr.io/distroless/cc-debian12@sha256:e5d81ddde149641e2a9ba55be4545bc125c67de07508b03ba4c22e6eb0ded5aa
COPY --from=build /sds_probe /sds_probe
ENTRYPOINT ["/sds_probe"]
