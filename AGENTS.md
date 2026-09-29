# prose project guidance

Use `jj` for version-control actions in this repository.

## What prose is (read this first)

prose is a voice-preserving authoring service for agent-assisted content:
deterministic, model-free infrastructure serving voice packs, drafts, verbs,
and approval attestations to agent harnesses. Design docs index:
`docs/README.md`. Canonical design history: tracker tickets #246 (intention)
and #247 (architecture; milestone status lives in its comments).

Session essentials:

- **Broker, never brain.** Never add model calls or judgment to prose
  itself; judgment belongs to attached agent runtimes.
- **Mechanism vs material.** Voice packs, verbs, and approval tiers are
  user data; prose ships schemas and process only, including zero built-in
  verbs.
- **Sovereignty.** Agents never approve content on the user's behalf;
  attestations are recorded only when the human actually approved.

## Development

- Enter the toolchain with `direnv allow` or `nix develop`.
- Nix formatting uses wrapped `alejandra -q`; run `nix fmt` or `nix fmt -- --check .`.
- Prefer local checks: `nix run .#static-checks` (fmt + clippy), `nix run .#ci-test`, `nix run .#ci-machete`, `nix run .#ci-sort`, `nix run .#ci-deny`, `nix run .#ci-audit`.
- `.builds/ci.yml` runs fmt, clippy, test, the package build, closure-size reporting, and a guarded Cachix cache warm on every push.

## Release workflow

When Chris says "ship a new version", run exactly:

```sh
nix run .#release -- --version X.Y.Z --check
nix run .#release -- --version X.Y.Z
```

Run both from an empty `@` whose parent, local `main`, and `main@origin` agree.
The SHA-pinned Fleet GitHub backend prepares and validates the release tree,
then atomically publishes leased `main` and its annotated tag. The read-only
workflow builds macOS arm64 and Linux x86_64 artifacts. Follow
`docs/release.md` to verify its six files and manually publish four GitHub
Release assets. No app or workflow publishes assets automatically. Prose is
not registered with the Fleet website and has no Pages project route.

`builds/release-linux-x86_64.yml` and existing SourceHut tags and assets are
archival. Never submit that manifest for a new release, dual-publish a future
tag, or create a tag merely to test release automation.

## Issue tracking

Project work belongs in the umbrella SourceHut tracker:
https://todo.sr.ht/~averagechris/projects

Use the `repo:prose` label for this repository. The label follows the fleet
convention `repo:<fleet-name>` and is intentionally separate from artifact,
binary, or SourceHut repository aliases.

<!-- Last audited: 2026-09-29 | moved future releases to manual GitHub publication -->
