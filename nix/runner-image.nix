# runner-image.nix — OCI image for skg-runner
#
# Produces a minimal OCI image containing the skg-runner static binary.
#
# Usage (from flake.nix):
#   packages.runner-image = pkgs.callPackage ./nix/runner-image.nix { };
#
# The derivation accepts an optional `binaryPath` attribute pointing to
# a pre-built skg-runner binary.  When omitted it falls back to building
# inside Nix via `cargo build --release -p skg-runner`, but that requires
# the full workspace source tree *and* protoc to be available, so the
# pre-built path is the recommended workflow for local dev.
#
# Build flow (macOS or Linux):
#   1. cargo build --release -p skg-runner          # native or cross-musl
#   2. nix build .#runner-image                      # Linux only (dockerTools)
#   3. docker load < result
#
# On macOS, dockerTools is unavailable — build the image in CI or a Linux VM.

{
  pkgs,
  lib,
  binaryPath ? null,
}:

let
  version = "0.4.0"; # keep in sync with runner/skg-runner/Cargo.toml

  # If no pre-built binary is supplied, build from source inside Nix.
  # This is best-effort: complex workspace + tonic-build makes pure Nix
  # builds fragile.  Prefer passing binaryPath from a cargo build step.
  binarySrc =
    if binaryPath != null then
      binaryPath
    else
      let
        src = lib.cleanSourceWith {
          src = ./..;
          filter =
            path: type:
            let
              relPath = lib.removePrefix (toString ./..) (toString path);
            in
            # Include Cargo manifests, Rust sources, proto files, build scripts
            (lib.hasSuffix ".toml" path)
            || (lib.hasSuffix ".rs" path)
            || (lib.hasSuffix ".proto" path)
            || (lib.hasSuffix ".lock" path)
            || (type == "directory");
        };
      in
      pkgs.rustPlatform.buildRustPackage {
        pname = "skg-runner";
        inherit version src;
        cargoLock.lockFile = ../Cargo.lock;
        nativeBuildInputs = [ pkgs.protobuf ];
        buildAndTestSubdir = "runner/skg-runner";
        doCheck = false; # tests run separately in CI
        meta.mainProgram = "skg-runner";
      };

  # Minimal /tmp for operators that need a writable scratch directory.
  tmpDir = pkgs.runCommand "tmp-dir" { } ''
    mkdir -p $out/tmp
  '';
in

pkgs.dockerTools.buildLayeredImage {
  name = "skg-runner";
  tag = version;

  contents = [
    binarySrc
    pkgs.cacert
    pkgs.tzdata
    pkgs.iana-etc
    tmpDir
  ];

  config = {
    Cmd = [ "/bin/skg-runner" ];

    ExposedPorts = {
      "50051/tcp" = { };
      "8080/tcp" = { };
    };

    Env = [
      "SSL_CERT_FILE=/etc/ssl/certs/ca-bundle.crt"
      "TZDIR=/share/zoneinfo"
    ];

    User = "65534";
  };
}
