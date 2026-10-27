#!/bin/sh
# Installs blessh on macOS from the directory this script is in (the
# release tarball).
#
#   sudo ./install.sh [all|client|server]     (default: all)
#
# client: /usr/local/bin/blessh.
# server: the BLE server, run as the account that called sudo by a
# LaunchAgent (macOS grants Bluetooth to logged-in users only), and sshd per
# session as root through a LaunchDaemon with the sshd configuration.
set -eu
cd "$(dirname "$0")"

# bootout returns before the job is gone; bootstrap fails until it is.
replace_job() {
    domain=$1 label=$2 plist=$3
    launchctl bootout "$domain/$label" 2>/dev/null || true
    for _ in 1 2 3 4 5 6 7 8 9 10; do
        launchctl print "$domain/$label" > /dev/null 2>&1 || break
        sleep 0.5
    done
    launchctl bootstrap "$domain" "$plist"
}
WHAT=${1:-all}
case $WHAT in all|client|server) ;; *) echo "usage: sudo $0 [all|client|server]"; exit 2 ;; esac
[ "$(id -u)" = 0 ] || { echo "run with sudo"; exit 2; }

install_client() {
    install -d -m 0755 /usr/local/bin
    install -m 0755 blessh /usr/local/bin/blessh
    echo "blessh client installed: /usr/local/bin/blessh"
    echo "  macOS asks once whether blessh may use Bluetooth; check: blessh scan"
}

install_server() {
    USER_NAME=${SUDO_USER:-}
    [ -n "$USER_NAME" ] && [ "$USER_NAME" != root ] || { echo "run with sudo from a user account, not as root"; exit 2; }
    HOME_DIR=$(dscl . -read "/Users/$USER_NAME" NFSHomeDirectory | awk '{ print $2 }')
    UID_NUM=$(id -u "$USER_NAME")

    install -d -m 0755 /usr/local/libexec/blessh /etc/blessh
    install -m 0755 blessh-server blessh-session /usr/local/libexec/blessh/
    [ -e /etc/blessh/sshd_config ] || install -m 0644 sshd_config /etc/blessh/sshd_config
    # Host keys exist once Remote Login was ever on; make sure they do.
    ssh-keygen -A > /dev/null 2>&1 || true

    launchctl bootout system/org.blessh.session 2>/dev/null || true
    install -o root -g wheel -m 0644 org.blessh.session.plist /Library/LaunchDaemons/
    replace_job system org.blessh.session /Library/LaunchDaemons/org.blessh.session.plist

    install -d -o "$USER_NAME" -m 0755 "$HOME_DIR/Library/LaunchAgents"
    launchctl bootout "gui/$UID_NUM/org.blessh.server" 2>/dev/null || true
    install -o "$USER_NAME" -m 0644 org.blessh.server.plist "$HOME_DIR/Library/LaunchAgents/"
    replace_job "gui/$UID_NUM" org.blessh.server "$HOME_DIR/Library/LaunchAgents/org.blessh.server.plist"

    echo "blessh server installed: runs as $USER_NAME (log: /tmp/blessh-server.log), sessions as root"
    echo "  Name      $(/usr/local/libexec/blessh/blessh-server --print-name) (the computer name; another one: put it in /etc/blessh/name)"
    echo "  Host key  $(ssh-keygen -l -f /etc/ssh/ssh_host_ed25519_key.pub | awk '{ print $2 }') (same as Remote Login)"
    echo "  Override  /etc/blessh/sshd_config (settings above the Include apply to BLE only)"
}

case $WHAT in
    client) install_client ;;
    server) install_server ;;
    all) install_client; install_server ;;
esac
