#!/bin/sh
# End-to-end test over real BlueZ with two virtual controllers (btvirt):
# ble-term-daemon serves on hci0 as an unprivileged user, ble-term-connect
# connects from hci1 as an OpenSSH ProxyCommand. sshd is started per
# connection by systemd-socket-activate and ble-term-session, mimicking
# ble-term-session.socket.
set -eu

BIN=$(pwd)/target/debug
SERVER=${SERVER:-00:AA:01:00:00:00}
T=/tmp/ble-term-vhci
rm -rf "${T:?}"
mkdir -p "$T"

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
PidFile none
LogLevel ERROR
CFG

systemd-socket-activate -l "$T/session.sock" --inetd -a "$BIN/ble-term-session" -- /usr/sbin/sshd -i -f "$T/sshd_config" -E "$T/sshd.log" \
    > "$T/socket-activate.log" 2>&1 &
SA=$!
RUST_LOG=debug "$BIN/ble-term-daemon" --adapter hci0 --name vhci-server --session-socket "$T/session.sock" \
    --host-key "$T/hostkey.pub" > "$T/daemon.log" 2>&1 &
D=$!
trap 'kill $D $SA 2>/dev/null || true' EXIT
sleep 3

# Successive clients share the one LE link from hci1: let the previous
# ble-term-connect finish (it disconnects on exit) before starting the next.
wait_link_down() {
    for _ in $(seq 40); do
        pgrep -x ble-term-connec > /dev/null || return 0
        sleep 0.25
    done
}

run() {
    wait_link_down
    timeout 120 ssh -F /dev/null -i "$T/id" -o IdentitiesOnly=yes -o BatchMode=yes \
        -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile="$T/known_hosts" -o HostKeyAlias=vhci-server \
        -o ProxyCommand="env RUST_LOG=info $BIN/ble-term-connect --adapter hci1 $SERVER 2>>$T/connect.log" \
        "$(id -un)@vhci-server" "$@"
}

# Like a phone: the client leaves the link up on exit and the next client
# reuses it.
run_keep() {
    timeout 60 ssh -F /dev/null -i "$T/id" -o IdentitiesOnly=yes -o BatchMode=yes \
        -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile="$T/known_hosts" -o HostKeyAlias=vhci-server \
        -o ProxyCommand="env RUST_LOG=info $BIN/ble-term-connect --keep-link --adapter hci1 $SERVER 2>>$T/connect.log" \
        "$(id -un)@vhci-server" "$@"
}

# With btvirt the first scans after start-up sometimes see no advertisement
# at all (not seen on real controllers); wait until the server is visible.
for _ in 1 2 3 4 5 6; do
    "$BIN/ble-term-connect" --adapter hci1 --scan --scan-timeout 8 2>/dev/null | grep -q "$SERVER" && break
done

head -c 100000 /dev/urandom > "$T/blob"
want=$(sha256sum < "$T/blob" | cut -d' ' -f1)

for i in 1 2; do
    echo "== round $i"
    out=$(run 'echo hello-over-gatt')
    [ "$out" = hello-over-gatt ] || { echo "FAIL: echo: $out"; exit 1; }
    got=$(run "cat '$T/blob'" | sha256sum | cut -d' ' -f1)
    [ "$got" = "$want" ] || { echo "FAIL: download checksum"; exit 1; }
    got=$(run 'sha256sum' < "$T/blob" | cut -d' ' -f1)
    [ "$got" = "$want" ] || { echo "FAIL: upload checksum"; exit 1; }
    tty=$(run -tt tty < /dev/null 2>/dev/null | tr -d '\r')
    case $tty in /dev/pts/*) ;; *) echo "FAIL: no pty: $tty"; exit 1 ;; esac
done

echo "== session ends when the client disappears"
(run -tt 'sleep 4242' < /dev/null > /dev/null 2>&1 &)
sleep 8
pgrep -x sleep -a | grep -q 4242 || { echo "FAIL: remote command did not start"; exit 1; }
pkill -x ble-term-connec
sleep 5
if pgrep -x sleep -a | grep -q 4242; then echo "FAIL: session survived disconnect"; exit 1; fi

echo "== reconnect over a link the previous client left up"
[ "$(run_keep 'echo a')" = a ] || { echo "FAIL: first kept-link session"; exit 1; }
sleep 0.5
[ "$(run_keep 'echo b')" = b ] || { echo "FAIL: reconnect over the same link"; exit 1; }
# Reconnect just as the server drops the idle link (10 s after the session):
# that attempt may fail, but must not leave the server stuck.
sleep 9.7
run_keep 'echo c' > /dev/null 2>&1 || echo "   (attempt racing the server's disconnect failed, as allowed)"
sleep 1
for attempt in 1 2 3; do
    out=$(run 'echo d' 2>/dev/null) && break
    sleep 2
done
[ "$out" = d ] || { echo "FAIL: server stuck after a reconnect raced its disconnect"; exit 1; }
grep -E "rejecting|no longer connected|did not open" "$T/daemon.log" | sed 's/^/   daemon: /' || true

echo "PASS"
