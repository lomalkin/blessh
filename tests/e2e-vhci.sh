#!/bin/sh
# End-to-end test over real BlueZ with two virtual controllers (btvirt):
# blessh-server serves on hci0 as an unprivileged user, blessh
# connects from hci1 as an OpenSSH ProxyCommand. sshd is started per
# connection by systemd-socket-activate and blessh-session, mimicking
# blessh-session.socket.
#
# DAEMON_USER=<user> runs the daemon as that user (through sudo) to test the
# D-Bus policy: install packaging/dbus.conf with _blessh replaced by that
# user into the bus configuration first. The test then also checks that the
# user is denied BlueZ calls the daemon does not need.
set -eu

BIN=$(pwd)/target/debug
SERVER=${SERVER:-00:AA:01:00:00:00}
T=/tmp/blessh-vhci
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

systemd-socket-activate -l "$T/session.sock" --inetd -a "$BIN/blessh-session" -- /usr/sbin/sshd -i -f "$T/sshd_config" -E "$T/sshd.log" \
    > "$T/socket-activate.log" 2>&1 &
SA=$!
if [ -n "${DAEMON_USER:-}" ]; then
    cp "$BIN/blessh-server" "$T/"
    sleep 1
    chmod 666 "$T/session.sock"
    sudo -u "$DAEMON_USER" env RUST_LOG=debug ${DBUS_SYSTEM_BUS_ADDRESS:+DBUS_SYSTEM_BUS_ADDRESS=$DBUS_SYSTEM_BUS_ADDRESS} \
        "$T/blessh-server" --adapter hci0 --name vhci-server --session-socket "$T/session.sock" \
        --host-key "$T/hostkey.pub" > "$T/daemon.log" 2>&1 &
else
    RUST_LOG=debug "$BIN/blessh-server" --adapter hci0 --name vhci-server --session-socket "$T/session.sock" \
        --host-key "$T/hostkey.pub" > "$T/daemon.log" 2>&1 &
fi
D=$!
trap 'kill $D $SA 2>/dev/null || true' EXIT
sleep 3

# Successive clients share the one LE link from hci1: let the previous
# blessh finish (it disconnects on exit) before starting the next.
wait_link_down() {
    for _ in $(seq 40); do
        pgrep -x blessh-connec > /dev/null || return 0
        sleep 0.25
    done
}

run() {
    wait_link_down
    timeout 120 ssh -F /dev/null -i "$T/id" -o IdentitiesOnly=yes -o BatchMode=yes \
        -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile="$T/known_hosts" -o HostKeyAlias=vhci-server \
        -o ProxyCommand="env RUST_LOG=info $BIN/blessh ${CONNECT_ARGS:-} --adapter hci1 $SERVER 2>>$T/connect.log" \
        "$(id -un)@vhci-server" "$@"
}

# Like a phone: the client leaves the link up on exit and the next client
# reuses it.
run_keep() {
    timeout 60 ssh -F /dev/null -i "$T/id" -o IdentitiesOnly=yes -o BatchMode=yes \
        -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile="$T/known_hosts" -o HostKeyAlias=vhci-server \
        -o ProxyCommand="env RUST_LOG=info $BIN/blessh --keep-link --adapter hci1 $SERVER 2>>$T/connect.log" \
        "$(id -un)@vhci-server" "$@"
}

# With btvirt the first scans after start-up sometimes see no advertisement
# at all (not seen on real controllers); wait until the server is visible.
for _ in 1 2 3 4 5 6; do
    "$BIN/blessh" --adapter hci1 --scan --scan-timeout 8 2>/dev/null | grep -q "$SERVER" && break
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

echo "== INFO and writes with response"
grep -q "server info: proto=1;name=vhci-server;mtu=[0-9]" "$T/connect.log" || { echo "FAIL: INFO"; exit 1; }
got=$(CONNECT_ARGS=--write-with-response run 'sha256sum' < "$T/blob" | cut -d' ' -f1)
[ "$got" = "$want" ] || { echo "FAIL: upload with write requests"; exit 1; }
grep -q "writing with response" "$T/connect.log" || { echo "FAIL: write mode not used"; exit 1; }

echo "== session ends when the client disappears"
(run -tt 'sleep 4242' < /dev/null > /dev/null 2>&1 &)
sleep 8
pgrep -x sleep -a | grep -q 4242 || { echo "FAIL: remote command did not start"; exit 1; }
pkill -x blessh-connec
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

if [ -n "${DAEMON_USER:-}" ]; then
    echo "== D-Bus policy denies other BlueZ calls"
    as_daemon() {
        sudo -u "$DAEMON_USER" env ${DBUS_SYSTEM_BUS_ADDRESS:+DBUS_SYSTEM_BUS_ADDRESS=$DBUS_SYSTEM_BUS_ADDRESS} \
            dbus-send --system --print-reply --dest=org.bluez "$@" 2>&1 || true
    }
    for call in "/org/bluez/hci0 org.bluez.Adapter1.StartDiscovery" \
        "/org/bluez/hci0 org.freedesktop.DBus.Properties.Set string:org.bluez.Adapter1 string:Pairable variant:boolean:true" \
        "/org/bluez/hci0 org.bluez.Adapter1.RemoveDevice objpath:/org/bluez/hci0/dev_00_AA_01_01_00_01"; do
        # shellcheck disable=SC2086
        as_daemon $call | grep -q AccessDenied || { echo "FAIL: allowed: $call"; exit 1; }
    done
    as_daemon /org/bluez/hci0 org.freedesktop.DBus.Properties.Get string:org.bluez.Adapter1 string:Powered \
        | grep -q boolean || { echo "FAIL: Properties.Get denied"; exit 1; }
fi

echo "PASS"
