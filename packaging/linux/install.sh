#!/bin/sh
# Installs blessh from its GitHub release on Debian, Ubuntu or Raspberry
# Pi OS (64-bit), and with --server turns its server on:
#
#   curl -fsSL https://github.com/lomalkin/blessh/releases/latest/download/install.sh | sh -s -- --server
#   curl -fsSL https://github.com/lomalkin/blessh/releases/latest/download/install.sh | sh
#
# The release job fills in the version and the packages' checksums
# (packaging/linux/render-install.sh). Running it again is safe: it
# installs this release over what is there. BLESSH_BASE replaces the
# download location (a URL or a directory), for tests.
set -eu

VERSION=@VERSION@
TAG=@TAG@
SHA256_AMD64=@SHA256_AMD64@
SHA256_ARM64=@SHA256_ARM64@

say() { echo "blessh install: $*"; }
fail() { echo "blessh install: $*" >&2; exit 1; }

fetch() {
    case $1 in
    /*) cp "$1" "$2" ;;
    *)
        if command -v curl > /dev/null; then
            curl -fsSL -o "$2" "$1"
        elif command -v wget > /dev/null; then
            wget -q -O "$2" "$1"
        else
            fail "needs curl or wget"
        fi
        ;;
    esac
}

main() {
    server=
    case ${1:-} in
    --server) server=1 ;;
    "") ;;
    *) fail "unknown option $1 (--server also turns the server on)" ;;
    esac

    if ! command -v dpkg > /dev/null || ! command -v apt-get > /dev/null; then
        fail "this installer is for Debian, Ubuntu and Raspberry Pi OS (apt); see https://github.com/lomalkin/blessh#installing"
    fi
    arch=$(dpkg --print-architecture)
    case $arch in
    amd64) sum=$SHA256_AMD64 ;;
    arm64) sum=$SHA256_ARM64 ;;
    *) sum= ;;
    esac
    [ -n "$sum" ] ||
        fail "no package for $arch: blessh is built for 64-bit systems (arm64, amd64); a Raspberry Pi needs the 64-bit Raspberry Pi OS"

    # The server needs sshd; nothing is installed without it.
    if [ -n "$server" ] && [ ! -x /usr/sbin/sshd ]; then
        cat >&2 << 'EOF'
blessh install: the server needs OpenSSH's server, which is not installed.
  Install it:  sudo apt install openssh-server
  (it also turns on network SSH; BLE does not need it: sudo systemctl disable --now ssh)
  Then run this command again.
EOF
        exit 1
    fi

    sudo=
    [ "$(id -u)" = 0 ] || sudo=sudo
    file=blessh_${VERSION}_$arch.deb
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    # apt reads the file as its own user.
    chmod 755 "$tmp"
    say "downloading $file ($TAG)"
    fetch "${BLESSH_BASE:-https://github.com/lomalkin/blessh/releases/download/$TAG}/$file" "$tmp/$file"
    echo "$sum  $tmp/$file" | sha256sum -c --status ||
        fail "$file does not match its checksum; nothing was installed"
    chmod 644 "$tmp/$file"
    say "installing $file"
    $sudo env DEBIAN_FRONTEND=noninteractive apt-get install -y -q "$tmp/$file" < /dev/null
    if [ -n "$server" ]; then
        say "turning the server on"
        $sudo /usr/bin/blessh server enable
    else
        say "done; connect with: blessh"
        say "to serve this machine as well: blessh server enable"
    fi
}

main "$@"
