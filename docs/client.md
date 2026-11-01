# The blessh client

`blessh` on Linux and macOS: how it connects, what it changes in ssh's
behaviour, and what each platform adds. Installation:
[README](../README.md#installing).

## Usage

```
blessh scan                        # servers in range
blessh alice@my-pi                 # by the advertised name
blessh alice@AA:BB:CC:DD:EE:FF     # by Bluetooth address (Linux)
blessh -p 22 -L 8080:localhost:80 alice@my-pi   # any ssh option or command
```

`blessh` runs the system ssh with a ProxyCommand of `blessh proxy` and a
HostKeyAlias (`blessh-<name>` or `blessh-<address>`), so the server's host
key is pinned under that name in `~/.ssh/known_hosts`; compare the
fingerprint on the first connection. It also turns on ssh compression
(`-o Compression=no` turns it off) and, without `user@` or `-l`, logs in
as the current user, like ssh does. To reach a host named `scan`, `proxy`
or `server`, give a user (`alice@server`) or put `--` before it.

The same works from `ssh_config`:

```
Host my-pi
    ProxyCommand blessh proxy my-pi
    HostKeyAlias blessh-my-pi
```

`BLESSH_PROXY_ARGS` passes options to the proxy, for example `--adapter
hci1` or `--gatt`; `blessh proxy --help` lists them.

## Server names

A server advertises a name that follows host name rules: letters, digits
and hyphens, parts separated by dots, at most 29 bytes (what fits the
advertisement), compared without regard to case. Defaults are derived
accordingly: the first part of a host name, other characters replaced by
hyphens. If no server has the name given, the error lists the names of the
servers in range.

## Transports and fallback

One line before the session says which transport is in use:
`blessh: my-pi AA:BB:CC:DD:EE:FF: l2cap, mtu 1024`, or `gatt, frame 497`
with a server that offers no channel.

A server can offer several L2CAP channels. `blessh` checks each with a
short handshake before SSH starts, falls back to the next one (and to the
GATT stream) when a channel stalls, and remembers in `~/.cache/blessh/`
which one worked for each server. Several sessions to one server share its
BLE link; the last `blessh` to exit disconnects it.

## Linux

The client machine needs no settings for the L2CAP channel. The GATT
stream (servers run with `--no-l2cap`, or `--gatt`) needs EATT off on the
client as well (`Channels = 1` in the `[GATT]` section of
`/etc/bluetooth/main.conf`, the default since BlueZ 5.67); `blessh` warns
if it is on.

The client makes the LE link itself and reads INFO over its own ATT
channel, so the client machine's bluetoothd neither connects over BR/EDR
(to a Mac, for example) nor discovers and probes the server's services.
That matters when the server also publishes a GATT service that requires
encryption, such as BLE MIDI from PipeWire's WirePlumber: bluetoothd's MIDI
plugin would read it and ask to pair, and the link would drop. The GATT
stream still goes through bluetoothd; for it, turn off BLE MIDI in
WirePlumber on the server, or start the client's bluetoothd without the
MIDI plugin (`-P midi`).

## macOS

The client uses the L2CAP channel only (the GATT stream is not available
on macOS yet), so the server must offer one: Linux and macOS servers do.

The first use of Bluetooth makes macOS ask, on the Mac's screen, whether
`blessh` may use it. The permission belongs to blessh itself, not to the
terminal or to sshd: `blessh` runs itself again as its own "responsible
process", so it also works when started over SSH or from scripts once
granted, and the client and the server share it. Until it is granted,
`blessh` reports "macOS has not decided whether this program may use
Bluetooth". macOS keys the permission to the program's signature: a build
signed ad hoc asks again after an upgrade.

macOS shows no Bluetooth addresses: `blessh scan` lists CoreBluetooth's
device identifiers instead, and `blessh user@<name>` selects by advertised
name. The client remembers which device had each server name
(`~/.cache/blessh/devices`): if a short scan does not find the name, it
connects to that device directly, since a server may not advertise right
after a previous session's link went down.
