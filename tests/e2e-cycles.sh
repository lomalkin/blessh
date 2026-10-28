#!/bin/sh
# Many short sessions against a server nearby, to exercise what fails most:
# connecting and disconnecting. Each session runs `echo`; failures keep the
# client's debug log. Runs on Linux and macOS with the installed `blessh`.
#
#   tests/e2e-cycles.sh [-n count] [-t seconds] [-m mode] [-u user] [-k key] <server name>
#
# Modes: seq (sessions back to back, default), pair (a second session
# starts while the first runs), all (both). -t is the time one session may
# take (default 40). Logs of failed sessions go to ./cycles-<server>/.
set -u

N=10 TIMEOUT=40 MODE=seq LOGIN= KEY=$HOME/.ssh/id_ed25519
while getopts n:t:m:u:k: opt; do
    case $opt in
        n) N=$OPTARG ;;
        t) TIMEOUT=$OPTARG ;;
        m) MODE=$OPTARG ;;
        u) LOGIN=$OPTARG ;;
        k) KEY=$OPTARG ;;
        *) exit 2 ;;
    esac
done
shift $((OPTIND - 1))
[ $# -eq 1 ] || { sed -n '6p' "$0"; exit 2; }
NAME=$1
DEST=${LOGIN:+$LOGIN@}$NAME
OUT=cycles-$NAME
mkdir -p "$OUT"

to() { if command -v timeout > /dev/null; then timeout "$@"; else perl -e 'alarm shift; exec @ARGV' "$@"; fi; }
now() { perl -MTime::HiRes=time -e 'printf "%.1f\n", time'; }

# session <label> <command...>: prints the remote output; the log goes to
# $OUT/<label>.log and is kept only if the session fails.
session() {
    label=$1
    shift
    to "$TIMEOUT" env RUST_LOG=debug,bluer=info blessh -F /dev/null -i "$KEY" -o IdentitiesOnly=yes \
        -o BatchMode=yes -o StrictHostKeyChecking=accept-new \
        -o UserKnownHostsFile="$HOME/.ssh/blessh-bench_known_hosts" "$DEST" "$@" 2> "$OUT/$label.log"
}

ok=0 failed=0
result() {
    if [ "$2" = "$3" ]; then
        ok=$((ok + 1))
        rm -f "$OUT/$1.log" "$OUT/$1.a.log" "$OUT/$1.b.log"
    else
        failed=$((failed + 1))
        echo "  FAIL $1: got [$2] after $4 s"
    fi
}

if [ "$MODE" = seq ] || [ "$MODE" = all ]; then
    for i in $(seq "$N"); do
        t0=$(now)
        out=$(session "seq-$i" echo "s$i")
        result "seq-$i" "$out" "s$i" "$(echo "$t0 $(now)" | awk '{ print $2 - $1 }')"
    done
    echo "seq: $ok of $N"
fi
if [ "$MODE" = pair ] || [ "$MODE" = all ]; then
    before=$ok
    for i in $(seq "$N"); do
        t0=$(now)
        session "pair-$i.a" "sleep 4; echo a$i" > "$OUT/pair-$i.a.out" &
        a=$!
        sleep 2
        b=$(session "pair-$i.b" echo "b$i")
        wait $a
        result "pair-$i" "$(cat "$OUT/pair-$i.a.out") $b" "a$i b$i" "$(echo "$t0 $(now)" | awk '{ print $2 - $1 }')"
        rm -f "$OUT/pair-$i.a.out"
    done
    echo "pair: $((ok - before)) of $N"
fi
echo "$ok passed, $failed failed; logs of failures in $OUT/"
[ "$failed" = 0 ]
