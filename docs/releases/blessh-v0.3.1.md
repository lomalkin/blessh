Fixes from the long test of 0.3.0.

* Linux server: after the adapter was switched off and on (a desktop's
  Bluetooth switch, rfkill, `bluetoothctl power off/on`), servers on
  controllers with extended advertising stayed invisible until
  restarted. The server now follows the adapter's power state and
  advertises again within seconds.
* Linux client: a client that found its server already connected (for
  example the other machine logged in to this one at the same time) took
  that link down when it exited, which ended the other machine's
  session. It now leaves alone a link it did not make.
* Linux client: when selecting a server by name, devices bluetoothd
  remembers from earlier scans no longer count unless they are heard
  now; a server that once advertised under another address made the
  client try that address for a minute.
