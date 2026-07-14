# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
