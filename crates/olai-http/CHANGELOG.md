# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.4](https://github.com/open-lakehouse/trestle/compare/olai-http-v0.0.3...olai-http-v0.0.4) - 2026-06-28

### Added

- *(http)* ConnectRPC transport backed by CloudClient + retry fix (#49)

## [0.0.3](https://github.com/open-lakehouse/trestle/compare/olai-http-v0.0.2...olai-http-v0.0.3) - 2026-06-23

### Documentation

- document request builder, send flow, and credentials
- tighten crate READMEs + refresh repo docs (#25)

### Fixed

- redact secrets in Azure/GCP/Databricks credential Debug (#28)

## [0.0.2](https://github.com/open-lakehouse/trestle/compare/olai-http-v0.0.1...olai-http-v0.0.2) - 2026-06-23

### Added

- automate releases with release-plz; rename trestle crate to olai-testle (#16)

### Fixed

- correct service-SAS string-to-sign + Azurite emulator support (#15)
