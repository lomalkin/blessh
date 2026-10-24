# Repository conventions

These rules apply to people and coding agents alike.

## Language

Everything in this repository is written in English: code, comments, log
and error messages, documentation, commit messages and file names. Notes,
quotes or research in another language are translated before they are
committed.

## Commits

Commit messages describe the change. They carry no tool or agent
attribution (no `Co-Authored-By` for tools, no "Generated with" lines).

## Before changing behaviour

* Protocol and client-facing behaviour: read
  [COMPATIBILITY.md](COMPATIBILITY.md). Anything listed there must keep
  working with v1 clients that are already installed.
* Run `cargo clippy --all-targets -- -D warnings`, `cargo test` and
  `cargo build && tests/e2e-loopback.sh`.

## Where things go

| File | Contents |
|---|---|
| [PROTOCOL.md](PROTOCOL.md) | BleStream wire protocol (normative) |
| [COMPATIBILITY.md](COMPATIBILITY.md) | What server and clients must keep compatible in v1 |
| [TESTING.md](TESTING.md) | How to test on real hardware |
| [HARDWARE.md](HARDWARE.md) | Hardware and OS versions verified so far; add a row after each hardware test |
| [backlog.md](backlog.md) | Ideas that are agreed to be useful but not scheduled |
| [research/](research/) | Original architecture research: [ble-terminal-research.md](research/ble-terminal-research.md) and working notes A–E (historical; not updated) |
