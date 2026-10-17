# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0](https://github.com/open-lakehouse/trestle/compare/trestle-diagram-v0.1.2...trestle-diagram-v0.2.0) - 2026-10-17

### Added

- [**breaking**] optional gateway with a direct-ports mode (#147)
- let hosts render a diagram node's name and role lines (#148)
- model refs, shared projection, and plan.json export (#142)
- Rivers orchestration module with gated dedicated listeners (#141)
- [**breaking**] hosted provider modules replace external marking; one provider per role (#136)

### Fixed

- docs aws-sim parity — Envoy 1.38, sts-shim init, bucket region header, publish_admin and bucket knobs (#144)

## [0.1.2](https://github.com/open-lakehouse/trestle/compare/trestle-diagram-v0.1.1...trestle-diagram-v0.1.2) - 2026-10-15

### Added

- PuppyGraph graph queries over Unity Catalog (#133)

### Changed

- name modules by product; drop databricks-emulator-env (#135)

## [0.1.1](https://github.com/open-lakehouse/trestle/compare/trestle-diagram-v0.1.0...trestle-diagram-v0.1.1) - 2026-10-14

### Added

- hosted marimo editor for the Python client workspace (#131)
