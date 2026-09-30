{
  description = "Voice-preserving authoring service for agent-assisted content: deterministic voice packs, drafts, verbs, and approval attestations.";

  nixConfig = {
    extra-substituters = ["https://averagechris-dotfiles.cachix.org"];
    extra-trusted-public-keys = ["averagechris-dotfiles.cachix.org-1:VwJkl5dG1+xGDY5x884mH/kVwwpgwBAdBKIF3BZiia4="];
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    fleet.url = "github:averagechris/fleet/ab828532afb4cd8fcf2835051d21b4c55b65609d";
  };

  outputs = {
    self,
    nixpkgs,
    fleet,
  }: let
    systems = [
      "aarch64-darwin"
      "aarch64-linux"
      "x86_64-darwin"
      "x86_64-linux"
    ];

    forAllSystems = nixpkgs.lib.genAttrs systems;
    pkgsFor = system: import nixpkgs {inherit system;};
    cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
    package = cargoToml.package;
    fleetApps = system:
      fleet.lib.fleet.presets.rust {
        pkgs = pkgsFor system;
        inherit self;
        pname = "prose";
        binaries = ["prose"];
        subdir = "prose";
        srhtRepo = "prose";
        versionMode = "package";
        versionFile = "Cargo.toml";
        lockPackages = ["prose"];
        releaseBackend = "github";
        releaseValidationApps = ["release-contract"];
      };
    mkToolApp = system: name: runtimeInputs: text: let
      pkgs = pkgsFor system;
    in
      pkgs.writeShellApplication {
        inherit name runtimeInputs text;
      };
    ciAudit = system:
      mkToolApp system "ci-audit" [(pkgsFor system).cargo (pkgsFor system).cargo-audit] ''
        cargo audit --deny warnings
      '';
    ciDeny = system:
      mkToolApp system "ci-deny" [(pkgsFor system).cargo (pkgsFor system).cargo-deny] ''
        cargo deny check
      '';
    ciMachete = system:
      mkToolApp system "ci-machete" [(pkgsFor system).cargo (pkgsFor system).cargo-machete] ''
        cargo machete
      '';
    ciSort = system:
      mkToolApp system "ci-sort" [(pkgsFor system).cargo (pkgsFor system).cargo-sort] ''
        cargo sort --workspace --check
      '';
    nixFormatter = system: let
      pkgs = pkgsFor system;
    in
      pkgs.writeShellApplication {
        name = "alejandra";
        runtimeInputs = [pkgs.alejandra];
        text = ''
          if [[ $# -eq 0 ]]; then
            exec alejandra -q .
          fi

          exec alejandra -q "$@"
        '';
      };
    releaseContract = system: let
      pkgs = pkgsFor system;
    in
      pkgs.writeShellApplication {
        name = "release-contract";
        runtimeInputs = [pkgs.gnugrep];
        text = ''
          help="$(${(fleetApps system).apps.release.program} --help)"
          grep -Fq -- 'release --version X.Y.Z [--check] [--allow-downgrade]' <<<"$help"
          grep -Fq -- '--check  nonmutating ref/version preflight only; does not run validation or build artifacts' <<<"$help"
          grep -Fq 'nix run .#release -- --version X.Y.Z --check' README.md
          grep -Fq 'nix run .#release -- --version X.Y.Z' README.md
          if grep -Eq -- '--(submit-linux-build|skip-(validate|tag|artifact|pages)|publish-pages)' <<<"$help" README.md AGENTS.md docs/release.md; then
            printf 'release help or documentation exposes an obsolete release flag\n' >&2
            exit 1
          fi
          grep -Fq 'No website or Pages integration is configured for prose.' docs/release.md
        '';
      };
  in {
    packages = forAllSystems (system: let
      pkgs = pkgsFor system;
      lib = pkgs.lib;
      app = pkgs.rustPlatform.buildRustPackage {
        pname = "prose";
        version = package.version;
        src = lib.cleanSource ./.;
        cargoLock.lockFile = ./Cargo.lock;

        meta = {
          description = package.description;
          license = with lib.licenses; [mit asl20];
          mainProgram = "prose";
        };
      };
    in {
      default = app;
      prose = app;
      ci-audit = ciAudit system;
      ci-deny = ciDeny system;
      ci-machete = ciMachete system;
      ci-sort = ciSort system;
      release-artifact = (fleetApps system).releaseArtifact system;
    });

    apps = forAllSystems (system: {
      default = self.apps.${system}.prose;
      prose = {
        type = "app";
        program = "${self.packages.${system}.prose}/bin/prose";
      };
      ci-audit = {
        type = "app";
        program = "${self.packages.${system}.ci-audit}/bin/ci-audit";
      };
      ci-deny = {
        type = "app";
        program = "${self.packages.${system}.ci-deny}/bin/ci-deny";
      };
      ci-machete = {
        type = "app";
        program = "${self.packages.${system}.ci-machete}/bin/ci-machete";
      };
      ci-sort = {
        type = "app";
        program = "${self.packages.${system}.ci-sort}/bin/ci-sort";
      };
      release-contract = {
        type = "app";
        program = "${releaseContract system}/bin/release-contract";
      };
      inherit ((fleetApps system).apps) prepare-release release-tag release ci-fmt ci-clippy static-checks ci-test;
    });

    checks = forAllSystems (system: {
      inherit (self.packages.${system}) prose release-artifact;
      release-contract = releaseContract system;
    });

    devShells = forAllSystems (system: let
      pkgs = pkgsFor system;
    in {
      default = pkgs.mkShell {
        packages = with pkgs; [
          alejandra
          cargo
          cargo-audit
          cargo-deny
          cargo-machete
          cargo-outdated
          cargo-sort
          clippy
          direnv
          jujutsu
          nixd
          rust-analyzer
          rustc
          rustfmt
          sccache
        ];
      };
    });

    formatter = forAllSystems nixFormatter;
  };
}
