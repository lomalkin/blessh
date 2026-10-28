#!/bin/sh
# End-to-end test over real BlueZ with two virtual controllers (btvirt):
# blessh-server serves on one adapter as an unprivileged user, blessh
# connects from the other as an OpenSSH ProxyCommand, over the L2CAP
# channel and over the GATT stream. sshd is started per session by
# systemd-socket-activate and blessh-session, mimicking blessh-session.socket.
#
# Setup (root): modprobe hci_vhci; btvirt -L -l2. The virtual controllers
# are hci0 and hci1 unless a real adapter holds an index; SERVER_HCI and
# CLIENT_HCI name them.
#
# DAEMON_USER=<user> runs the daemon as that user (through sudo) to test the
# D-Bus policy: install packaging/linux/dbus.conf with _blessh replaced by
# that user into the bus configuration first. The test then also checks
# that the user is denied BlueZ calls the daemon does not need.
set -eu

BIN=$(pwd)/target/debug
SERVER_HCI=${SERVER_HCI:-hci0}
CLIENT_HCI=${CLIENT_HCI:-hci1}
SERVER=$(busctl get-property org.bluez "/org/bluez/$SERVER_HCI" org.bluez.Adapter1 Address | cut -d'"' -f2)
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
DAEMON_ARGS="--adapter $SERVER_HCI --name vhci-server --session-socket $T/session.sock --host-key $T/hostkey.pub --max-sessions 2"
if [ -n "${DAEMON_USER:-}" ]; then
    cp "$BIN/blessh-server" "$T/"
    sleep 1
    chmod 666 "$T/session.sock"
    # shellcheck disable=SC2086
    sudo -u "$DAEMON_USER" env RUST_LOG=debug ${DBUS_SYSTEM_BUS_ADDRESS:+DBUS_SYSTEM_BUS_ADDRESS=$DBUS_SYSTEM_BUS_ADDRESS} \
        "$T/blessh-server" $DAEMON_ARGS > "$T/daemon.log" 2>&1 &
else
    # shellcheck disable=SC2086
    RUST_LOG=debug "$BIN/blessh-server" $DAEMON_ARGS > "$T/daemon.log" 2>&1 &
fi
D=$!
trap 'kill $D $SA 2>/dev/null || true' EXIT
sleep 3
grep -q "L2CAP channels on PSM" "$T/daemon.log" || { echo "FAIL: no L2CAP channel"; cat "$T/daemon.log"; exit 1; }

# Successive clients share the one LE link: let the previous blessh finish
# (the last one disconnects on exit) before starting the next.
wait_link_down() {
    for _ in $(seq 40); do
        pgrep -x blessh > /dev/null || return 0
        sleep 0.25
    done
}

SSH="ssh -F /dev/null -i $T/id -o IdentitiesOnly=yes -o BatchMode=yes -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile=$T/known_hosts"

run() {
    wait_link_down
    # shellcheck disable=SC2086
    timeout 120 $SSH -o HostKeyAlias=vhci-server \
        -o ProxyCommand="env RUST_LOG=info $BIN/blessh proxy ${CONNECT_ARGS:-} --adapter $CLIENT_HCI $SERVER 2>>$T/connect.log" \
        "$(id -un)@vhci-server" "$@"
}

# Like a phone: the client leaves the link up on exit and the next client
# reuses it.
run_keep() {
    # shellcheck disable=SC2086
    timeout 60 $SSH -o HostKeyAlias=vhci-server \
        -o ProxyCommand="env RUST_LOG=info $BIN/blessh proxy --keep-link ${CONNECT_ARGS:-} --adapter $CLIENT_HCI $SERVER 2>>$T/connect.log" \
        "$(id -un)@vhci-server" "$@"
}

# Without waiting for other clients: several sessions over one link.
run_now() {
    # shellcheck disable=SC2086
    timeout 60 $SSH -o HostKeyAlias=vhci-server \
        -o ProxyCommand="env RUST_LOG=info $BIN/blessh proxy ${CONNECT_ARGS:-} --adapter $CLIENT_HCI $SERVER 2>>$T/connect.log" \
        "$(id -un)@vhci-server" "$@"
}

# With btvirt the first scans after start-up sometimes see no advertisement
# at all (not seen on real controllers); wait until the server is visible.
for _ in 1 2 3 4 5 6; do
    "$BIN/blessh" scan --adapter "$CLIENT_HCI" --timeout 8 2>/dev/null | grep -q "$SERVER" && break
done

head -c 100000 /dev/urandom > "$T/blob"
want=$(sha256sum < "$T/blob" | cut -d' ' -f1)

