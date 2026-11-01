# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.10](https://github.com/open-lakehouse/trestle/compare/olai-http-v0.0.9...olai-http-v0.0.10) - 2026-11-01

### Changed

- share JWT/expiry helpers and drop narration (#250)

### Fixed

- error on unparsable GCP impersonation expiry (#251)

## [0.0.9](https://github.com/open-lakehouse/trestle/compare/olai-http-v0.0.8...olai-http-v0.0.9) - 2026-10-30

### Added

- GCP token calls with wall-clock expiries (#228)
- AssumeRole options and reusable user delegation keys (#207)
- build stores from vended credentials (#201)
- downscoping policies with RustFS emulator tests (#193)

### Fixed

- harden credential vending crates after review (#225)
- correct user delegation and directory SAS signatures (#197)

## [0.0.8](https://github.com/open-lakehouse/trestle/compare/olai-http-v0.0.7...olai-http-v0.0.8) - 2026-10-27

### Added

- GCP service-account ID tokens and user-assigned MSI ids (#177)
- bearer-token auth for any API (#175)

## [0.0.7](https://github.com/open-lakehouse/trestle/compare/olai-http-v0.0.6...olai-http-v0.0.7) - 2026-10-26

### Added

- azure-client-secret and azure-cli Databricks auth (#174)
- [**breaking**] reqwest 0.13 and CloudClient::new_databricks_from_builder (#171)

## [0.0.6](https://github.com/open-lakehouse/trestle/compare/olai-http-v0.0.5...olai-http-v0.0.6) - 2026-07-14

### Added

- add send_raw to preserve non-2xx status and body (#101)

## [0.0.5](https://github.com/open-lakehouse/trestle/compare/olai-http-v0.0.4...olai-http-v0.0.5) - 2026-06-30

### Changed

- self-contained module templates + object-store gateway routing (#57)

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
