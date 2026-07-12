# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.6](https://github.com/open-lakehouse/trestle/compare/olai-codegen-v0.0.5...olai-codegen-v0.0.6) - 2026-07-12

### Added

- case-insensitive names + unified migrator for SqlStore consumers (#88)

## [0.0.5](https://github.com/open-lakehouse/trestle/compare/olai-codegen-v0.0.4...olai-codegen-v0.0.5) - 2026-06-30

### Added

- generate ergonomic ConnectRPC clients in olai-codegen (#52)

### Changed

- self-contained module templates + object-store gateway routing (#57)

### Fixed

- surface builder TokenStream parse errors (#53)

## [0.0.4](https://github.com/open-lakehouse/trestle/compare/olai-codegen-v0.0.3...olai-codegen-v0.0.4) - 2026-06-28

### Added

- real #[pyclass] wrapper models for Python (#43)
- support buffa + Python via generated PyO3 model conversions (#42)
- injectable auth-header hook for the WASM client (with_auth / with_credentials) (#41)
- browser WASM client — the frontend consumes the generated client (#37)
- golden-path example — REST + buf ConnectRPC on one port, with the generator/template fixes to get there (#29)

### Changed

- structured config + co-located codegen path reconciliation (#45)

### Documentation

- scaffold testle skill (#31)

### Fixed

- correct PyO3 prost boxing + buffa-runtime lint cleanups (#44)
- handle proto package versions (#39)
- *(codegen)* allow empty docs in generated tonic code (#36)
- eliminate clippy warnings in generated code (#35)
- annotate example Catalog.name as IDENTIFIER (#33)

## [0.0.3](https://github.com/open-lakehouse/trestle/compare/olai-codegen-v0.0.2...olai-codegen-v0.0.3) - 2026-06-23

### Documentation

- add generator module docs and entry-point errors
- tighten crate READMEs + refresh repo docs (#25)

## [0.0.2](https://github.com/open-lakehouse/trestle/compare/olai-codegen-v0.0.1...olai-codegen-v0.0.2) - 2026-06-23

### Added

- automate releases with release-plz; rename trestle crate to olai-testle (#16)
- WASM/browser client — transport seam, olai-http-wasm, and wasm-bindgen JS bindings (#14)
- add opt-in buffa runtime backend alongside prost (#13)
- better generated clients/bindings + emitter-layer cleanup (#12)
- honor name_field and fix plural handler/model naming (#11)
- scaffold project templating (#8)

### Fixed

- correctness and quality fixes for olai-codegen output (#10)
