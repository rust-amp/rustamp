# Spec request: the stream status in the main window's title scroller (S19 amendment)

Filed 2026-10-24 by the orchestrator, as an **audit template**. This is a
request to the dirty room and to the auditor ([CLEANROOM.md](../../CLEANROOM.md)
§2, §3), not an implementation. The implementers stay clean: nothing below may
be built until an auditor has passed it and it is exported to
`specs/S19-streaming.md` (or a successor section).

## What is established (observation, in our own words)

Source: three measurement runs of the reference player in the Hyper-V test VM
(2026-10-21, 2026-10-24 twice), a local HTTP server that paces the body so the
transfer lasts about 30 s, and PNG captures of the main window at 2 Hz and at
about 10 Hz. Details and method: `docs/notes/streaming-display-measurement.md`
(sections "Third attempt" and "Fourth attempt"). The raw frames stay in the
dirty room's staging (`D:\code\rustamp-vm\staging\out\transfer-normal\`).

All of it concerns the **main window's scrolling title text** (the green
marquee, 275 x 116 window), for a playlist entry whose source is an `http://`
URL on the loopback address with a known `Content-Length`:

1. **First frames after the window exists:** the marquee reads the playlist
   index, a period and the **bare URL** (`1. HTTP://127.0.0.1:<port>/<path>`).
   No bracketed prefix.
2. **From the next frame** (about 0.1-0.2 s later) to the end of the sampled
   window (about 2.4 s at 10 Hz): the text is `1. [READING ID3] ` followed by
   the URL. The URL is the title for as long as nothing better is known.
3. **Later (2 Hz run):** `1. [BUFFER: n%] STREAM.MP3 (0:02)`, with `n`
   counting up (0, 22, 65 were seen) while the readouts already show the
   stream's rate and frequency (`128 kbps`, `44 kHz`). The title is then the
   file name and a **length in parentheses**, `(m:ss)`.
4. **A re-buffer** was seen: `[BUFFER: 0%]` again while the time display shows
   `00:02`, i.e. the percentage can fall back to 0 after playback began.
5. **After the transfer and the end of the track:** the prefix is gone:
   `1. STREAM.MP3 (0:02)`, the readouts are blank, the player is stopped.
6. The prefixes are **upper case**, in brackets, and sit **between the
   playlist index and the title**. They scroll with the rest of the line.
7. **Not seen in the marquee** at 0.1 s resolution in the first 2.4 s, or at
   0.5 s resolution later: any of S19 §2's texts (`Connecting to host...`,
   `Sending HTTP request`, `Reading HTTP response`, `Retrieving file...`,
   counters). Window titles (the OS title bars) only ever changed by the
   playlist index, the title and a `[Stopped]` suffix.

## What this contradicts or leaves open

* S19 §2 lists ten status entries as **string facts** and says the order in
  which they are shown "was not observed" (§6, open question 15). The run
  shows the bracketed prefixes **instead of**, or **in addition to**, those
  texts, in a place S19 never named. Where S19 §2's texts are drawn (or whether
  they are drawn for a fast local connect at all) is **still unknown**.
* S19 §6 open question 6 (where the stream title appears) is **answered for
  the title text**: in the marquee and in the main window's OS title bar
  (prefixed with the playlist index).

## Questions the next measurement must settle

1. The complete set of bracketed prefixes and the phase each one names. Seen:
   `[READING ID3]`, `[BUFFER: n%]`. Not yet looked for: a prefix for a refused
   connection, a lost connection, or a server that closes early (the `cut` and
   `refused` runs end the process within about 5 s with no frames, which is a
   finding of its own: **what does the reference do on a refused connect?**).
2. Which event ends `[READING ID3]` and starts `[BUFFER: n%]` (response
   headers, the first bytes, the end of the tag)?
3. How `n` is computed (share of a fixed buffer, or of the total) and how often
   it updates.
