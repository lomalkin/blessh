#!/bin/sh
# Signs the blessh binaries in a directory (an unpacked release tarball)
# with a code signing identity from the keychain. macOS keys the Bluetooth
# permission to the signature: with the same identity it survives updates,
# with the linker's ad-hoc signature it is asked for again after each one.
#
#   packaging/macos/sign.sh <identity> [directory]
set -eu
[ $# -ge 1 ] || { sed -n '6p' "$0"; exit 2; }
ID=$1
DIR=${2:-$(dirname "$0")}
codesign --force -s "$ID" -i org.blessh.cli "$DIR/blessh"
codesign --force -s "$ID" -i org.blessh.server "$DIR/blessh-server"
codesign --force -s "$ID" -i org.blessh.session "$DIR/blessh-session"
