# prose

Voice-preserving authoring service for agent-assisted content: deterministic voice packs, drafts, verbs, and approval attestations.

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
