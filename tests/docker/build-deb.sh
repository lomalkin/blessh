#!/bin/sh
# Builds the Debian packages (blessh-server, blessh-client; amd64 and
# arm64) in a Debian container, as the CI job does, into dist/.
#
#   tests/docker/build-deb.sh [deb-version]
#
# The version defaults to the workspace version with a ~local suffix, which
# sorts before the release. Cargo's registry and the build directory live
# in Docker volumes, so later builds are incremental.
set -eu
cd "$(dirname "$0")/../.."
VER=${1:-$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)~local-1}
DOCKER=${DOCKER:-docker}
$DOCKER build -q -t blessh-deb-builder -f tests/docker/Dockerfile.deb tests/docker > /dev/null
mkdir -p dist
$DOCKER run --rm -v "$PWD:/src" -v blessh-cargo-registry:/root/.cargo/registry -v blessh-deb-target:/src/target \
    blessh-deb-builder sh -ec "
    for t in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
        for p in blessh-server blessh-client; do
            cargo deb -p \$p --target \$t --deb-version '$VER'
        done
        cp target/\$t/debian/*.deb /src/dist/
    done
    chown -R $(id -u):$(id -g) /src/dist"
ls -l dist/
