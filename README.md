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

## Development

```sh
direnv allow   # or: nix develop
nix run . -- --help
nix run .#ci-fmt
nix run .#ci-clippy
nix run .#static-checks
nix run .#ci-test
```

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
