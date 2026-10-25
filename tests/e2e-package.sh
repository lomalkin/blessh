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
grep -E '^(ble-term:|  )' "$T/install.log"

[ ! -e /etc/ble-term/ssh_host_ed25519_key ]
check "no host key of its own" $?
grep -q 'SSH host key fingerprint' "$T/install.log"
check "installer prints the system host key fingerprint" $?
grep -q 'BLE logins follow the system SSH server' "$T/install.log"
check "installer prints the login policy" $?
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
dpkg-reconfigure ble-term 2>&1 | grep -q 'passwords: no'
check "installer reports passwords off" $?

sed -i 's/^#AllowGroups ble-term/AllowGroups ble-term/' /etc/ble-term/sshd_config
[ "$(ble allowgroups)" = ble-term ]
check "setting above the Include applies to BLE" $?
! /usr/sbin/sshd -T | grep -q '^allowgroups'
check "... and not to the network SSH server" $?
rm /etc/ssh/sshd_config.d/00-test.conf

apt-get purge -y ble-term > "$T/purge.log" 2>&1 || { cat "$T/purge.log"; exit 1; }
[ ! -e /etc/ble-term ] && [ ! -e /etc/default/ble-term ]
check "purge removes the configuration" $?
[ -f /etc/ssh/ssh_host_ed25519_key ]
check "purge keeps the system host keys" $?

echo "$passed passed, $failed failed"
[ "$failed" = 0 ]
