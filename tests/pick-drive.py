#!/usr/bin/env python3
"""Runs a command on a pseudo-terminal and, once a row of blessh's list
names SERVER, presses that row's key; prints everything the command wrote
and exits with its code.

Usage: pick-drive.py SERVER COMMAND [ARG...]
"""

import os
import pty
import re
import select
import sys
import time

server = sys.argv[1].encode()
argv = sys.argv[2:]
pid, fd = pty.fork()
if pid == 0:
    os.execvp(argv[0], argv)

row = re.compile(rb"^ *([1-9a-z]) +" + re.escape(server) + rb" ", re.M)
out = b""
pressed = False
end = time.monotonic() + 120
while time.monotonic() < end:
    ready, _, _ = select.select([fd], [], [], 0.5)
    if not ready:
        continue
    try:
        data = os.read(fd, 4096)
    except OSError:
        break
    if not data:
        break
    out += data
    if not pressed:
        m = row.search(out)
        if m:
            os.write(fd, m.group(1))
            pressed = True

if not pressed:
    os.kill(pid, 15)
_, status = os.waitpid(pid, 0)
sys.stdout.write(out.decode(errors="replace"))
sys.exit(os.waitstatus_to_exitcode(status) if pressed else 124)
