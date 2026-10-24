# Repository conventions

These rules apply to people and coding agents alike.

## Language

Everything in this repository is written in English: code, comments, log
and error messages, documentation, commit messages and file names. Notes,
quotes or research in another language are translated before they are
committed.

## File names

Lower case with hyphens (`protocol.md`, `e2e-vhci.sh`), except files that
tools and readers look for by their exact name: `README.md`, `LICENSE`,
`CHANGELOG.md`, `AGENTS.md`, `SECURITY.md`.

## No local details

Host names, addresses, user names, passwords and other details of the
machines used for testing are not committed (also not in commit messages).
Tools that need them read a local config: the repository has an
`*.example` file next to it, the real one is listed in `.gitignore`.
Documentation describes test machines by hardware and OS only.

## Commits

Commit messages describe the change. They carry no tool or agent
attribution (no `Co-Authored-By` for tools, no "Generated with" lines).

## Before changing behaviour

* Protocol and client-facing behaviour: read
  [compatibility.md](compatibility.md). Anything listed there must keep
  working with v1 clients that are already installed.
* Run `cargo clippy --all-targets -- -D warnings`, `cargo test` and
  `cargo build && tests/e2e-loopback.sh`.

## Where things go

| File | Contents |
|---|---|
| [protocol.md](protocol.md) | BleStream wire protocol (normative) |
| [compatibility.md](compatibility.md) | What server and clients must keep compatible in v1 |
| [testing.md](testing.md) | How to test on real hardware |
| [hardware.md](hardware.md) | Hardware and OS versions verified so far; add a row after each hardware test |
| [backlog.md](backlog.md) | Ideas that are agreed to be useful but not scheduled |
| [research/](research/) | Original architecture research: [ble-terminal-research.md](research/ble-terminal-research.md) with its working notes (historical; not updated); client platforms (web, macOS, Windows, iOS): working notes [client-platforms-notes.md](research/client-platforms-notes.md), summary for implementers [client-other-platforms.md](research/client-other-platforms.md) |
