# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
