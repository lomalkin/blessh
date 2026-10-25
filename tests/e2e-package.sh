#!/bin/sh
# Installs a ble-term package in a throwaway Debian system (a container,
# as root) and checks that BLE sessions get the system SSH server's
# configuration: settings in /etc/ssh/sshd_config.d apply, settings above
# the Include in /etc/ble-term/sshd_config apply to BLE only, and purge
# leaves nothing behind.
#
#   tests/e2e-package.sh <ble-term_*.deb>
set -u

[ $# -eq 1 ] || { sed -n '8p' "$0"; exit 2; }
[ "$(id -u)" = 0 ] || { echo "run as root, in a container"; exit 2; }
DEB=$(realpath "$1")
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
ble() { /usr/sbin/sshd -T -f /etc/ble-term/sshd_config | awk -v k="$1" '$1 == k { $1 = ""; sub(/^ /, ""); print; exit }'; }

export DEBIAN_FRONTEND=noninteractive
apt-get install -y --no-install-recommends "$DEB" > "$T/install.log" 2>&1 || { cat "$T/install.log"; exit 1; }
sed -n '/^Setting up ble-term/,$p' "$T/install.log"

[ ! -e /etc/ble-term/ssh_host_ed25519_key ]
check "no host key of its own" $?
fp=$(ssh-keygen -l -f /etc/ssh/ssh_host_ed25519_key.pub | awk '{ print $2 }')
grep -q "^  Host key  $fp (same as network SSH)" "$T/install.log"
check "installer prints the system host key fingerprint" $?
grep -q '^  Policy    as network SSH: passwords ' "$T/install.log"
check "installer prints the login policy" $?
grep -q '^  sshd configuration  valid  *OK$' "$T/install.log"
check "installer checks the sshd configuration" $?
! sed -n '/^ble-term .*: SSH over Bluetooth LE$/,/^Processing triggers/p' "$T/install.log" | grep -q '.\{101\}'
check "installer lines fit in 100 columns" $?
/usr/sbin/sshd -t -f /etc/ble-term/sshd_config
check "sshd accepts the configuration" $?
keys=$(/usr/sbin/sshd -T -f /etc/ble-term/sshd_config | grep '^hostkey ')
echo "$keys" | grep -qx 'hostkey /etc/ssh/ssh_host_ed25519_key' && ! echo "$keys" | grep -q /etc/ble-term
check "system host keys" $?

printf 'PasswordAuthentication no\nKbdInteractiveAuthentication no\nAllowUsers alice\nMaxAuthTries 2\n' > /etc/ssh/sshd_config.d/00-test.conf
[ "$(ble passwordauthentication)" = no ] && [ "$(ble kbdinteractiveauthentication)" = no ]
check "passwords off in sshd_config.d apply to BLE" $?
[ "$(ble allowusers)" = alice ]
check "AllowUsers in sshd_config.d applies to BLE" $?
[ "$(ble maxauthtries)" = 2 ]
check "MaxAuthTries in sshd_config.d applies to BLE" $?
dpkg-reconfigure ble-term 2>&1 | grep -q 'as network SSH: passwords no, root [a-z-]*, AllowUsers alice$'
check "installer reports passwords off and AllowUsers" $?

sed -i 's/^#AllowGroups ble-term/AllowGroups ble-term/' /etc/ble-term/sshd_config
[ "$(ble allowgroups)" = ble-term ]
check "setting above the Include applies to BLE" $?
! /usr/sbin/sshd -T | grep -q '^allowgroups'
check "... and not to the network SSH server" $?
rm /etc/ssh/sshd_config.d/00-test.conf

# EATT: off on BlueZ < 5.67 through the kernel's ECRED switch (a stand-in
# file here), untouched on later BlueZ or with an explicit Channels setting.
eatt() { echo Y > "$T/ecred"; rm -f "$T/state"; BLE_TERM_ECRED="$T/ecred" BLE_TERM_STATE="$T/state" /usr/lib/ble-term/eatt-off > /dev/null; cat "$T/ecred"; }
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
grep -q 'ExecStartPre=+/usr/lib/ble-term/eatt-off' /usr/lib/systemd/system/ble-term.service
check "service runs eatt-off" $?

apt-get purge -y ble-term > "$T/purge.log" 2>&1 || { cat "$T/purge.log"; exit 1; }
[ ! -e /etc/ble-term ] && [ ! -e /etc/default/ble-term ]
check "purge removes the configuration" $?
[ -f /etc/ssh/ssh_host_ed25519_key ]
check "purge keeps the system host keys" $?

echo "$passed passed, $failed failed"
[ "$failed" = 0 ]
