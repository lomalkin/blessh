#!/bin/sh
# Builds the macOS release tarball into dist/: binaries, launchd jobs,
# install scripts. With BLESSH_SIGN_IDENTITY (a code signing identity in
# the keychain), the binaries are signed with it (sign.sh, also in the
# tarball to sign on another Mac before installing).
#
#   packaging/macos/build-tarball.sh
set -eu
cd "$(dirname "$0")/../.."
ver=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
arch=$(uname -m)
d=dist/blessh-$ver-macos-$arch
cargo build --release
rm -rf "$d" "$d.tar.gz"
mkdir -p "$d"
cp target/release/blessh target/release/blessh-server target/release/blessh-session "$d/"
[ -z "${BLESSH_SIGN_IDENTITY:-}" ] || packaging/macos/sign.sh "$BLESSH_SIGN_IDENTITY" "$d"
cp packaging/macos/install.sh packaging/macos/uninstall.sh packaging/macos/sign.sh packaging/macos/sshd_config \
    packaging/macos/org.blessh.server.plist packaging/macos/org.blessh.session.plist \
    README.md docs/protocol.md "$d/"
tar -C dist -czf "$d.tar.gz" "$(basename "$d")"
echo "$d.tar.gz"