rounds() {
    for i in 1 2; do
        echo "== $1, round $i"
        out=$(run 'echo hello-over-ble')
        [ "$out" = hello-over-ble ] || { echo "FAIL: echo: $out"; exit 1; }
        got=$(run "cat '$T/blob'" | sha256sum | cut -d' ' -f1)
        [ "$got" = "$want" ] || { echo "FAIL: download checksum"; exit 1; }
        got=$(run 'sha256sum' < "$T/blob" | cut -d' ' -f1)
        [ "$got" = "$want" ] || { echo "FAIL: upload checksum"; exit 1; }
        tty=$(run -tt tty < /dev/null 2>/dev/null | tr -d '\r')
        case $tty in /dev/pts/*) ;; *) echo "FAIL: no pty: $tty"; exit 1 ;; esac
    done
}

rounds "L2CAP channel"
grep -q "l2cap, mtu [0-9]" "$T/connect.log" || { echo "FAIL: client did not report the L2CAP channel"; exit 1; }
grep -q "client connected (l2cap, " "$T/daemon.log" || { echo "FAIL: server did not log an L2CAP session"; exit 1; }
grep -q "transport l2cap" "$T/socket-activate.log" || { echo "FAIL: session helper did not log the transport"; exit 1; }

echo "== ssh through the blessh wrapper"
out=$(wait_link_down; BLESSH_PROXY_ARGS="--adapter $CLIENT_HCI" timeout 120 "$BIN/blessh" -F /dev/null -i "$T/id" -o IdentitiesOnly=yes \
    -o BatchMode=yes -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile="$T/known_hosts" "$(id -un)@$SERVER" 'echo via-wrapper')
[ "$out" = via-wrapper ] || { echo "FAIL: wrapper: $out"; exit 1; }
grep -q "^blessh-$(echo "$SERVER" | tr -d : | tr 'A-F' 'a-f') " "$T/known_hosts" || { echo "FAIL: wrapper did not set the host key alias"; exit 1; }

echo "== two sessions at once over one link"
wait_link_down
run_now 'sleep 4; echo first' > "$T/first.out" 2>&1 &
FIRST=$!
sleep 2
out=$(run_now 'echo second')
[ "$out" = second ] || { echo "FAIL: second session: $out"; exit 1; }
wait $FIRST
[ "$(cat "$T/first.out")" = first ] || { echo "FAIL: first session: $(cat "$T/first.out")"; exit 1; }

echo "== session limit (--max-sessions 2)"
wait_link_down
run_now 'sleep 6; echo a' > "$T/a.out" 2>&1 &
A=$!
run_now 'sleep 6; echo b' > "$T/b.out" 2>&1 &
B=$!
sleep 3
if run_now 'echo c' > "$T/c.out" 2>&1; then echo "FAIL: third session was admitted"; exit 1; fi
grep -q "refused, 2 sessions in progress" "$T/daemon.log" || { echo "FAIL: no refusal logged"; exit 1; }
wait $A $B
[ "$(cat "$T/a.out")" = a ] && [ "$(cat "$T/b.out")" = b ] || { echo "FAIL: held sessions broke"; exit 1; }

echo "== session ends when the client disappears"
wait_link_down
(run -tt 'sleep 4242' < /dev/null > /dev/null 2>&1 &)
sleep 8
pgrep -x sleep -a | grep -q 4242 || { echo "FAIL: remote command did not start"; exit 1; }
pkill -x blessh
sleep 5
if pgrep -x sleep -a | grep -q 4242; then echo "FAIL: session survived disconnect"; exit 1; fi

echo "== reconnect over a link the previous client left up"
wait_link_down
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

CONNECT_ARGS=--gatt rounds "GATT stream"
grep -q "server info: proto=1;name=vhci-server;mtu=[0-9]*;psm=[0-9]" "$T/connect.log" || { echo "FAIL: INFO"; exit 1; }
grep -q "gatt, frame [0-9]" "$T/connect.log" || { echo "FAIL: client did not report the GATT stream"; exit 1; }
grep -q "client connected (gatt, " "$T/daemon.log" || { echo "FAIL: server did not log a GATT session"; exit 1; }
echo "== GATT stream, writes with response"
got=$(CONNECT_ARGS="--gatt --write-with-response" run 'sha256sum' < "$T/blob" | cut -d' ' -f1)
[ "$got" = "$want" ] || { echo "FAIL: upload with write requests"; exit 1; }
grep -q "writes with response" "$T/connect.log" || { echo "FAIL: write mode not used"; exit 1; }

if [ -n "${DAEMON_USER:-}" ]; then
    echo "== D-Bus policy denies other BlueZ calls"
    as_daemon() {
        sudo -u "$DAEMON_USER" env ${DBUS_SYSTEM_BUS_ADDRESS:+DBUS_SYSTEM_BUS_ADDRESS=$DBUS_SYSTEM_BUS_ADDRESS} \
            dbus-send --system --print-reply --dest=org.bluez "$@" 2>&1 || true
    }
    for call in "/org/bluez/$SERVER_HCI org.bluez.Adapter1.StartDiscovery" \
        "/org/bluez/$SERVER_HCI org.freedesktop.DBus.Properties.Set string:org.bluez.Adapter1 string:Pairable variant:boolean:true" \
        "/org/bluez/$SERVER_HCI org.bluez.Adapter1.RemoveDevice objpath:/org/bluez/$SERVER_HCI/dev_00_AA_01_01_00_01"; do
        # shellcheck disable=SC2086
        as_daemon $call | grep -q AccessDenied || { echo "FAIL: allowed: $call"; exit 1; }
    done
fi

grep -E "rejecting|no longer connected|did not open|refused" "$T/daemon.log" | sed 's/^/   daemon: /' || true
echo PASS
