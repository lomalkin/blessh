#!/bin/sh
# Prints DEB_VERSION (with Debian revision) for $GITHUB_ENV.
# Tag builds (blessh-vX.Y.Z) are releases: X.Y.Z-1; the tag must match the
# workspace version in Cargo.toml. Other builds are X.Y.Z~ciN-1, which
# sorts before the X.Y.Z release.
set -eu
ver=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
if [ "${GITHUB_REF_TYPE:-}" = tag ]; then
    if [ "$GITHUB_REF_NAME" != "blessh-v$ver" ]; then
        echo "tag $GITHUB_REF_NAME does not match the workspace version $ver" >&2
        exit 1
    fi
    echo "DEB_VERSION=$ver-1"
else
    echo "DEB_VERSION=$ver~ci${GITHUB_RUN_NUMBER:-0}-1"
fi
