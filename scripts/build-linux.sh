#!/bin/sh
# Build rsqldev for Linux in a container and package dist/rsqldev-linux-<arch>.tar.gz.
#   scripts/build-linux.sh [x86_64|aarch64]   (default: Docker host architecture)
# Needs Docker; a foreign architecture runs under QEMU:
#   docker run --privileged --rm tonistiigi/binfmt --install all
# Ubuntu 22.04 base (glibc 2.35): runs on Ubuntu 22.04+, Debian 12+, Fedora 36+.
set -e
cd "$(dirname "$0")/.."
ARCH=${1:-$(docker version -f '{{.Server.Arch}}' | sed 's/amd64/x86_64/; s/arm64/aarch64/')}
case $ARCH in
  x86_64) P=linux/amd64; IMG=ubuntu:22.04 ;;
  aarch64) P=linux/arm64; IMG=ubuntu:22.04 ;;
  *) echo "unknown architecture: $ARCH (x86_64, aarch64)"; exit 1 ;;
esac
docker run --rm --platform "$P" -v "$PWD":/src -w /src -e ARCH="$ARCH" -e DEBIAN_FRONTEND=noninteractive \
  -e CARGO_TARGET_DIR="/src/target/linux-$ARCH" "$IMG" sh -ec '
  apt-get update -qq
  apt-get install -y -qq --no-install-recommends build-essential cmake pkg-config curl ca-certificates git \
    libgtk-3-dev libssl-dev libclang-dev >/dev/null
  curl -sSf https://sh.rustup.rs | sh -s -- -y -q --profile minimal
  . "$HOME/.cargo/env"
  cargo build --release -p rsqldev
  scripts/linux/package.sh "$ARCH" "$CARGO_TARGET_DIR"
'
