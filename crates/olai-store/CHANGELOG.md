# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.6](https://github.com/open-lakehouse/trestle/compare/olai-store-v0.0.5...olai-store-v0.0.6) - 2026-07-12

### Added

- [**breaking**] keyset pagination for list/search/query_edges (#91)
- AssociationStore pass-through for ManagedObjectStore (#90)
- [**breaking**] native Postgres backend (PgStore) alongside SQLite (#89)
- case-insensitive names + unified migrator for SqlStore consumers (#88)
- tracing instrumentation with OTel-shaped fields (#85)
- [**breaking**] TAO-style edge listing (query_edges, v7 recency, count) (#84)
- payload search/filter API (Phase 1) (#80)
- envelope encryption for sensitive fields; consolidate store API (#77)
- CAS updates, rename, transactions + InMemory & SQLite backends (#76)

### Documentation

- rustdoc improvements + docs.rs feature build/badges (#83)

### Performance

- SQLite filter pushdown for search (Phase 2) (#81)

## [0.0.5](https://github.com/open-lakehouse/trestle/compare/olai-store-v0.0.4...olai-store-v0.0.5) - 2026-06-30

### Changed

- self-contained module templates + object-store gateway routing (#57)

## [0.0.3](https://github.com/open-lakehouse/trestle/compare/olai-store-v0.0.2...olai-store-v0.0.3) - 2026-06-23

### Documentation

- document error sections, Label seam, and store traits
- tighten crate READMEs + refresh repo docs (#25)

## [0.0.2](https://github.com/open-lakehouse/trestle/compare/olai-store-v0.0.1...olai-store-v0.0.2) - 2026-06-23

### Added

- automate releases with release-plz; rename trestle crate to olai-testle (#16)

### Fixed

- allow setting resource IDs externally
