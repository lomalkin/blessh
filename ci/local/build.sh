#!/bin/sh
# Builds the release packages without GitHub Actions, with the same checks
# as .github/workflows/ci.yml (fmt, clippy, tests, loopback e2e, armhf tests
# under qemu as ARMv6):
#   ci/local/build.sh <git ref> [deb version]
# The deb version defaults to X.Y.Z-1 for a tag vX.Y.Z. Needs git and docker.
# Work tree and cargo cache: $BUILD_DIR (default ~/.cache/ble-term-build).
# Output: $BUILD_DIR/out/<deb version>/ with the packages, SHA256SUMS and
# notes.md, the release notes the CI release job would write.
set -eu
REF=$1
repo=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
BUILD_DIR=${BUILD_DIR:-$HOME/.cache/ble-term-build}
mkdir -p "$BUILD_DIR/cargo-registry"
BUILD_DIR=$(cd "$BUILD_DIR" && pwd)
src=$BUILD_DIR/src

docker build -q -t ble-term-build "$repo/ci/local" >/dev/null
# A run that failed inside the container leaves files owned by root.
docker run --rm -v "$BUILD_DIR:/w" ble-term-build rm -rf /w/src
git clone -q "$repo" "$src"
git -C "$src" checkout -q "$REF"
VER=${2:-$(cd "$src" && GITHUB_REF_TYPE=tag GITHUB_REF_NAME=$REF sh ci/deb-version.sh | cut -d= -f2)}
OUT=$BUILD_DIR/out/$VER
rm -rf "$OUT"
mkdir -p "$OUT"

docker run --rm -e HOME=/root -v "$src:/src" -v "$BUILD_DIR/cargo-registry:/opt/cargo/registry" -w /src ble-term-build sh -euc "
  trap 'chown -R $(id -u):$(id -g) /src /opt/cargo/registry' EXIT
  cargo fmt --check
  cargo clippy --all-targets -- -D warnings
  cargo test
  cargo build
  mkdir -p /run/sshd
  tests/e2e-loopback.sh target/debug/ble-term-loopback
  export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
  for t in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
    cargo deb --target \$t --deb-version $VER
  done

  # armhf for ARMv6 (Pi Zero, Pi 1): Raspberry Pi OS glibc as the qemu sysroot.
  T=arm-unknown-linux-gnueabihf
  base=http://raspbian.raspberrypi.com/raspbian
  mkdir -p /tmp/rb/sysroot
  cd /tmp/rb
  curl -sSf \$base/dists/bookworm/main/binary-armhf/Packages.xz | xz -d > Packages
  for pkg in libc6 libgcc-s1; do
    deb=\$(awk -v pkg=\$pkg '\$0 == \"Package: \" pkg {p=1} p && /^Filename:/ {print \$2; exit}' Packages)
    curl -sSfLo \$pkg.deb \$base/\$deb
    dpkg-deb -x \$pkg.deb sysroot
  done
  cd /src
  export QEMU_LD_PREFIX=/tmp/rb/sysroot QEMU_CPU=arm1176
  tests=\$(cargo zigbuild --lib --tests --target \$T.2.31 --message-format=json | jq -r 'select(.profile.test == true) | .executable // empty')
  for x in \$tests; do qemu-arm \$x; done
  cargo zigbuild --release --target \$T.2.31 --bins
  for b in ble-term-daemon ble-term-session ble-term-connect; do
    readelf -A target/\$T/release/\$b | grep -q 'Tag_CPU_arch: v6\$'
    qemu-arm target/\$T/release/\$b --version
  done
  cargo deb --target \$T --no-build --deb-version $VER
"

cp "$src"/target/*/debian/*.deb "$OUT"/
(cd "$OUT" && sha256sum *.deb > SHA256SUMS && cat SHA256SUMS)

# Release notes as the CI release job writes them.
prev=$(git -C "$src" describe --tags --abbrev=0 "$REF^" 2>/dev/null || true)
{
    if [ -f "$src/docs/releases/$REF.md" ]; then
        cat "$src/docs/releases/$REF.md"
        printf '\n**Commits**\n\n'
    fi
    git -C "$src" log --no-merges --format='- %s' ${prev:+"$prev.."}"$REF"
} > "$OUT/notes.md"
echo "built $OUT"
