{
  description = "Voice-preserving authoring service for agent-assisted content: deterministic voice packs, drafts, verbs, and approval attestations.";

  nixConfig = {
    extra-substituters = ["https://averagechris-dotfiles.cachix.org"];
    extra-trusted-public-keys = ["averagechris-dotfiles.cachix.org-1:VwJkl5dG1+xGDY5x884mH/kVwwpgwBAdBKIF3BZiia4="];
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    fleet.url = "git+https://git.sr.ht/~averagechris/averagechris.srht.site";
    srht.url = "git+https://git.sr.ht/~averagechris/srht";
  };

  outputs = {
    self,
    nixpkgs,
    fleet,
    srht,
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
    ciNixFmt = system:
      mkToolApp system "ci-nix-fmt" [(pkgsFor system).alejandra] ''
        alejandra -q --check .
      '';
    fleetCiTool = system: name: let
      pkgs = pkgsFor system;
      program = (fleetApps system).apps.${name}.program;
    in
      pkgs.writeShellScriptBin name ''
        exec ${program} "$@"
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
      mkExtension = browser:
        pkgs.runCommand "prose-extension-${browser}-${package.version}" {nativeBuildInputs = [pkgs.nodejs pkgs.zip];} ''
          mkdir -p "$out/unpacked"
          node ${./extension/scripts/build.mjs} ${browser} "$out/unpacked" ${./.}
          (cd "$out/unpacked" && zip -qr "$out/prose-extension-${browser}.zip" .)
        '';
      extensionChrome = mkExtension "chrome";
      extensionFirefox = mkExtension "firefox";
      browserTests = pkgs.runCommand "prose-browser-tests-${package.version}" {nativeBuildInputs = [pkgs.nodejs];} ''
        cp -R ${./.} source
        chmod -R u+w source
        cd source
        node --test extension/tests/*.test.js
        node --check extension/background.js
        node --check extension/queue.js
        node --check extension/content-core.js
        node --check extension/content.js
        node --check extension/scripts/build.mjs
        touch "$out"
      '';
    in {
      default = app;
      prose = app;
      prose-extension = extensionChrome;
      prose-extension-chrome = extensionChrome;
      prose-extension-firefox = extensionFirefox;
      prose-browser-tests = browserTests;
      ci-audit = ciAudit system;
      ci-clippy = fleetCiTool system "ci-clippy";
      ci-deny = ciDeny system;
      ci-fmt = fleetCiTool system "ci-fmt";
      ci-machete = ciMachete system;
      ci-nix-fmt = ciNixFmt system;
      ci-sort = ciSort system;
      ci-test = fleetCiTool system "ci-test";
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
      inherit ((fleetApps system).apps) prepare-release release-tag release ci-fmt ci-clippy static-checks ci-test;
    });

    checks = forAllSystems (system: {
      inherit (self.packages.${system}) prose prose-extension-chrome prose-extension-firefox prose-browser-tests release-artifact;
    });

    devShells = forAllSystems (system: let
      pkgs = pkgsFor system;
    in {
      default = pkgs.mkShell {
        packages = with pkgs;
          [
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
            nodejs
            rust-analyzer
            rustc
            rustfmt
            sccache
          ]
          ++ [
            self.packages.${system}.ci-audit
            self.packages.${system}.ci-clippy
            self.packages.${system}.ci-deny
            self.packages.${system}.ci-fmt
            self.packages.${system}.ci-machete
            self.packages.${system}.ci-nix-fmt
            self.packages.${system}.ci-sort
            self.packages.${system}.ci-test
            srht.packages.${system}.srht
          ];
      };
    });

    formatter = forAllSystems nixFormatter;
  };
}
