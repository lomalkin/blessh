# The blessh client

`blessh` on Linux and macOS: how it connects, what it changes in ssh's
behaviour, and what each platform adds. Installation:
[README](../README.md#installing).

## Usage

```
blessh                             # pick a server from a list
blessh alice@                      # the same, logging in as alice
blessh alice@my-pi                 # by the advertised name
blessh alice@AA:BB:CC:DD:EE:FF     # by Bluetooth address (Linux)
blessh -p 22 -L 8080:localhost:80 alice@my-pi   # any ssh option or command
blessh scan                        # servers in range, for scripts
```

`blessh` runs the system ssh with a ProxyCommand of `blessh proxy` and a
HostKeyAlias (`blessh-<name>` or `blessh-<address>`), so the server's host
key is pinned under that name in `~/.ssh/known_hosts`; compare the
fingerprint on the first connection. It also turns on ssh compression
(`-o Compression=no` turns it off). The user is ssh's choice: `user@`,
`-l`, `User` in `~/.ssh/config` for the server's name, else the current
user. To reach a host named `scan`, `proxy` or `server`, give a user
(`alice@server`) or put `--` before it.

The same works from `ssh_config`:

```
Host my-pi
    ProxyCommand blessh proxy my-pi
    HostKeyAlias blessh-my-pi
```

`BLESSH_PROXY_ARGS` passes options to the proxy, for example `--adapter
hci1` or `--gatt`; `blessh proxy --help` lists them.

## Choosing a server

Without a server, on a terminal, `blessh` lists the servers in range as
it hears them and connects to the one whose key you press:

```
$ blessh
Scanning for blessh servers · 1-9 a-z connect · 0 q Esc quit
 1  my-pi                          ▂▄▆█  -52 dBm  AA:BB:CC:00:00:01
 2  garage-pi                      ▂▄▆   -67 dBm  AA:BB:CC:00:00:02
 3  my-pi                          ▂▄    -74 dBm  AA:BB:CC:00:00:03  same name as 1
→ garage-pi
```

* Keys `1` to `9`, then `a` to `z` without `q`, select rows; a row keeps
  its key, and rows are only added, never redrawn. `0`, `q`, Esc and
  Ctrl-C quit. Nothing else selects, Enter included.
* The signal is the strength of the first advertisement heard, in dBm and
  as four bars (`####` without a UTF-8 locale); colours only on a terminal
  and without `NO_COLOR`.
* On macOS the column after the signal is the start of CoreBluetooth's
  device identifier; `blessh scan` prints it whole.
* ssh options and a command given without a server apply to the server
  chosen: `blessh alice@ uptime`, `blessh -l alice`, `blessh -L
  8080:localhost:80`.
* ssh connects to the server's name, so the host key and `~/.ssh/config`
  entries are those of the name; a server whose name another row has, or
  whose name is not a valid server name, is reached by its address (on
  macOS its identifier) instead, and its host key is pinned under that.
  The proxy goes straight to the device chosen (`blessh proxy --device`)
  and refuses it if its INFO names another server by then.
* While macOS asks for the Bluetooth permission, a line says so.

The list's own process keeps the terminal and runs ssh; a child `blessh
scan --format tsv --timeout 0` scans, so on macOS ssh keeps the
terminal's privacy permissions (files in Documents, for example) while
the scan has blessh's Bluetooth permission. Without a terminal, `blessh`
alone is `blessh scan`.

## Scan output

`blessh scan` prints a row per server as soon as its advertised name and
signal strength are known, each server once; servers whose name never
came are printed at the end without one. `--timeout` sets the seconds to
scan (10 by default); `--timeout 0` scans until the process is stopped or
its parent exits. `--format` chooses:

* `human` (default): the name in a column 29 characters wide (the
  longest server name), the signal, the device, and `duplicate name`
  after a name an earlier row had; on a terminal also signal bars and
  colours.
* `tsv`: `name`, `rssi` and `id` separated by tabs, one row per line.
* `json`: one object per line, `{"name":"my-pi","rssi":-52,"id":"AA:BB:CC:00:00:01"}`.

The fields, in every format:

| Field | Value |
|---|---|
| `name` | the advertised name, empty if unknown; control characters and backslashes as `\xHH`, since anyone in range can advertise any name |
| `rssi` | signal strength in dBm, an integer |
| `id` | Bluetooth address (Linux) or CoreBluetooth identifier in lower case (macOS): what `blessh` and `blessh proxy` accept as the server |

Fields may be added after `id` in later versions; a reader ignores those
it does not know. Errors and notes go to stderr.

## Server names

A server advertises a name that follows host name rules, at most 29 bytes,
compared without regard to case ([protocol.md](protocol.md#names)).
Servers derive their default from the host name: the first part, other
characters replaced by hyphens. If no server has the name given, the error
lists the names of the servers in range.

## Transports and fallback

The transports and the order in which a client tries them are part of the
protocol ([protocol.md](protocol.md#requirements)); this section describes
what `blessh` shows of them.

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
