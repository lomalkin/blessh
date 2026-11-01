#!/bin/sh
# Installs the blessh packages in a throwaway Debian system (a container,
# as root) and checks that BLE sessions get the system SSH server's
# configuration: settings in /etc/ssh/sshd_config.d apply, settings above
# the Include in /etc/blessh/sshd_config apply to BLE only, the client
# runs, and purge leaves nothing behind.
#
#   tests/e2e-package.sh <blessh-server_*.deb> <blessh-client_*.deb>
set -u

[ $# -ge 2 ] || { sed -n '8p' "$0"; exit 2; }
[ "$(id -u)" = 0 ] || { echo "run as root, in a container"; exit 2; }
SERVER_DEB=$(realpath "$1")
CLIENT_DEB=$(realpath "$2")
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

export DEBIAN_FRONTEND=noninteractive
# The server package needs the client package: both use the one program.
apt-get install -y --no-install-recommends "$SERVER_DEB" "$CLIENT_DEB" > "$T/install.log" 2>&1 || { cat "$T/install.log"; exit 1; }
# apt runs dpkg on a pseudo-terminal: lines end in CR LF.
tr -d '\r' < "$T/install.log" > "$T/install.txt" && mv "$T/install.txt" "$T/install.log"
sed -n '/^Setting up blessh-server/,$p' "$T/install.log"

blessh server run --version > /dev/null
check "server runs" $?
blessh server session --version > /dev/null
check "session helper runs" $?
blessh server status | grep -q '^  Sessions '
check "server status" $?
getent passwd _blessh > /dev/null
check "installer creates the _blessh user" $?
fp=$(ssh-keygen -l -f /etc/ssh/ssh_host_ed25519_key.pub | awk '{ print $2 }')
grep -q "^  Host key  $fp (same as network SSH)" "$T/install.log"
check "installer prints the system host key fingerprint" $?
grep -q '^  Policy    as network SSH: passwords ' "$T/install.log"
check "installer prints the login policy" $?
grep -q '^  sshd configuration  valid  *OK$' "$T/install.log"
check "installer checks the sshd configuration" $?
! sed -n '/^blessh-server .*: SSH over Bluetooth LE$/,/^Processing triggers/p' "$T/install.log" | grep -q '.\{101\}'
check "installer lines fit in 100 columns" $?
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
dpkg-reconfigure blessh-server 2>&1 | grep -q 'as network SSH: passwords no, root [a-z-]*, AllowUsers alice$'
check "installer reports passwords off and AllowUsers" $?

sed -i 's/^#AllowGroups blessh/AllowGroups blessh/' /etc/blessh/sshd_config
[ "$(ble allowgroups)" = blessh ]
check "setting above the Include applies to BLE" $?
! /usr/sbin/sshd -T | grep -q '^allowgroups'
check "... and not to the network SSH server" $?
rm /etc/ssh/sshd_config.d/00-test.conf

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

apt-get purge -y blessh-server > "$T/purge.log" 2>&1 || { cat "$T/purge.log"; exit 1; }
[ ! -e /etc/blessh ] && [ ! -e /etc/default/blessh ]
check "purge removes the configuration" $?
[ -f /etc/ssh/ssh_host_ed25519_key ]
check "purge keeps the system host keys" $?
[ -x /usr/bin/blessh ]
check "the client stays without the server" $?
apt-get purge -y blessh-client > "$T/client-purge.log" 2>&1 || { cat "$T/client-purge.log"; exit 1; }
[ ! -e /usr/bin/blessh ]
check "client purge removes the program" $?

echo "$passed passed, $failed failed"
[ "$failed" = 0 ]