4. Whether the two-line counters text of S19 §2 appears for a **slow connect**
   (a server that waits several seconds before answering), a **chunked or
   length-less** response, or through a proxy. Use a server delay of 3 s before
   the headers, and capture from the process start at >= 10 Hz.
5. Whether the title becomes `<file name> (m:ss)` from the stream's own
   metadata, and what a stream with an `icy-name` or an ID3 title shows.
6. The marquee's start position, scroll step and period while the text is
   shorter than the marquee (the first frames suggest a long URL fills it).

## What our player does today

Nothing of this: the scroller shows the title only, with no prefix, and there
is no status text for a running transfer (`grep` of the crates and `specs/`
for `READING ID3` and `BUFFER` finds nothing). The stream is added through the
location dialog (Ctrl+L) and plays; only the status feedback is missing.

## Audit notes

Contains no code and no text copied from the reference's binaries or SDK: the
bracketed words are what a screenshot shows. The auditor should confirm that,
and decide whether the prefixes belong in S19 §2 or in a new section on the
marquee (S02 owns the marquee).

## Update, same day: the runs that were asked for (slow, refused, cut)

New harness modes (`measure-transfer.ps1`): `slow` (the server waits 3 s before
the headers), `refused-run` and `cut-run` (the player starts **without** the
URL and is handed the URL once its windows exist, because with the URL on the
command line a refused or cut stream ended the whole process within ~1.5 s and
left nothing to observe). Frames at ~10 Hz plus the OS title of every window
at every step (`inner-transcript.txt` of each job).

**The same status words appear in the OS title bar**, in a different case, as
a bracketed prefix between the index and the title: `1. [Connecting]
http://...`, `1. [reading id3] http://...`, `1. [Buffer: N%] stream.mp3`.
So claim 7 above ("window titles only ever changed by...") was **wrong** and
is withdrawn: the earlier title polling had simply run too late.

Per mode, in order (distinct states, 10 Hz):

| mode | states seen |
|---|---|
| normal | bare URL (~0.2 s) -> `[reading id3]` -> `[Buffer: N%]` (the long phase) -> plain title -> `[Stopped]` suffix |
| slow (3 s before the headers) | `[Connecting]` (about the 3 s of the delay and more) -> `[reading id3]` -> `[Buffer: N%]` -> plain title -> `[Stopped]` |
| refused (handed to a running player) | `[Connecting]` (~1.5 s) -> `[could not connect]`, which **stays**; the entry stays in the list and the player is `[Stopped]`. The marquee shows `[COULD NOT CONNECT]` in capitals with the URL, and repeats with a ` *** ` separator |
| cut (10 % of the body, then close) | `[Connecting]` -> `[HTTP/1.1 200 OK]` which **stays** for the rest of the run: the server's **status line** is the prefix once the transfer ended early |

New facts:

* **Marquee separator:** the scrolling line wraps with a gap of ` *** `.
* **Case:** the marquee shows the prefixes in capitals (the skin's font is
  capitals-only); the OS title bar keeps the original case. The original case
  is `Connecting`, `reading id3`, `Buffer: N%`, `could not connect`,
  `HTTP/1.1 200 OK` (the last one is the server's own text).
* **Command-line vs running:** a stream given on the command line that refuses
  the connection **ends the player's process within ~1.5 s**, with no window
  left to read (also for the cut case). Handed to a running player, the same
  streams leave the player alive and show the prefixes above.
* S19 §2's ten texts are **still not seen** anywhere, even with a 3 s server
  delay; the prefixes are what the reference shows while a stream runs.

Answers to the questions above: 1. partly - set seen is {`Connecting`,
`reading id3`, `Buffer: N%`, `could not connect`, a server status line}; a
lost connection mid-play is not yet exercised. 4. answered for a slow connect
(no §2 text). 2, 3, 5, 6 remain open.

The frames and transcripts stay under `D:\codeustamp-vm\staging\out	ransfer-*`.
