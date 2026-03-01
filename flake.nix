{
  description = "neuron — composable building blocks for AI agents";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      imports = [
        inputs.treefmt-nix.flakeModule
      ];

      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];

      perSystem =
        {
          config,
          inputs',
          pkgs,
          lib,
          system,
          ...
        }:
        let
          rustToolchain = inputs'.fenix.packages.stable.withComponents [
            "cargo"
            "clippy"
            "rust-src"
            "rustc"
            "rustfmt"
            "rust-analyzer"
          ];
        in
        {
          _module.args.pkgs = import inputs.nixpkgs {
            inherit system;
            overlays = [ inputs.fenix.overlays.default ];
          };

          # ── treefmt ──────────────────────────────────────────────
          treefmt = {
            projectRootFile = "flake.nix";
            programs = {
              rustfmt = {
                enable = true;
                package = rustToolchain;
              };
              nixfmt.enable = true;
              taplo.enable = true;
            };
          };

          # ── devShell ─────────────────────────────────────────────
          devShells.default = pkgs.mkShell {

            buildInputs = [
              rustToolchain

              # Cargo extensions
              pkgs.cargo-watch
              pkgs.cargo-edit
              pkgs.cargo-deny
              pkgs.cargo-audit

              # reqwest (HTTP client used by all provider crates)
              pkgs.pkg-config
              pkgs.openssl

              # Doc & link tools
              pkgs.mdbook
              pkgs.lychee

              # Pre-commit hooks
              pkgs.prek

              # Nix tooling
              pkgs.nixd
            ]
            ++ lib.optionals pkgs.stdenv.isDarwin [
              pkgs.libiconv
            ]
            ++ lib.optionals pkgs.stdenv.isLinux [
              pkgs.cargo-tarpaulin
            ];

            OPENSSL_NO_VENDOR = 1;
            RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";

            shellHook = ''
              prek install --install-hooks 2>/dev/null

              echo "neuron dev shell"
              echo ""
              echo "  rustc --version   — $(rustc --version)"
              echo "  nix fmt           — format everything (rustfmt + nixfmt + taplo)"
              echo "  nix flake check   — verify formatting"
              echo "  cargo clippy      — lint all crates"
              echo "  cargo test        — run all tests"
              echo "  cargo deny check  — dependency review"
              echo "  cargo audit       — security audit"
              echo "  prek run -a       — run all pre-commit hooks"
              echo ""
            '';
          };
        };
    };
}
