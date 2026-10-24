#!/bin/sh
# End-to-end test without a radio: OpenSSH client -> ble-term-loopback
# (BleStream client and server over a simulated BLE link) -> unprivileged
# `sshd -i`. Usage: tests/e2e-loopback.sh [path/to/ble-term-loopback]
set -eu

BIN=${1:-target/debug/ble-term-loopback}
BIN=$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")
SSHD=${SSHD:-/usr/sbin/sshd}
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT

ssh-keygen -q -t ed25519 -N '' -f "$T/hostkey"
ssh-keygen -q -t ed25519 -N '' -f "$T/id"
cp "$T/id.pub" "$T/authorized_keys"
cat > "$T/sshd_config" <<CFG
HostKey $T/hostkey
AuthorizedKeysFile $T/authorized_keys
StrictModes no
UsePAM no
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
PidFile none
LogLevel ERROR
CFG

run() {
    mtu=$1; reorder=$2; shift 2
    timeout 120 ssh -F /dev/null -i "$T/id" -o IdentitiesOnly=yes -o BatchMode=yes \
        -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile="$T/known_hosts" \
        -o ProxyCommand="$BIN --mtu $mtu --reorder-every $reorder --exec '$SSHD -i -f $T/sshd_config -E $T/sshd.log'" \
        "$(id -un)@loopback" "$@"
}

head -c 300000 /dev/urandom > "$T/blob"
want=$(sha256sum < "$T/blob" | cut -d' ' -f1)

for cfg in "20 0" "20 5" "185 3" "512 0"; do
    set -- $cfg
    echo "== MTU $1, reorder every $2"
    out=$(run "$1" "$2" 'echo hello-over-ble')
    [ "$out" = hello-over-ble ] || { echo "FAIL: echo: $out"; exit 1; }
    got=$(run "$1" "$2" "cat '$T/blob'" | sha256sum | cut -d' ' -f1)
    [ "$got" = "$want" ] || { echo "FAIL: download checksum"; exit 1; }
    got=$(run "$1" "$2" 'sha256sum' < "$T/blob" | cut -d' ' -f1)
    [ "$got" = "$want" ] || { echo "FAIL: upload checksum"; exit 1; }
    tty=$(run "$1" "$2" -tt 'tty' < /dev/null 2>/dev/null | tr -d '\r')
    case $tty in /dev/pts/*) ;; *) echo "FAIL: no pty: $tty"; exit 1 ;; esac
done
echo "PASS"
