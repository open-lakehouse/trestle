# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.15](https://github.com/open-lakehouse/trestle/compare/olai-trestle-v0.0.14...olai-trestle-v0.0.15) - 2026-10-26

### Added

- *(trestle)* trestle workspace commands for side-by-side environments (#169)

## [0.0.14](https://github.com/open-lakehouse/trestle/compare/olai-trestle-v0.0.13...olai-trestle-v0.0.14) - 2026-10-25

### Added

- [**breaking**] async Python clients via pyo3-async-runtimes (#161)

## [0.0.13](https://github.com/open-lakehouse/trestle/compare/olai-trestle-v0.0.12...olai-trestle-v0.0.13) - 2026-10-23

### Added

- export environment diagrams as Mermaid for Markdown (#157)
- download the configured environment as a zip with fresh credentials (#156)

### Changed

- [**breaking**] name the Unity Catalog module's model unityCatalogServer (#158)

## [0.0.12](https://github.com/open-lakehouse/trestle/compare/olai-trestle-v0.0.11...olai-trestle-v0.0.12) - 2026-10-18

### Added

- [**breaking**] deployment steps README with vendor setup for hosted Postgres and storage (#152)

## [0.0.11](https://github.com/open-lakehouse/trestle/compare/olai-trestle-v0.0.10...olai-trestle-v0.0.11) - 2026-10-17

### Added

- [**breaking**] per-node config files modal with highlighting; Authelia users.yml beside a README (#151)

## [0.0.10](https://github.com/open-lakehouse/trestle/compare/olai-trestle-v0.0.9...olai-trestle-v0.0.10) - 2026-10-17

### Added

- [**breaking**] optional gateway with a direct-ports mode (#147)
- [**breaking**] P3b — UC knobs, DB-registered storage, inline secrets, endpoint aliases (#143)
- model refs, shared projection, and plan.json export (#142)
- Rivers orchestration module with gated dedicated listeners (#141)
- [**breaking**] hosted provider modules replace external marking; one provider per role (#136)

### Fixed

- docs aws-sim parity — Envoy 1.38, sts-shim init, bucket region header, publish_admin and bucket knobs (#144)

## [0.0.9](https://github.com/open-lakehouse/trestle/compare/olai-trestle-v0.0.8...olai-trestle-v0.0.9) - 2026-10-15

### Added

- PuppyGraph graph queries over Unity Catalog (#133)

### Changed

- name modules by product; drop databricks-emulator-env (#135)

## [0.0.8](https://github.com/open-lakehouse/trestle/compare/olai-trestle-v0.0.7...olai-trestle-v0.0.8) - 2026-10-14

### Added

- hosted marimo editor for the Python client workspace (#131)
- [**breaking**] always-on gateway with sign-in and policy as gateway settings (#127)
- defaults-first quick start for `trestle env new` (#125)
- Python client workspace (uv + marimo notebooks) for trestle env (#126)
- external (cloud-provided) resources in hybrid environments (#123)
- [**breaking**] pluggable deploy targets (DeployTarget trait + Compose target) (#122)
- client env, OPA authorization, and gateway auth fixes (#115)
- [**breaking**] vend UC S3 credentials via RustFS and emulated AWS (#113)
- *(env)* harden trestle env scenarios, knobs, and Compose secrets (#110)
- *(trestle)* `trestle env` for composed environments; remove the duplicate compose path from `trestle new` (#108)

### Changed

- [**breaking**] split planning from rendering ahead of pluggable deploy targets (#121)

### Fixed

- serve auth-enabled stacks over HTTPS so Authelia boots (#120)

## [0.0.7](https://github.com/open-lakehouse/trestle/compare/olai-trestle-v0.0.6...olai-trestle-v0.0.7) - 2026-07-19

### Added

- dual transport and wasm token cache (#107)
- alias and repair OpenAPI schema keys in enrich-openapi (#104)

## [0.0.6](https://github.com/open-lakehouse/trestle/compare/olai-trestle-v0.0.5...olai-trestle-v0.0.6) - 2026-07-14

### Added

- recompose derived full_name on read + idempotent serde rename swap (#100)
- snake_case JSON field names for buffa models (#95)
- *(trestle)* format generated code in `trestle generate` (#94)
- [**breaking**] trestle generate owns the buf pipeline + usability fixes (#92)

## [0.0.5](https://github.com/open-lakehouse/trestle/compare/olai-trestle-v0.0.4...olai-trestle-v0.0.5) - 2026-07-12

### Added

- prebuilt binary releases for cargo binstall (#86)

### Fixed

- pass full tag ref to upload action; drop Intel macOS (#87)

## [0.0.4](https://github.com/open-lakehouse/trestle/compare/olai-trestle-v0.0.3...olai-trestle-v0.0.4) - 2026-06-30

### Added

- generate ergonomic ConnectRPC clients in olai-codegen (#52)

### Changed

- self-contained module templates + object-store gateway routing (#57)

## [0.0.3](https://github.com/open-lakehouse/trestle/compare/olai-trestle-v0.0.2...olai-trestle-v0.0.3) - 2026-06-28

### Added

- support buffa + Python via generated PyO3 model conversions (#42)
- browser WASM client — the frontend consumes the generated client (#37)
- make buf ConnectRPC a first-class, opt-in template output (F11) (#34)
- golden-path example — REST + buf ConnectRPC on one port, with the generator/template fixes to get there (#29)

### Changed

- structured config + co-located codegen path reconciliation (#45)

### Fixed

- handle proto package versions (#39)

## [0.0.2](https://github.com/open-lakehouse/trestle/compare/olai-trestle-v0.0.1...olai-trestle-v0.0.2) - 2026-06-23

### Documentation

- *(trestle)* document template manifest schema and advanced CLI usage
- tighten crate READMEs + refresh repo docs (#25)
