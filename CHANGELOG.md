# Changelog

## Unreleased

## v0.1.1 - 2026-08-06

### Fixed

- Serialized schema inspection and creation so concurrent first-time store opens
  cannot race and fail after both callers decide to initialize the database.

## v0.1.0 - 2026-08-05

### Added

- Initial project scaffold.
- M1 local authoring store and Rust API with append-only voice-pack revisions,
  linear draft versions, immutable bulk approval attestations, and strict typed
  user-content schemas.
- Fleet-style optional JSON CLI for pack import/export/selection and typed item
  management, explicit context and verb serving, draft versioning, and
  attestation recording.
- Separate, explicitly imported starter-pack data with tighten, proofread,
  shape, and draft; no built-in verbs or code-review voice material.
- M1 reference documentation and end-to-end integration coverage.
- Canonical verb families, instructions, context bindings and constraints;
  surface resolution; frozen draft pack revisions; and approval provenance,
  methods, exact item versions, and per-item exceptions.

### Changed

- Updated compatible Rust dependencies and the pinned Nix toolchain, fleet,
  and SourceHut release tooling.

### Fixed

- Made concurrent first-time store opens reliably negotiate SQLite WAL mode.
