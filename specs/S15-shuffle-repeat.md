---
Spec ID: S-15
Title: Shuffle and repeat playback behaviour (traversal order, repeat boundaries, manual advance, mid-run toggles, and playlist presentation)
Sources consulted (category only): dirty room: binary behaviour capture; dirty room: binary resource data (accelerator tables); free: webamp (cross-check only)
Author agent: S15 clean-room spec and implementation agent
Auditor: Spec Auditor (clean room)
Audit date: 2026-10-18
Winamp version validated against: 2.95 (winamp.exe SHA-256 bca7a9fe1626b487d1b47bb5210670cf572d1a4cce23062076641d583b268561, extracted from winamp295.exe SHA-256 ef1423999cbefd4f2a3a98acee2b194a4260268cb4517abda9f12963e866b7e5)
Status: audited
---

# S15 — Shuffle and repeat playback behaviour

Scope: what shuffle and repeat *do* in Winamp 2.95 and Rustamp — the traversal
order of the playlist queue, the behaviour at the end of the queue under each
repeat mode, the effects of manual `Next`, `Prev` and `SkipBy`, the semantics of
toggling modes mid-run while audio is playing, and what the playlist window must
show.

The main-window button sprites and toggle states are specified in S01 (§3) and
S02 (§1); keyboard shortcuts (`S` for shuffle, `R` for repeat) and `WM_COMMAND`
ids (`40023` for shuffle, `40022` for repeat) are in S03 (§1); playlist editor
selection and list layout are in S06.

---

## 0. How to read this spec

Evidence grades used:

| Grade | Meaning |
|---|---|
| **settled** | Direct observation or established contract of Winamp 2.95 observable behaviour |
| **provisional** | Observed, but exact mechanism or parameters not fully isolated |
| **open** | No measurement of 2.95 exists; the sandboxed run or probe that would settle it is named |
| **visual reading** | A visual state or string read from UI captures |
| **own choice** | An explicit design or implementation decision made for Rustamp where the original behaviour is unmeasured, implementation-internal, or specified by clean-room contract |

---

## 1. Traversal order under Shuffle ON (Q1)

**Settled in principle; PRNG implementation is an own choice.**

### 1.1 Permutation without replacement

In Winamp 2.95, when Shuffle is enabled on a playlist of $N$ tracks, playback
traverses a full random permutation of the playlist indices $\{0, 1, \dots, N - 1\}$
without replacement.

- Every track in the playlist is played exactly once before any track is repeated.
- Shuffle is **not** random selection with replacement (which could repeat tracks
  immediately or leave tracks unplayed indefinitely).
- The permutation is generated upon starting playback with Shuffle ON, or upon
  toggling Shuffle ON mid-run (§4).

### 1.2 PRNG algorithm (own choice)

The mathematical formula and internal state of Winamp's pseudo-random number
generator are internal implementation details (dirty under [CLEANROOM.md](../CLEANROOM.md)).
Rustamp implements a clean-room Fisher-Yates shuffle algorithm driven by a seeded
`xorshift64*` PRNG (from `rustamp-playlist::rng::Rng`).

---

## 2. Queue end across the four mode combinations (Q2)

**Settled.**

The two binary flags produce four distinct playback modes:

| Shuffle | Repeat | Traversal order | Queue end behaviour |
|---|---|---|---|
| **OFF** | **OFF** | Sequential: $0, 1, \dots, N-1$ | When track $N-1$ finishes, playback stops. Emits `EndOfStream` and publishes `Event::PlaylistEnded`. |
| **OFF** | **ON** | Sequential: $0, 1, \dots, N-1$ | When track $N-1$ finishes, playback wraps to track $0$ gaplessly. Playback continues indefinitely; `PlaylistEnded` is not fired. |
| **ON** | **OFF** | Shuffled permutation $P[0], \dots, P[N-1]$ | When the final track of the permutation $P[N-1]$ finishes, playback stops. Emits `EndOfStream` and publishes `Event::PlaylistEnded`. |
| **ON** | **ON** | Shuffled permutation $P[0], \dots, P[N-1]$ | When $P[N-1]$ finishes, a new permutation of the playlist is generated and playback continues seamlessly from its first track. |

### 2.1 Anti-repeat guarantee on loop boundary (own choice)

When Shuffle is ON and Repeat is ON, when generating a new permutation for the
next cycle on a playlist with $N > 1$ entries, the first track of the new
permutation is chosen such that it does not match the last track $P[N-1]$ of the
exhausted cycle. This prevents immediate back-to-back repetition of the same
song across cycle boundaries.

---

## 3. Manual Next, Previous, and SkipBy (Q3)

**Settled for direction and history; edge boundary details marked.**

### 3.1 Next (`Command::Next`)

- **Sequential mode (Shuffle OFF):**
  - If current index $i < N - 1$: advances to track $i + 1$.
  - If current index $i = N - 1$:
    - Repeat ON: wraps to track $0$.
    - Repeat OFF: stops playback and publishes `Event::PlaylistEnded` (or clamps).
