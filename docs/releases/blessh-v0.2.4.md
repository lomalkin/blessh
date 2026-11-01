Logs for links the server drops.

* Linux server: the journal says when the server drops a link that has
  carried no channel for ten seconds ("no channel for 10s,
  disconnecting"), and, at debug level, when a client reads INFO. Both
  help to tell a dropped idle link from a lost one.

A client that reconnects within a second or two of that moment can
still lose the new session; this is rare and documented in the backlog.
