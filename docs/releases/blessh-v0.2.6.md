Phones no longer lose a session when they reconnect soon after the previous one.

* Linux server: the 30-second grace period after a link's last session now
  ends when the client closes the link. Before, the timer was keyed by the
  client's address; phones keep their address across links, so the timer
  of a closed link could cut the phone's next link while it was still
  setting up. The release notes of 0.2.3 to 0.2.5 put these cut sessions
  down to the old link's disconnect; it was this timer.
* Works with ConnectBot blessh r4, which also connects about 20 seconds
  faster on Android 11.
