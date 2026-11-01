#!/bin/sh
# Prints install.sh for the blessh packages in a directory: their version,
# the release tag and their SHA-256 sums filled in. An architecture
# without a package gets an empty sum, which the installer refuses.
#
#   packaging/linux/render-install.sh <dir with blessh_*.deb>
set -eu
dir=$1
deb() { ls "$dir"/blessh_*_"$1".deb 2> /dev/null | tail -n 1; }
sum() { f=$(deb "$1"); [ -z "$f" ] || sha256sum "$f" | cut -d' ' -f1; }
any=$(deb amd64)
[ -n "$any" ] || any=$(deb arm64)
[ -n "$any" ] || { echo "no blessh_*.deb in $dir" >&2; exit 1; }
version=$(basename "$any" | sed 's/^blessh_\(.*\)_[a-z0-9]*\.deb$/\1/')
sed -e "s/@VERSION@/$version/" -e "s/@TAG@/blessh-v${version%-*}/" \
    -e "s/@SHA256_AMD64@/$(sum amd64)/" -e "s/@SHA256_ARM64@/$(sum arm64)/" \
    "$(dirname "$0")/install.sh"
