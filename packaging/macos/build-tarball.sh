#!/bin/sh
# Builds the macOS release tarball into dist/: the blessh program as a
# universal binary (arm64 and x86_64), the signing script and the README.
# The Homebrew formula installs from it (packaging/homebrew); for a manual
# test, unpack it and run ./blessh. With BLESSH_SIGN_IDENTITY (a code
# signing identity in the keychain), the binary is signed with it
# (sign.sh): macOS then keeps the Bluetooth permission across updates.
# BLESSH_TARGETS limits the architectures (default: both), e.g. for a quick
# local build: BLESSH_TARGETS=aarch64-apple-darwin.
#
#   packaging/macos/build-tarball.sh
set -eu
cd "$(dirname "$0")/../.."
ver=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
d=dist/blessh-$ver-macos
targets=${BLESSH_TARGETS:-aarch64-apple-darwin x86_64-apple-darwin}
bins=
for t in $targets; do
    cargo build --release --target "$t" -p blessh-client
    bins="$bins target/$t/release/blessh"
done
rm -rf "$d" "$d.tar.gz"
mkdir -p "$d"
# shellcheck disable=SC2086
lipo -create -output "$d/blessh" $bins
# lipo invalidates the linker's ad-hoc signatures; sign the result again.
codesign --force -s - -i org.blessh "$d/blessh"
[ -z "${BLESSH_SIGN_IDENTITY:-}" ] || packaging/macos/sign.sh "$BLESSH_SIGN_IDENTITY" "$d"
cp packaging/macos/sign.sh README.md "$d/"
tar -C dist -czf "$d.tar.gz" "$(basename "$d")"
echo "$d.tar.gz"
