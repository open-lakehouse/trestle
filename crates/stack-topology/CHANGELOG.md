# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.6](https://github.com/open-lakehouse/trestle/compare/olai-stack-topology-v0.0.5...olai-stack-topology-v0.0.6) - 2026-10-17

### Added

- [**breaking**] optional gateway with a direct-ports mode (#147)
- add a postgres `data` knob for isolated test runs (#146)
- add a unity-catalog `writable_mounts` knob (#145)
- [**breaking**] P3b — UC knobs, DB-registered storage, inline secrets, endpoint aliases (#143)
- model refs, shared projection, and plan.json export (#142)
- Rivers orchestration module with gated dedicated listeners (#141)
- serve MLflow's model registry from Unity Catalog (#138)
- [**breaking**] hosted provider modules replace external marking; one provider per role (#136)

### Fixed

- docs aws-sim parity — Envoy 1.38, sts-shim init, bucket region header, publish_admin and bucket knobs (#144)
- let marimo install from a package-index mirror (#140)

## [0.0.5](https://github.com/open-lakehouse/trestle/compare/olai-stack-topology-v0.0.4...olai-stack-topology-v0.0.5) - 2026-10-15

### Added

- PuppyGraph graph queries over Unity Catalog (#133)

### Changed

- name modules by product; drop databricks-emulator-env (#135)

## [0.0.4](https://github.com/open-lakehouse/trestle/compare/olai-stack-topology-v0.0.3...olai-stack-topology-v0.0.4) - 2026-10-14

### Added

- hosted marimo editor for the Python client workspace (#131)
- [**breaking**] always-on gateway with sign-in and policy as gateway settings (#127)
- defaults-first quick start for `trestle env new` (#125)
- Python client workspace (uv + marimo notebooks) for trestle env (#126)
- external (cloud-provided) resources in hybrid environments (#123)
- [**breaking**] pluggable deploy targets (DeployTarget trait + Compose target) (#122)
- edit the environment on the topology canvas; databases and buckets in provider cards (#118)
- [**breaking**] Omnigent agent platform with MLflow 3.17's AI Gateway (#117)
- client env, OPA authorization, and gateway auth fixes (#115)
- [**breaking**] functional environment topology with the gateway's API surface (#114)
- [**breaking**] vend UC S3 credentials via RustFS and emulated AWS (#113)
- *(env)* harden trestle env scenarios, knobs, and Compose secrets (#110)

### Changed

- [**breaking**] split planning from rendering ahead of pluggable deploy targets (#121)

### Fixed

- serve auth-enabled stacks over HTTPS so Authelia boots (#120)
- validate resource names and prefer selected providers in the planner; Azurite containers label (#119)

## [0.0.3](https://github.com/open-lakehouse/trestle/compare/olai-stack-topology-v0.0.2...olai-stack-topology-v0.0.3) - 2026-07-14

### Added

- snake_case JSON field names for buffa models (#95)

## [0.0.2](https://github.com/open-lakehouse/trestle/compare/olai-stack-topology-v0.0.1...olai-stack-topology-v0.0.2) - 2026-06-30

### Added

- headwaters migrate init job + environment-level extra resources (#71)
- persist environment choices (EnvManifest) + layout report (#70)
- Authelia forward-auth for the gateway via ENVOY_AUTH knob (#66)
- headwaters knobs, generated config.toml + healthcheck (#64)
- add headwaters lineage module (#63)
- network-only backends + configurable gateway ports (#62)
- mount config files via compose configs: aliases (#61)
- typed connection model + credential-aware rendering (#56)
- materialized artifacts, resource demands, and provider choice (#55)
- module catalog + collision-free route planner (#54)

### Changed

- [**breaking**] ergonomic phase-oriented API + module regrouping (#69)
- [**breaking**] collapse single-variant RenderSpec to a struct (#68)
- [**breaking**] drop dead public API (#67)
- [**breaking**] make Module a trait with knob-computed topology (#65)
- unify the render engine and concretize compose wiring (#58)
- self-contained module templates + object-store gateway routing (#57)
