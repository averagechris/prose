# prose project guidance

Use `jj` for version-control actions in this repository.

## Development

- Enter the toolchain with `direnv allow` or `nix develop`.
- Nix formatting uses wrapped `alejandra -q`; run `nix fmt` or `nix fmt -- --check .`.
- Prefer local checks: `nix run .#static-checks` (fmt + clippy), `nix run .#ci-test`, `nix run .#ci-machete`, `nix run .#ci-sort`, `nix run .#ci-deny`, `nix run .#ci-audit`.
- `.builds/ci.yml` runs fmt, clippy, test, the package build, closure-size reporting, and a guarded Cachix cache warm on every push.

## Release workflow

This repo uses the standard averagechris fleet interface:

```sh
nix run .#prepare-release -- --version X.Y.Z
nix run .#release-tag
nix build .#release-artifact
nix run .#static-checks
nix run .#release -- --version X.Y.Z --submit-linux-build
```

`.builds/ci.yml` runs automatically on every push and warms the
averagechris-dotfiles Cachix cache when the CI secret is available.
`builds/release-linux-x86_64.yml` is explicit-submit only; do not move it to `.builds/`.

## Issue tracking

Project work belongs in the umbrella SourceHut tracker:
https://todo.sr.ht/~averagechris/projects

Use the `repo:prose` label for this repository. The label follows the fleet
convention `repo:<fleet-name>` and is intentionally separate from artifact,
binary, or SourceHut repository aliases.
