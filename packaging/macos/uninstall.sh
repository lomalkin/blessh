#!/bin/sh
# Removes what install.sh installed. Run with sudo from the same account.
#
#   sudo ./uninstall.sh [all|client|server]   (default: all)
set -u
WHAT=${1:-all}
case $WHAT in all|client|server) ;; *) echo "usage: sudo $0 [all|client|server]"; exit 2 ;; esac
[ "$(id -u)" = 0 ] || { echo "run with sudo"; exit 2; }

if [ "$WHAT" != client ]; then
    USER_NAME=${SUDO_USER:-}
    if [ -n "$USER_NAME" ] && [ "$USER_NAME" != root ]; then
        HOME_DIR=$(dscl . -read "/Users/$USER_NAME" NFSHomeDirectory | awk '{ print $2 }')
        launchctl bootout "gui/$(id -u "$USER_NAME")/org.blessh.server" 2>/dev/null || true
        rm -f "$HOME_DIR/Library/LaunchAgents/org.blessh.server.plist"
    fi
    launchctl bootout system/org.blessh.session 2>/dev/null || true
    rm -f /Library/LaunchDaemons/org.blessh.session.plist /var/run/blessh-session.sock
    rm -f /usr/local/libexec/blessh/blessh-server /usr/local/libexec/blessh/blessh-session
    rmdir /usr/local/libexec/blessh 2>/dev/null || true
    echo "blessh server removed; /etc/blessh was kept (rm -r /etc/blessh to drop its name and sshd_config)"
fi
if [ "$WHAT" != server ]; then
    rm -f /usr/local/bin/blessh
    echo "blessh client removed"
fi
