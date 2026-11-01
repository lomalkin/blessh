#!/bin/sh
# Writes a formula for a macOS release tarball to stdout:
#
#   packaging/homebrew/render.sh <formula> <tarball> [url]
#
# <formula> is a template here without .rb.in (blessh). The URL
# defaults to the tarball's asset in the GitHub release of the workspace
# version (tag blessh-vX.Y.Z). The version is set explicitly: Homebrew
# might read a number from the file name.
set -eu
formula=$1 tarball=$2
dir=$(dirname "$0")
ver=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$dir/../../Cargo.toml" | head -n 1)
url=${3:-https://github.com/lomalkin/blessh/releases/download/blessh-v$ver/$(basename "$tarball")}
sha=$(shasum -a 256 "$tarball" | cut -d' ' -f1)
sed -e "s|@URL@|$url|" -e "s|@VERSION@|$ver|" -e "s|@SHA256@|$sha|" "$dir/$formula.rb.in"
