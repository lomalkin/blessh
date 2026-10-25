#!/bin/sh
# Basic end-to-end checks over the radio between two bench machines: run on
# the client machine, against a ble-term server nearby (tier 2 in
# docs/testing.md).
#
#   tests/e2e-bench.sh [-k key] [-s bytes] [-u user] <server BLE address> <server name>
#
# The server name is the one it advertises (INFO `name`); it also names the
# host key entry. The public key (-k, default ~/.ssh/id_ed25519) must be in
# the user's authorized_keys on the server, and ble-term-connect must be
# installed here. Host key pins go to ~/.ssh/ble-term-bench_known_hosts.
set -u

KEY=$HOME/.ssh/id_ed25519
SIZE=30000
USER_NAME=$(id -un)
while getopts k:s:u: opt; do
    case $opt in
        k) KEY=$OPTARG ;;
        s) SIZE=$OPTARG ;;
        u) USER_NAME=$OPTARG ;;
        *) exit 2 ;;
    esac
done
shift $((OPTIND - 1))
[ $# -eq 2 ] || { sed -n '6,7p' "$0"; exit 2; }
ADDR=$1 NAME=$2

T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
LOG=$T/connect.log
: > "$LOG"

run() {
    extra=$1
    shift
    ssh -F /dev/null -i "$KEY" -o IdentitiesOnly=yes -o BatchMode=yes -o ConnectTimeout=60 \
        -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile="$HOME/.ssh/ble-term-bench_known_hosts" \
        -o HostKeyAlias="$NAME-ble" \
        -o ProxyCommand="env RUST_LOG=info ble-term-connect $extra $ADDR 2>>$LOG" \
        "$USER_NAME@$NAME-ble" "$@"
}

passed=0 failed=0
check() {
    if [ "$2" = 0 ]; then
        echo "  pass  $1"
        passed=$((passed + 1))
    else
        echo "  FAIL  $1 ${3:-}"
        failed=$((failed + 1))
    fi
}
rate() { awk "BEGIN { printf \"%.1f KB/s\", $SIZE / 1000 / ($2 - $1) }"; }

head -c "$SIZE" /dev/urandom > "$T/blob"
want=$(sha256sum < "$T/blob" | cut -d' ' -f1)
remote=/tmp/ble-term-bench.$$

out=$(run "" echo hello)
[ "$out" = hello ]
check "echo" $? "$out"

# Sessions right after each other: the previous client may still be
# disconnecting, and frames of the previous session may still arrive.
r=0
for i in 1 2 3 4 5; do
    [ "$(run "" echo "n$i")" = "n$i" ] || r=1
done
check "5 sessions back to back" $r

t0=$(date +%s.%N)
got=$(run "" "cat > $remote; sha256sum < $remote" < "$T/blob" | cut -d' ' -f1)
t1=$(date +%s.%N)
[ "$got" = "$want" ]
check "upload $SIZE bytes ($(rate "$t0" "$t1"))" $?

t0=$(date +%s.%N)
got=$(run "" "cat $remote; rm $remote" | sha256sum | cut -d' ' -f1)
t1=$(date +%s.%N)
[ "$got" = "$want" ]
check "download $SIZE bytes ($(rate "$t0" "$t1"))" $?

tty=$(run "" -tt tty < /dev/null 2>/dev/null | tr -d '\r')
case $tty in /dev/pts/*) r=0 ;; *) r=1 ;; esac
check "pty" $r "$tty"

t0=$(date +%s.%N)
got=$(run --write-with-response sha256sum < "$T/blob" | cut -d' ' -f1)
t1=$(date +%s.%N)
[ "$got" = "$want" ]
check "upload with write requests ($(rate "$t0" "$t1"))" $?

grep -q "server info: proto=1;name=$NAME;mtu=[0-9]" "$LOG"
check "INFO with mtu" $?
! grep -qi "out of order\|EATT" "$LOG"
check "no reordering or EATT warning" $?

# Link loss: the remote command must end when the client disappears.
marker=$((40000 + $$ % 10000))
(run "" -tt "sleep $marker" < /dev/null > /dev/null 2>&1 &)
sleep 12
for p in $(ps -eo pid,args | awk -v a="$ADDR" '$2 == "ble-term-connect" && $NF == a { print $1 }'); do
    kill "$p"
done
sleep 6
r=$(run "" "ps -eo args | grep -c '^sleep $marker\$'")
[ "$r" = 0 ]
check "session ends on link loss" $? "$r"

[ "$(run "" echo again)" = again ]
check "reconnect" $?

echo "  $passed passed, $failed failed; client waited for another instance $(grep -c 'waiting for another' "$LOG") times"
grep -E "WARN|ERROR" "$LOG" | sed 's/^/  /' | sort | uniq -c | head -5
[ "$failed" = 0 ]
