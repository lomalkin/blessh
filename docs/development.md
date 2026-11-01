# Development

Building, testing and releasing blessh. Repository conventions:
[AGENTS.md](AGENTS.md); test tiers and the hardware procedure:
[testing.md](testing.md).

## Layout

```
crates/blessh-core     protocol code without I/O, shared by every platform
crates/blessh-ble      Bluetooth layer: BlueZ (Linux) and CoreBluetooth (macOS)
crates/blessh-server   the server as a library (`blessh server ...`)
crates/blessh-client   the one program, blessh: client, and the server commands
packaging/linux        Debian package: systemd units, sshd_config, maintainer scripts
packaging/macos        Info.plist, the sshd launchd job, tarball and signing
packaging/homebrew     the formula template for the tap
tests/                 end-to-end tests, fuzzing, package tests
ci/local/              the CI release build in docker, without GitHub
```

Details of each component: [architecture.md](architecture.md).

## Checks

```
cargo fmt --check && cargo clippy --all-targets -- -D warnings
cargo test                              # protocol, relay, client and server unit tests
cargo build && tests/e2e-loopback.sh    # ssh ↔ simulated BLE ↔ sshd -i, no radio needed
tests/e2e-vhci.sh                       # both transports over BlueZ with btvirt (root; testing.md)
tests/docker/build-deb.sh               # the Debian packages, in a Debian container, into dist/
tests/docker/test-package.sh            # install, configuration checks and purge in a clean Debian
tests/e2e-cycles.sh <server>            # many short sessions against a server nearby
```

`blessh-loopback` runs both ends of the GATT stream over a simulated link
(small MTU, packet reordering) and is what the loopback test uses.

## macOS builds

The workspace builds on macOS as well, with the CoreBluetooth client and
server.

```
packaging/macos/build-tarball.sh                                    # universal program into dist/
BLESSH_TARGETS=aarch64-apple-darwin packaging/macos/build-tarball.sh # one architecture, quicker
packaging/homebrew/render.sh blessh dist/blessh-<version>-macos.tar.gz [url]   # the formula
```

The universal build needs both Rust targets (`rustup target add
aarch64-apple-darwin x86_64-apple-darwin`). With `BLESSH_SIGN_IDENTITY`
set to a code signing identity in the keychain, the program is signed with
it (`packaging/macos/sign.sh`), so that macOS keeps the Bluetooth
permission across builds; otherwise it is signed ad hoc. To try a formula
before release, put the rendered file with a `file://` URL into a local
tap (`brew tap-new --no-git <user>/local`) and install from there.

## Releases

Set `version` in the workspace `Cargo.toml`, commit, then push a matching
tag:

```
git tag blessh-vX.Y.Z && git push origin blessh-vX.Y.Z
```

CI (`.github/workflows/blessh.yml`) builds the Debian packages and the
macOS tarball and publishes a GitHub release with `SHA256SUMS`, the notes
from `docs/releases/<tag>.md` if present and the commit subjects since the
previous tag. Untagged CI builds are versioned `X.Y.Z~ciN`, which sorts
before the `X.Y.Z` release.

Repository secrets used by the release:

| Secret | Use |
|---|---|
| `HOMEBREW_TAP_DEPLOY_KEY` | deploy key with write access to the tap `lomalkin/homebrew-tap`; the release commits the rendered formula there |
| `BLESSH_SIGN_P12`, `BLESSH_SIGN_P12_PASSWORD` | base64 of a PKCS#12 file with the "blessh local signing" identity, and its password; the macOS program is signed with it |

Without them the release still happens: the formula is not published and
the program is signed ad hoc.

Without CI, `ci/local/build.sh blessh-vX.Y.Z` runs the same checks and
builds the same Debian packages in docker on any x86_64 Linux host, and
writes the release notes next to them; publish with `gh release create
blessh-vX.Y.Z <out>/*.deb <out>/SHA256SUMS --title "blessh X.Y.Z"
--notes-file <out>/notes.md`.
