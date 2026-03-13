#!/usr/bin/env bash
# build-runner-image.sh — build skg-runner OCI image end-to-end
#
# Steps:
#   1. cargo build --release -p skg-runner
#   2. nix build .#runner-image
#   3. (optional) docker load < result
#
# Requires: cargo, protoc, nix (with dockerTools — Linux only).
set -euo pipefail

echo "==> Building skg-runner (release)…"
cargo build --release -p skg-runner

echo "==> Building OCI image via Nix…"
nix build .#runner-image

echo "==> Image tarball at: ./result"
echo "    Load with: docker load < ./result"
