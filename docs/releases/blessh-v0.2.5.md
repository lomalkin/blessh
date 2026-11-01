Fewer cut sessions right after a reconnect.

* Linux server: an idle link now stays up for 30 seconds after its last
  session instead of 10. Clients usually come back within about ten
  seconds, which met the disconnect of the old link while it was under way
  (it takes up to three seconds on a Pi Zero 2 W) and cut the new session.
  A client that reconnects within the 30 seconds also reuses the link and
  connects faster. Servers whose controllers cannot advertise while
  connected become visible again 30 seconds after a session.