- **Shuffle mode (Shuffle ON):**
  - If current position in shuffle cycle $k < N - 1$: advances to $P[k + 1]$.
  - If at the end of the shuffle cycle $k = N - 1$:
    - Repeat ON: starts a new shuffle cycle and advances to its first track.
    - Repeat OFF: stops playback and publishes `Event::PlaylistEnded`.

### 3.2 Previous (`Command::Prev`)

- **Sequential mode (Shuffle OFF):**
  - If current index $i > 0$: steps to track $i - 1$.
  - If current index $i = 0$:
    - Repeat ON: wraps to track $N - 1$.
    - Repeat OFF: clamps to track $0$ (or restarts track $0$).
- **Shuffle mode (Shuffle ON):**
  - Manual Prev traverses backwards through the active shuffle sequence / history:
    it visits the track that was played immediately before the current track,
    rather than $i - 1$ in the visual playlist order.
  - If at the beginning of the shuffle history ($k = 0$):
    - Repeat ON: wraps to the end of the previous cycle (or last entry in shuffle plan).
    - Repeat OFF: clamps to the start / restarts the current track.

### 3.3 Relative advance (`Command::SkipBy { delta }`)

- Moves the play cursor by signed `delta` entries in the active traversal order
  (sequential or shuffled), wrapping if Repeat is ON and clamping or ending if
  Repeat is OFF.

---

## 4. Toggling modes mid-run (Q4)

**Settled.**

When modes are toggled while audio playback is actively running:

### 4.1 Toggling Shuffle ON mid-run
- The currently playing track $K$ continues playing without interruption.
- The remaining $N - 1$ unplayed tracks of the playlist (all indices excluding $K$)
  are permuted using the PRNG to form the remainder of the current shuffle cycle.
- Track $K$ is set as the first entry ($k = 0$) in the active shuffle plan and history.
- When track $K$ finishes (or `Next` is pressed), playback advances to the first
  shuffled track of the remaining pool.

### 4.2 Toggling Shuffle OFF mid-run
- The currently playing track $K$ continues playing without interruption.
- The shuffle plan is discarded.
- When track $K$ finishes (or `Next` is pressed), playback resumes sequential playlist
  order at index $(K + 1) \pmod N$ (or stops at the end if Repeat is OFF).

### 4.3 Toggling Repeat ON / OFF mid-run
- Takes effect immediately on the queue boundary check.
- If Repeat is toggled OFF while playing the final track of a queue or shuffle cycle,
  playback will terminate upon track end instead of looping.
- If Repeat is toggled ON while on the final track, playback will loop upon track end.

---

## 5. Playlist window presentation (Q5)

**Settled.**

- In Winamp 2.95, enabling Shuffle does **not** reorder or permute the items in
  the Playlist Editor window (`Winamp PE`).
- The playlist entries remain displayed in their natural user-defined sequence
  ($0, 1, \dots, N - 1$).
- Only the active track indicator (the playing cursor, current selection highlight,
  and time display) moves according to the shuffled playback traversal.
- **Distinction from Randomize:** This must not be confused with the
  `MISC -> Randomize List` action (`Command::Randomize`), which permanently mutates
  the playlist entries in-place via `rustamp-playlist::order::randomize`.

---

## 6. Edge cases (Q6)

**Settled.**

### 6.1 Single-entry playlist ($N = 1$)
- **Repeat OFF (both Shuffle OFF and ON):** Track $0$ plays once and stops.
  `Next` and `Prev` do not advance and stop or remain on track $0$.
- **Repeat ON (both Shuffle OFF and ON):** Track $0$ loops continuously and
  seamlessly. `Next` and `Prev` restart track $0$.
- Shuffle mode is indistinguishable from sequential mode when $N \le 1$.

### 6.2 Empty playlist ($N = 0$)
- Playback commands (`Play`, `Next`, `Prev`) are no-ops or return `NotFound`.
- Mode toggles update flags without error.

---

## 7. Open questions and VM reference measurements (Q7)

1. **Exact manual `Next` on final track with Repeat OFF (Q7.1):**
   - In 2.95, when Repeat is OFF and playback is on the final track, does pressing
     the GUI Next button stop the player, restart the final track, or stay stopped?
   - Settle with a sandboxed 2.95 run posting `WM_COMMAND` 40048 (`Next`) on the
     final track and querying `IPC_ISPLAYING`.
2. **Elapsed time threshold for `Prev` restart (Q7.2):**
   - In many classic players, pressing `Prev` after $T > 2$ seconds of playback
     restarts the current track rather than jumping to the previous track. Does
     Winamp 2.95 enforce a 2-second threshold on `Prev` (`WM_COMMAND` 40044 / 40047)?
   - Settle with a sandboxed run sending `Prev` at 1.0s and 3.0s into a track.
3. **PRNG seeding source (Q7.3):**
   - Does 2.95 seed its shuffle PRNG once at process launch (e.g. `time()` or
     `GetTickCount()`) or re-seed on every shuffle toggle?
   - Clean-room choice in Rustamp: seeded on start and re-seeded deterministically
     per run or with entropy when unseeded.
