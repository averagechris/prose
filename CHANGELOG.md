# Changelog

## Unreleased

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
- M2 MCP stdio tools with full CLI capability parity through one shared typed
  application layer.
- OpenCode pointer-skill rendering that fetches live pack material instead of
  copying voice guidance into static adapters.
- M3 loopback HTTP service, backend-neutral command transport for assist,
  append-only submit-time captures, and a GitHub/Linear Manifest V3 extension.
