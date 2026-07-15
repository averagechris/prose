# prose

Voice-preserving authoring service for agent-assisted content: deterministic voice packs, drafts, verbs, and approval attestations.

## Vision

Everything published in your name should be something you actually said, in
substance and in sound, no matter how much help you had getting there.

prose is the machinery for that: user-owned voice packs, editing verbs, draft
provenance, and approval attestations, served to whatever agent harness you
work in. prose is deterministic infrastructure. The intelligence stays in
your agents, and the final word stays with you.

Design docs live in [docs/](docs/README.md); design history lives in the
umbrella tracker: [#246 (intention and
non-negotiables)](https://todo.sr.ht/~averagechris/projects/246) and
[#247 (architecture and milestones)](https://todo.sr.ht/~averagechris/projects/247).

## M1 local authoring store

M1 is a production Rust library and CLI for local, model-free authoring
state. It stores user-authored packs, explicit context and verb material,
linear draft histories, and immutable human-approval attestations. The binary
ships schemas and process only: there are no built-in packs, contexts, verbs,
tiers, surface mappings, models, or review judgment. A separate, explicitly
imported starter-pack document supplies four editing operations; it does not
contain the human-authored code-review voice module.

The store defaults to `$XDG_DATA_HOME/prose/prose.db` (falling back to
`$HOME/.local/share/prose/prose.db`). Use `--store PATH` or `PROSE_STORE` to
isolate automation and tests. See [the M1 reference](docs/m1.md) for the schema,
strict JSON input formats, CLI examples, and output protocol.

```sh
# Pack material may come from a file or stdin. Nothing is auto-installed.
prose --json pack import --file packs/starter.json --activate
prose --json pack create --file personal-pack.json
prose --json pack use personal
prose --json context code-review
prose --json verb tighten
prose --json surface github-pr

# --json gives the fleet one-document stdout/stderr protocol.
prose --json draft create --file draft.json
prose --json draft append my-draft --parent 1 --content-file revised.txt
prose --json attest --file approvals.json
```

## Development

```sh
direnv allow   # or: nix develop
nix run . -- --help
jj lint        # inside the dev shell; otherwise: nix develop . -c -- jj lint
nix run .#ci-fmt
nix run .#ci-clippy
nix run .#static-checks
nix run .#ci-test
```

Without Nix, the local Rust checks are `cargo fmt -- --check`, `cargo check`,
and `cargo test`.

## MCP and host adapters

`prose mcp` exposes the same pack, context, verb, surface, draft, attestation,
capture, and rendering operations as the CLI over MCP stdio. Both transports call one
typed application layer; neither duplicates storage or approval behavior.

`prose render --host opencode` emits a small pointer that tells the host to
fetch live material through prose. It does not copy personal voice content into
the repository or host adapter. See [the M2 reference](docs/m2.md).

## Browser service

`prose serve` exposes the same application operations on loopback for the
unpacked Chrome/Firefox extension in `extension/`. Assist uses an optional
attached-agent command; submit-time capture remains available when that backend
is absent. `nix build .#prose-extension` produces both an unpacked directory and
a zip archive. See [the M3 reference](docs/m3.md).

## Release

```sh
nix run .#release -- --version X.Y.Z --submit-linux-build
```

The shared release interface comes from
`git+https://git.sr.ht/~averagechris/averagechris.srht.site#lib.fleet.presets.rust`.

## Issues

Track project work in the umbrella tracker at <https://todo.sr.ht/~averagechris/projects> with
the `repo:prose` label. The `new-project` bootstrap ensures that label exists
idempotently when srht credentials are available.
