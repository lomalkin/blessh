#!/bin/sh
# Installs the blessh server and client on macOS from the directory this
# script is in (the release tarball): binaries, the launchd jobs and the
# sshd configuration. Run with sudo from the account that should serve
# BLE sessions: the BLE server runs as that user (macOS grants Bluetooth to
# logged-in users only), sshd runs as root through launchd.
#
#   sudo ./install.sh
set -eu
cd "$(dirname "$0")"
[ "$(id -u)" = 0 ] || { echo "run with sudo"; exit 2; }
USER_NAME=${SUDO_USER:-}
[ -n "$USER_NAME" ] && [ "$USER_NAME" != root ] || { echo "run with sudo from a user account, not as root"; exit 2; }
HOME_DIR=$(dscl . -read "/Users/$USER_NAME" NFSHomeDirectory | awk '{ print $2 }')
UID_NUM=$(id -u "$USER_NAME")

install -d -m 0755 /usr/local/bin /usr/local/libexec/blessh /etc/blessh
install -m 0755 blessh /usr/local/bin/blessh
install -m 0755 blessh-server blessh-session /usr/local/libexec/blessh/
[ -e /etc/blessh/sshd_config ] || install -m 0644 sshd_config /etc/blessh/sshd_config
# Host keys exist once Remote Login was ever on; make sure they do.
ssh-keygen -A > /dev/null 2>&1 || true

launchctl bootout system/org.blessh.session 2>/dev/null || true
install -o root -g wheel -m 0644 org.blessh.session.plist /Library/LaunchDaemons/
launchctl bootstrap system /Library/LaunchDaemons/org.blessh.session.plist

install -d -o "$USER_NAME" -m 0755 "$HOME_DIR/Library/LaunchAgents"
launchctl bootout "gui/$UID_NUM/org.blessh.server" 2>/dev/null || true
install -o "$USER_NAME" -m 0644 org.blessh.server.plist "$HOME_DIR/Library/LaunchAgents/"
launchctl bootstrap "gui/$UID_NUM" "$HOME_DIR/Library/LaunchAgents/org.blessh.server.plist"

echo "blessh installed: server runs as $USER_NAME (log: /tmp/blessh-server.log), sessions as root"
echo "  Host key  $(ssh-keygen -l -f /etc/ssh/ssh_host_ed25519_key.pub | awk '{ print $2 }') (same as Remote Login)"
echo "  Allow Bluetooth for the terminal when macOS asks; check: /usr/local/bin/blessh scan"
echo "  Override  /etc/blessh/sshd_config (settings above the Include apply to BLE only)"
