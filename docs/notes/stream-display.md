# The main window's display for a stream (S19)

The engine can play an `http://` stream (`wave319`) and refuses a seek on it.
This note states what the **main window** shows for such a track, so the app
lane and the next lane do not have to re-derive it.

Everything here is implemented in `crates/rustamp-ui` (the behaviour machine
and the renderer) and fed by `crates/rustamp-app`. Nothing in `rustamp-audio` or
`rustamp-core` changed: the window learns the facts from the events it already
receives plus **one** extra fact the app hands over.

## The three rules

1. **A track that cannot be seeked offers no seek.** The position slider is
   drawn in its disabled look, a press on it emits **no** command, the thumb
   does not move, and the S02 hint `SEEK TO: …` never appears.
2. **A track whose length is unknown has no remaining time.** The time display
   shows the placeholder instead of a `00:00` that would read as "the track is
   over". The **elapsed** time still counts - it is known.
3. **The bar and the position text never name a target the track cannot
   reach.** A stream has no percentage of anything, so its bar shows no target
   and the hint stays silent.

## The placeholder

The remaining-time display of a track with an unknown length shows **the
skin's own dash and nothing else**: the `MINUS_SIGN` sprite at (38, 32) - the
mark the remaining display already carries - and **no digit cells at all**.

*Why:* S19 has no text for the unknown length (its open question 7 asks what
the reference shows without a length), so the placeholder is ours. A dash is
the skin's own "remaining" mark, so it costs no new art and it is the one
thing the display can say that is true. The cells are left empty on purpose:
the classic "no digit" filler is a *dimmed* `88` in the look-alike skin, and
four of those would read as a time - the very thing the placeholder is there
to avoid.

The elapsed display is untouched: it shows the elapsed time as before, and it
keeps counting.

## The disabled look

The classic skin carries **no** "disabled" sprite for the position slider, and
S19 leaves the reference's own display open, so the look is **Rustamp's own**:
the skin's own thumb sprite, drawn at **38 % opacity** (`DISABLED_ALPHA = 96`)
over the track. The control keeps its shape and reads as inactive. The shaded
window's position bar carries the same look.

The disabled look is a *drawing*; the slider is also *inert*, so a user cannot
grab a target the engine will refuse.

## What the app must feed

The engine owns the fact (`Engine::is_seekable`). The app hands it over:

```rust
// crates/rustamp-app/src/app.rs
fn sync_seekable(&mut self) {
    self.ui.set_seekable(self.engine.is_seekable());
}
```

It is called when a track opens (`Event::TrackChanged`) and when the app
resyncs from a `PlayerState` snapshot. The engine sets its own flag *before*
it publishes `TrackChanged`, so the value is already the new track's.

The **length** needs no extra feed: the window already learns it from
`Event::TrackChanged { duration_ms }` and `Event::PositionChanged`.

## What is *not* here (and who owns it)

* **The S19 status chain** (`Connecting to host...`, `Retrieving file...`,
  …) is **not** owned by the UI model: the main window has no status surface
  of its own - the marquee is the S02 track title, the time display is the
  S02 time, the kbps/kHz fields are the S02 stream info - and S19 does not
  say where the reference shows the text either (its open question 15 is about
  the order). The app lane owns the decision where a stream's phase shows up.
* **The keyboard's five-second jump** (`Left`/`Right`, `Num7`/`Num9`) still
  asks the engine, which refuses a stream seek and reports it. That is the
  right answer for a key the user pressed deliberately: the error tells the
  user *why* nothing happened, while the slider is a control the window
  itself offers. S03 fixes the key's action; changing it is a product call.
* **The cursor** over a disabled position slider is still the position cursor
  (`crates/rustamp-app/src/window.rs`, `apply_main_cursor`): the cursor is
  chosen by the widget under the pointer, and a disabled widget is still the
  widget. Cosmetic; the GUI shell's business.
* **A seekable track with an unknown length** (a file whose container states
  no duration) keeps today's behaviour: the thumb moves, the hint names the
  S02 total as `0:00`, and no seek is emitted (the length is needed to
  compute a target). Tying the slider to the length as well would change the
  "nothing loaded" frame and the goldens, so it stays a reported gap.

## The goldens

No existing golden moved: a seekable track renders exactly as before. Four
**new** goldens pin the new display:

| file | what it shows |
|---|---|
| `tests/golden/lookalike/main_window_stream.png` | the stream in the look-alike skin: the disabled position bar, the elapsed time still counting |
| `tests/golden/lookalike/main_window_stream_remaining.png` | the same with the remaining time: the dash, no digits |
| `tests/golden/rust-theme/main_window_stream.png` | the same state in the shipped default skin |
| `tests/golden/rust-theme/main_window_stream_remaining.png` | the remaining time there |
