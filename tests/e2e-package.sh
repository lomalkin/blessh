#!/bin/sh
# Installs the blessh package in a throwaway Debian system (a container,
# as root): first through the installer script, then checks that the
# server stays off until enabled, that BLE sessions get the system SSH
# server's configuration (settings in /etc/ssh/sshd_config.d apply,
# settings above the Include in /etc/blessh/sshd_config apply to BLE
# only), that the client runs, and that purge leaves nothing behind.
#
#   tests/e2e-package.sh <blessh_*.deb>
set -u

[ $# -ge 1 ] || { sed -n '9p' "$0"; exit 2; }
[ "$(id -u)" = 0 ] || { echo "run as root, in a container"; exit 2; }
DEB=$(realpath "$1")
SRC=$(cd "$(dirname "$0")/.." && pwd)
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT

passed=0 failed=0
check() {
    if [ "$2" = 0 ]; then
        echo "  pass  $1"
        passed=$((passed + 1))
    else
        echo "  FAIL  $1"
        failed=$((failed + 1))
    fi
}
# Effective value of an sshd keyword for BLE sessions.
ble() { /usr/sbin/sshd -T -f /etc/blessh/sshd_config | awk -v k="$1" '$1 == k { $1 = ""; sub(/^ /, ""); print; exit }'; }
installed() { dpkg-query -W -f '${Status}' blessh 2> /dev/null | grep -q 'ok installed'; }

export DEBIAN_FRONTEND=noninteractive
[ ! -x /usr/sbin/sshd ] || { echo "the container must not have openssh-server yet"; exit 2; }

# The installer, with the package in a directory instead of a release.
mkdir -p "$T/release"
cp "$DEB" "$T/release/"
"$SRC/packaging/linux/render-install.sh" "$T/release" > "$T/install.sh"
export BLESSH_BASE="$T/release"

sh "$T/install.sh" --bogus > "$T/out" 2>&1; rc=$?
[ $rc != 0 ] && grep -q 'unknown option --bogus' "$T/out"
check "installer refuses an unknown option" $?
sed 's/^\(SHA256_AMD64=\).*/\1'"$(printf '0%.0s' $(seq 64))"'/' "$T/install.sh" > "$T/bad.sh"
sh "$T/bad.sh" > "$T/out" 2>&1; rc=$?
[ $rc != 0 ] && grep -q 'does not match its checksum' "$T/out" && ! installed
check "installer refuses a package with a wrong checksum" $?
sh "$T/install.sh" --server > "$T/out" 2>&1; rc=$?
[ $rc != 0 ] && grep -q 'sudo apt install openssh-server' "$T/out" && ! installed
check "installer --server without sshd says what to install, installs nothing" $?
sh "$T/install.sh" > "$T/install.log" 2>&1; rc=$?
[ $rc = 0 ] && installed
check "installer installs the client" $?
tr -d '\r' < "$T/install.log" > "$T/install.txt"
grep -q '^  Serve     blessh server enable' "$T/install.txt" && grep -q 'sudo apt install openssh-server' "$T/install.txt"
check "first installation prints how to connect and serve" $?
! sed -n '/^blessh .*: SSH over Bluetooth LE$/,/Docs/p' "$T/install.txt" | grep -q '.\{81\}'
check "its lines fit in 80 columns" $?

[ ! -e /etc/systemd/system/multi-user.target.wants/blessh-server.service ] &&
    [ ! -e /etc/systemd/system/sockets.target.wants/blessh-session.socket ]
check "the server stays off" $?
getent passwd _blessh > /dev/null
check "installation creates the _blessh user" $?
blessh server enable > "$T/out" 2>&1; rc=$?
[ $rc != 0 ] && grep -q 'sudo apt install openssh-server' "$T/out"
check "enable without sshd says what to install" $?

apt-get install -y --no-install-recommends openssh-server > "$T/sshd.log" 2>&1 || { cat "$T/sshd.log"; exit 1; }
blessh server run --version > /dev/null
check "server runs" $?
blessh server session --version > /dev/null
check "session helper runs" $?
blessh server status > "$T/status" 2>&1
fp=$(ssh-keygen -l -f /etc/ssh/ssh_host_ed25519_key.pub | awk '{ print $2 }')
grep -q "^  Host key  $fp (same as network SSH)" "$T/status"
check "status prints the system host key fingerprint" $?
grep -q '^  Policy    as network SSH: passwords ' "$T/status"
check "status prints the login policy" $?
grep -q '^  sshd configuration  valid  *OK$' "$T/status"
check "status checks the sshd configuration" $?
grep -q '^  Visible  *to blessh clients only  *OK$' "$T/status"
check "status: hidden from device lists by default" $?
! grep -q '.\{101\}' "$T/status"
check "status lines fit in 100 columns" $?
/usr/sbin/sshd -t -f /etc/blessh/sshd_config
check "sshd accepts the configuration" $?
keys=$(/usr/sbin/sshd -T -f /etc/blessh/sshd_config | grep '^hostkey ')
echo "$keys" | grep -qx 'hostkey /etc/ssh/ssh_host_ed25519_key' && ! echo "$keys" | grep -q /etc/blessh
check "system host keys" $?

printf 'PasswordAuthentication no\nKbdInteractiveAuthentication no\nAllowUsers alice\nMaxAuthTries 2\n' > /etc/ssh/sshd_config.d/00-test.conf
[ "$(ble passwordauthentication)" = no ] && [ "$(ble kbdinteractiveauthentication)" = no ]
check "passwords off in sshd_config.d apply to BLE" $?
[ "$(ble allowusers)" = alice ]
check "AllowUsers in sshd_config.d applies to BLE" $?
[ "$(ble maxauthtries)" = 2 ]
check "MaxAuthTries in sshd_config.d applies to BLE" $?
blessh server status 2>&1 | grep -q '^  Policy    as network SSH: passwords no, root [a-z-]*, AllowUsers alice$'
check "status reports passwords off and AllowUsers" $?

sed -i 's/^#AllowGroups blessh/AllowGroups blessh/' /etc/blessh/sshd_config
[ "$(ble allowgroups)" = blessh ]
check "setting above the Include applies to BLE" $?
! /usr/sbin/sshd -T | grep -q '^allowgroups'
check "... and not to the network SSH server" $?
rm /etc/ssh/sshd_config.d/00-test.conf

echo 'BLESSH_ARGS="--discoverable"' >> /etc/default/blessh
blessh server status 2>&1 | grep -q '^  Visible  *also in device lists  *OK$'
check "status: --discoverable shows it in device lists" $?

# EATT: off on BlueZ < 5.67 through the kernel's ECRED switch (a stand-in
# file here), untouched on later BlueZ or with an explicit Channels setting.
eatt() { echo Y > "$T/ecred"; rm -f "$T/state"; BLESSH_ECRED="$T/ecred" BLESSH_STATE="$T/state" /usr/lib/blessh/eatt-off > /dev/null; cat "$T/ecred"; }
if dpkg --compare-versions "$(dpkg-query -W -f '${Version}' bluez)" lt 5.67; then
    [ "$(eatt)" = N ] && [ -e "$T/state" ]
    check "ECRED off on BlueZ < 5.67" $?
    cp /etc/bluetooth/main.conf "$T/main.conf"
    sed -i 's/^\[GATT\]$/[GATT]\nChannels = 3/' /etc/bluetooth/main.conf
    [ "$(eatt)" = Y ]
    check "... unless main.conf sets Channels" $?
    cp "$T/main.conf" /etc/bluetooth/main.conf
else
    [ "$(eatt)" = Y ]
    check "ECRED left alone on BlueZ >= 5.67" $?
fi
grep -q 'ExecStartPre=+/usr/lib/blessh/eatt-off' /usr/lib/systemd/system/blessh-server.service
check "service runs eatt-off" $?
grep -q 'ExecStart=/usr/bin/blessh server run ' /usr/lib/systemd/system/blessh-server.service
check "service starts the server" $?
grep -qx 'RuntimeDirectory=sshd' /usr/lib/systemd/system/blessh-session@.service && [ ! -e /usr/lib/tmpfiles.d/blessh.conf ]
check "session unit creates /run/sshd (no tmpfiles line)" $?
grep -qx 'ListenStream=/run/blessh/session.sock' /usr/lib/systemd/system/blessh-session.socket
check "session socket path" $?

blessh --help | grep -q '^Usage: blessh \[ssh options\]'
check "client prints its usage" $?
blessh --version | grep -q '^blessh [0-9]'
check "client prints its version" $?
blessh proxy --help > /dev/null && blessh scan --help > /dev/null
check "client subcommands" $?

# Without systemd in the container, enable gets as far as turning the
# units on for the next boot.
sh "$T/install.sh" --server > "$T/out" 2>&1
grep -q 'turning the server on' "$T/out" && [ -e /etc/systemd/system/multi-user.target.wants/blessh-server.service ]
check "installer --server reinstalls and enables the server" $?

apt-get purge -y blessh > "$T/purge.log" 2>&1 || { cat "$T/purge.log"; exit 1; }
[ ! -e /etc/blessh ] && [ ! -e /etc/default/blessh ] && [ ! -e /usr/bin/blessh ]
check "purge removes the program and the configuration" $?
[ ! -e /etc/systemd/system/multi-user.target.wants/blessh-server.service ]
check "purge turns the server off" $?
[ -f /etc/ssh/ssh_host_ed25519_key ]
check "purge keeps the system host keys" $?

echo "$passed passed, $failed failed"
[ "$failed" = 0 ]
