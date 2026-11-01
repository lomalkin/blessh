A fix for clients that reconnect shortly after a session.

**Fixes**

* Linux server: a client that connected again about ten seconds after its
  previous session could have the new session cut right after it began,
  by the server dropping the old link. Reading INFO, which every client
  does before it opens a channel, now keeps the link up.

The macOS program is unchanged apart from the version.
