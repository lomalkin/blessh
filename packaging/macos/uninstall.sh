#!/bin/sh
# Removes what install.sh installed. Run with sudo from the same account.
set -u
[ "$(id -u)" = 0 ] || { echo "run with sudo"; exit 2; }
USER_NAME=${SUDO_USER:-}
if [ -n "$USER_NAME" ] && [ "$USER_NAME" != root ]; then
    HOME_DIR=$(dscl . -read "/Users/$USER_NAME" NFSHomeDirectory | awk '{ print $2 }')
    launchctl bootout "gui/$(id -u "$USER_NAME")/org.blessh.server" 2>/dev/null || true
    rm -f "$HOME_DIR/Library/LaunchAgents/org.blessh.server.plist"
fi
launchctl bootout system/org.blessh.session 2>/dev/null || true
rm -f /Library/LaunchDaemons/org.blessh.session.plist /var/run/blessh-session.sock
rm -f /usr/local/bin/blessh /usr/local/libexec/blessh/blessh-server /usr/local/libexec/blessh/blessh-session
rmdir /usr/local/libexec/blessh 2>/dev/null || true
echo "blessh removed; /etc/blessh was kept (rm -r /etc/blessh to drop its sshd_config)"
