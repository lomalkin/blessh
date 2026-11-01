#!/bin/sh
# Runs tests/e2e-package.sh (install, configuration checks, purge) in a
# clean Debian container for the amd64 package in dist/.
#
#   tests/docker/test-package.sh [image]      default: debian:bookworm
set -eu
cd "$(dirname "$0")/../.."
IMAGE=${1:-debian:bookworm}
DOCKER=${DOCKER:-docker}
deb=$(ls dist/blessh_*_amd64.deb | tail -n 1)
$DOCKER run --rm -v "$PWD:/src:ro" "$IMAGE" sh -ec "
    apt-get update -qq
    /src/tests/e2e-package.sh /src/$deb"
