Keeps the Bluetooth permission on macOS across Homebrew upgrades.

**Upgrading on macOS**

macOS keys the Bluetooth permission of blessh to the program's path, and
Homebrew installed every version under a new one: after an upgrade, the
server waited for a permission prompt. blessh now runs from a copy at a
path that stays (`$(brew --prefix)/var/lib/blessh/blessh`). After this
upgrade macOS asks once more, on the Mac's screen; answer it (or run
`blessh server enable`), and later upgrades keep the permission.

Linux packages are unchanged apart from the version.
