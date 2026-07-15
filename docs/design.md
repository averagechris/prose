# prose design

## North star

Everything published in the user's name should be something they actually
said, in substance and in sound, regardless of how much help they had
getting there.

Deeper purpose: reconnect the user's thinking to human audiences. Agents
handle the labor between brain dump and publication; the human keeps the
voice and the final word. Chris is the first user, not the only user.

Canonical design history lives in the umbrella tracker:

- [#246](https://todo.sr.ht/~averagechris/projects/246): intention,
  non-negotiables, audience-keyed approval tiers, first slice.
- [#247](https://todo.sr.ht/~averagechris/projects/247): solution
  architecture, milestones, and spike items. Milestone status is tracked
  as comments on this ticket.

## Invariants

These must survive every change:

- **Broker, never brain.** prose is model-free by architecture,
  permanently. Judgment executes in attached agent runtimes: harnesses pull
  material (context modules, verb specs, tiers) from prose, or prose serve
  pushes assembled requests to a configured agent backend. prose
  contributes deterministic assembly, storage, transport, and recording
  only.
- **Mechanism vs material.** prose owns process and schemas; voice packs,
  context modules, verbs, surface mappings, and approval tiers are user
  data. prose ships zero verbs; the starter pack ships
  tighten/proofread/shape/draft.
- **Sovereignty.** Nothing wearing the user's name ships to a human
  audience without a recorded approval attestation. Publishing paths that
  go through prose enforce this at the exit. In harness-mediated flows the
  guarantee is auditable rather than enforced: the harness reports the
  human's approval and prose records it.
- **Cheaper to use than to bypass**, or the tool is dead. Frictionless as
  the default path is an existential requirement, not a UX nicety. Capture
  must never depend on the brain being available.

## v1 shape

- One binary, three faces: `prose <verb>` (CLI), `prose mcp` (MCP for
  harnesses), `prose serve` (HTTP for the browser extension and later
  surfaces).
- No daemon in v1. WAL SQLite at `~/.local/share/prose/` shared by local
  processes. A supervised `prose serve` arrives with the extension (M3).
- Append-only event-ish schema: drafts as version chains, attestations and
  flywheel evidence as immutable events. Flywheel data (what agents drafted
  vs what the human approved) is recorded from day one and read by nothing
  yet.
- Posting tools (gander, srht, linear-cli, ...) stay ignorant of prose;
  harnesses compose the loop: pre-fill, refine through prose, human
  approves, attest, post.
- Adapters render the hook, not the guide: per-host output is a small
  pointer ("route human-facing content through prose; report approvals as
  attestations"). Substance is served interactively so it cannot drift.

## Milestones

See #247 for full definitions and status comments.

- **M0 bootstrap:** done.
- **M1:** store schema, pack CRUD, `prose context code-review`, verb spec
  serving, draft version chains, attestation recording. Human-only task:
  Chris hand-writes v1 of the code-review voice module.
- **M2:** `prose mcp`, pointer-skill rendering, daily dogfood on the code
  review flow.
- **M3:** supervised `prose serve`, browser extension (assist plus
  submit-time capture on GitHub and Linear), broker transport spike.
- **M4:** native pack-item provenance, honest capture observations, hardened
  service/backend lifecycle, conservative browser adapters, and browser-specific
  packaging.
- **M5:** deterministic evidence reader and bounded audit export; no judgment.
- **M6:** messaging-harness and pack-lifecycle contracts, including onboarding
  users through empty private stores without built-in material.
- **M7:** separately configured authenticated remote serve and exact-version
  phone approval client, after local operational acceptance and threat review.
- **M8:** privacy-bounded historical refinement that emits candidate drafts and
  installs only explicitly approved versions.

The detailed dependency order, boundaries, and acceptance criteria live in
[post-m4.md](post-m4.md). These are refined plans, not authorization for future
schema migrations or immediate implementation.
