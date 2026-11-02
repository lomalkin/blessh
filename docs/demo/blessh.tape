# The README animation: an interactive blessh session from a Mac to a Pi.
#
# Recorded with VHS (https://github.com/charmbracelet/vhs) on a Mac with
# blessh installed, the server rns in range and its host key already
# known:
#
#   cd docs/demo && vhs blessh.tape
#
# Keys in the list follow the order in which servers are found; the tape
# presses 1, so it first waits for rns to be listed under 1. If another
# server comes first, the wait times out and VHS fails: record again.

Output blessh.gif

Require blessh

Set Shell "zsh"
Set FontSize 18
Set Width 1200
Set Height 620
Set Padding 24
Set Framerate 24
Set TypingSpeed 45ms
Set WaitTimeout 60s

Env LANG "en_US.UTF-8"

# A neutral local prompt, set before the recording starts.
Hide
Type "PS1='my-mac-or-linux $ '"
Enter
Type "clear"
Enter
Show

Sleep 1s

Type "blessh"
Sleep 400ms
Enter

# Pick rns once it is listed, after a few seconds in which other servers
# may join the list.
Wait+Screen@15s /\b1\s+rns\s/
Sleep 3.5s
Type "1"

# The shell prompt on rns.
Wait@60s /@rns:.*\$\s*$/
Sleep 1s

Type "grep Model /proc/cpuinfo"
Sleep 400ms
Enter
Sleep 2s

Type "vcgencmd measure_temp"
Sleep 400ms
Enter
Sleep 2s

Type "htop"
Sleep 400ms
Enter
Sleep 3.5s
Type "q"
Sleep 1s

Type "cowsay blessh just works"
Sleep 400ms
Enter
Sleep 3s

Type "exit"
Sleep 400ms
Enter
Sleep 4s
