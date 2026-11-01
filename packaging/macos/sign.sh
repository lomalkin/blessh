#!/bin/sh
# Signs the blessh program in a directory (an unpacked release tarball)
# with a code signing identity from the keychain. macOS keys the Bluetooth
# permission to the signature: with the same identity it survives updates,
# with an ad-hoc signature it is asked for again after each one.
#
#   packaging/macos/sign.sh <identity> [directory]
set -eu
[ $# -ge 1 ] || { sed -n '7p' "$0"; exit 2; }
ID=$1
DIR=${2:-$(dirname "$0")}
codesign --force -s "$ID" -i org.blessh "$DIR/blessh"
