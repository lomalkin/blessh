#!/bin/sh
# Builds the release packages without GitHub Actions, with the same checks
# as .github/workflows/blessh.yml (fmt, clippy, tests, loopback e2e):
#   ci/local/build.sh <git ref> [deb version]
# The deb version defaults to X.Y.Z-1 for a tag blessh-vX.Y.Z. Needs git and docker.
# Work tree and cargo cache: $BUILD_DIR (default ~/.cache/blessh-build).
# Output: $BUILD_DIR/out/<deb version>/ with the packages, SHA256SUMS and
# notes.md, the release notes the CI release job would write.
set -eu
REF=$1
repo=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
BUILD_DIR=${BUILD_DIR:-$HOME/.cache/blessh-build}
mkdir -p "$BUILD_DIR/cargo-registry"
BUILD_DIR=$(cd "$BUILD_DIR" && pwd)
src=$BUILD_DIR/src

docker build -q -t blessh-build "$repo/ci/local" >/dev/null
# A run that failed inside the container leaves files owned by root.
docker run --rm -v "$BUILD_DIR:/w" blessh-build rm -rf /w/src
git clone -q "$repo" "$src"
git -C "$src" checkout -q "$REF"
VER=${2:-$(cd "$src" && GITHUB_REF_TYPE=tag GITHUB_REF_NAME=$REF sh ci/deb-version.sh | cut -d= -f2)}
OUT=$BUILD_DIR/out/$VER
rm -rf "$OUT"
mkdir -p "$OUT"

docker run --rm -e HOME=/root -v "$src:/src" -v "$BUILD_DIR/cargo-registry:/opt/cargo/registry" -w /src blessh-build sh -euc "
  trap 'chown -R $(id -u):$(id -g) /src /opt/cargo/registry' EXIT
  cargo fmt --check
  cargo clippy --all-targets -- -D warnings
  cargo test
  cargo build
  mkdir -p /run/sshd
  tests/e2e-loopback.sh
  export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
  for t in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
    cargo deb -p blessh-client --target \$t --deb-version $VER
  done
"

cp "$src"/target/*/debian/*.deb "$OUT"/
"$src/packaging/linux/render-install.sh" "$OUT" > "$OUT/install.sh"
(cd "$OUT" && sha256sum *.deb install.sh > SHA256SUMS && cat SHA256SUMS)

# Release notes as the CI release job writes them.
prev=$(git -C "$src" describe --tags --match 'blessh-v*' --abbrev=0 "$REF^" 2>/dev/null || true)
{
    [ ! -f "$src/docs/releases/$REF.md" ] || cat "$src/docs/releases/$REF.md"
    if [ -n "$prev" ]; then
        printf '\n**Commits**\n\n'
        git -C "$src" log --no-merges --format='- %s' "$prev..$REF"
    fi
} > "$OUT/notes.md"
echo "built $OUT"
