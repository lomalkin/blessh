#!/bin/sh
# End-to-end checks over the radio between two bench machines, through the
# installed `blessh` wrapper (tier 2 in docs/testing.md). Run on the client
# machine against a blessh server nearby:
#
#   tests/e2e-bench.sh [-s stage] [-k key] [-n size] [-u user] <server name>
#
# Stages run in order up to the one given (default: 2):
#   1  smoke: one session, login name and compression
#   2  basics: sessions back to back, upload and download, pty, two at once
#   3  long: 1 MB each way, 20 sessions back to back, link loss, reconnect
#
# The public key (-k, default ~/.ssh/id_ed25519) must be in the
# authorized_keys of the user on the server: -u, else the current user,
# which the smoke stage then checks is the default. Host key pins go to
# ~/.ssh/blessh-bench_known_hosts.
set -u

KEY=$HOME/.ssh/id_ed25519
STAGE=2
SIZE=100000
LOGIN=
while getopts s:k:n:u: opt; do
    case $opt in
        s) STAGE=$OPTARG ;;
        k) KEY=$OPTARG ;;
        n) SIZE=$OPTARG ;;
        u) LOGIN=$OPTARG ;;
        *) exit 2 ;;
    esac
done
shift $((OPTIND - 1))
[ $# -eq 1 ] || { sed -n '6,7p' "$0"; exit 2; }
NAME=$1
DEST=${LOGIN:+$LOGIN@}$NAME

T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
LOG=$T/connect.log
: > "$LOG"

# Portable between Linux and macOS, on both ends.
to() { if command -v timeout > /dev/null; then timeout "$@"; else perl -e 'alarm shift; exec @ARGV' "$@"; fi; }
sha() { if command -v sha256sum > /dev/null; then sha256sum; else shasum -a 256; fi | cut -d' ' -f1; }
RSHA='{ if command -v sha256sum >/dev/null; then sha256sum; else shasum -a 256; fi; } | cut -d" " -f1'

run() {
    to 180 env RUST_LOG=info blessh -F /dev/null -i "$KEY" -o IdentitiesOnly=yes -o BatchMode=yes \
        -o ConnectTimeout=60 -o StrictHostKeyChecking=accept-new \
        -o UserKnownHostsFile="$HOME/.ssh/blessh-bench_known_hosts" "$DEST" "$@" 2>>"$LOG"
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
now() { perl -MTime::HiRes=time -e 'printf "%.3f\n", time'; }
rate() { awk "BEGIN { printf \"%.1f KB/s\", $1 / 1000 / ($3 - $2) }"; }
finish() {
    echo "  $passed passed, $failed failed"
    grep -E "WARN|ERROR|^blessh: [^ ]+ [^ ]+: [^l]" "$LOG" | sed 's/^/  /' | sort | uniq -c | head -8
    [ "$failed" = 0 ]
    exit
}

echo "== stage 1: smoke"
out=$(run echo hello)
[ "$out" = hello ]
check "echo" $? "$out"
out=$(run 'echo $USER')
[ "$out" = "${LOGIN:-$(id -un)}" ]
check "logs in as the current user" $? "$out"
to 180 blessh -v -F /dev/null -i "$KEY" -o IdentitiesOnly=yes -o BatchMode=yes \
    -o UserKnownHostsFile="$HOME/.ssh/blessh-bench_known_hosts" "$DEST" true 2>&1 |
    grep -qi "enabling compression"
check "compression" $?
grep -q ": l2cap, " "$LOG"
check "L2CAP channel" $?
[ "$STAGE" -ge 2 ] || finish

echo "== stage 2: basics"
r=0
for i in 1 2 3 4 5; do
    [ "$(run echo "n$i")" = "n$i" ] || r=1
done
check "5 sessions back to back" $r

head -c "$SIZE" /dev/urandom > "$T/blob"
want=$(sha < "$T/blob")
remote=/tmp/blessh-bench.$$
t0=$(now)
got=$(run "cat > $remote; cat $remote | $RSHA" < "$T/blob")
t1=$(now)
[ "$got" = "$want" ]
check "upload $SIZE bytes ($(rate "$SIZE" "$t0" "$t1"))" $?
t0=$(now)
got=$(run "cat $remote; rm $remote" | sha)
t1=$(now)
[ "$got" = "$want" ]
check "download $SIZE bytes ($(rate "$SIZE" "$t0" "$t1"))" $?

tty=$(run -tt tty < /dev/null 2>/dev/null | tr -d '\r')
case $tty in /dev/pts/* | /dev/ttys*) r=0 ;; *) r=1 ;; esac
check "pty" $r "$tty"

run 'sleep 5; echo first' > "$T/first" &
FIRST=$!
sleep 2
out=$(run echo second)
wait $FIRST
[ "$out" = second ] && [ "$(cat "$T/first")" = first ]
check "two sessions at once" $? "$out / $(cat "$T/first")"
[ "$STAGE" -ge 3 ] || finish

echo "== stage 3: long"
BIG=1000000
head -c $BIG /dev/urandom > "$T/big"
want=$(sha < "$T/big")
t0=$(now)
got=$(run "$RSHA" < "$T/big")
t1=$(now)
[ "$got" = "$want" ]
check "upload 1 MB ($(rate $BIG "$t0" "$t1"))" $?
t0=$(now)
got=$(run "head -c $BIG /dev/zero" | wc -c | tr -d ' ')
t1=$(now)
[ "$got" = $BIG ]
check "download 1 MB ($(rate $BIG "$t0" "$t1"))" $? "$got"

r=0
for i in $(seq 20); do
    [ "$(run echo "m$i")" = "m$i" ] || { r=1; echo "    session $i failed"; }
done
check "20 sessions back to back" $r

# Link loss: the remote command must end when the client disappears.
marker=$((40000 + $$ % 10000))
(run -tt "sleep $marker" < /dev/null > /dev/null 2>&1 &)
sleep 10
pkill -f "blessh proxy .*$NAME"
sleep 6
r=$(run "ps -eo args | grep -c '^sleep $marker\$'")
[ "$r" = 0 ]
check "session ends on link loss" $? "$r"
[ "$(run echo again)" = again ]
check "reconnect" $?
finish
