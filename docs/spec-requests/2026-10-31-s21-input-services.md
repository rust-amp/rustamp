# Spec request: what a Winamp 5.x input plug-in needs in order to play (S21)

Filed 2026-10-31. **A request to the dirty room, not work.** Implementers stay
clean ([CLEANROOM.md](../../CLEANROOM.md) §2, §5): nothing below may be built until
the dirty room has described the behaviour in its own words, an auditor has passed
it and it is exported to `specs/`.

## Why

Input plug-ins now play through the player's engine
(`crates/rustamp-plughost/src/input_resident.rs`). `in_openmpt` does, measured on
the VM's virtual cable. The Winamp 5.66 `in_wave.dll` loads and runs `init`, then
faults in `play`: an access violation reading address 0. Its trace shows one
service lookup the host answers "unknown" during `init`, with the identifier bytes
`BE F8 FB AE AA E0 18 43 8C C1 43 53 41 0B 64 DC`.

`tests/loopback.rs::a_failing_input_plugin_leaves_the_player_playing` records what
`in_wave`, `in_flac` and `in_vorbis` do on each run.

## What is asked

1. Which service that identifier names, and what a plug-in calls on it.
2. Whatever else a 5.x input plug-in's `play` needs of its host in order to reach
   its output module.

## What must stay true

A plug-in that faults faults in its own helper process, and the player plays on.
That is pinned by the test named above and holds with or without this work.
