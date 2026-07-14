# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
