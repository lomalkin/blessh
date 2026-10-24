#!/bin/sh
# End-to-end test without a radio: OpenSSH client -> ble-term-loopback
# (BleStream client and server over a simulated BLE link) -> unprivileged
# `sshd -i`. The last round goes through a session socket and
# ble-term-session, as the packaged units do.
# Usage: tests/e2e-loopback.sh [path/to/ble-term-loopback]
set -eu

BIN=${1:-target/debug/ble-term-loopback}
BIN=$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")
SSHD=${SSHD:-/usr/sbin/sshd}
T=$(mktemp -d)
SESSION=$(dirname "$BIN")/ble-term-session
SA=
trap '[ -z "$SA" ] || kill $SA 2>/dev/null; rm -rf "$T"' EXIT

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
echo "== session socket and ble-term-session"
systemd-socket-activate -l "$T/session.sock" --inetd -a \
    "$SESSION" -- "$SSHD" -i -f "$T/sshd_config" -E "$T/sshd.log" > "$T/session.log" 2>&1 &
SA=$!
for _ in $(seq 50); do [ -S "$T/session.sock" ] && break; sleep 0.1; done
out=$(timeout 60 ssh -F /dev/null -i "$T/id" -o IdentitiesOnly=yes -o BatchMode=yes \
    -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile="$T/known_hosts" \
    -o ProxyCommand="$BIN --socket $T/session.sock" "$(id -un)@loopback" 'echo hello-over-ble')
[ "$out" = hello-over-ble ] || { echo "FAIL: socket echo: $out"; exit 1; }
grep -Eq 'session [0-9a-f]{16}: BLE device 00:00:00:00:00:00 \(unknown address\)' "$T/session.log" ||
    { echo "FAIL: no session log line"; cat "$T/session.log"; exit 1; }
# Anything that is not a preamble is refused before sshd runs.
timeout 15 python3 -c '
import socket, sys
s = socket.socket(socket.AF_UNIX)
s.connect(sys.argv[1])
s.sendall(b"SSH-2.0-test\r\n")
sys.stdout.buffer.write(s.recv(4096))
' "$T/session.sock" > "$T/raw.out" 2>/dev/null || true
for _ in $(seq 50); do grep -q 'refusing session' "$T/session.log" && break; sleep 0.1; done
grep -q 'refusing session: invalid session preamble: bad magic' "$T/session.log" ||
    { echo "FAIL: raw SSH was not refused"; cat "$T/session.log"; exit 1; }
[ ! -s "$T/raw.out" ] || { echo "FAIL: sshd answered without a preamble"; exit 1; }
echo "PASS"
