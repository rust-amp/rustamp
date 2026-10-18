# M31 — Wave89 merge reds: decoder open helper vs Ping, xtask layout hardening

**Date:** 2026-10-18. **Lane:** `wave89-merge-reds`.
**Worktree / write boundary:** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave89-merge-reds`
(write ONLY here; never touched `D:\code\Rustamp`, `D:\code\rustamp-research` or `D:\code\rustamp-refbin`).
Owned paths used: `crates/rustamp-mediahost/**`, `xtask/**`, this card.
`crates/rustamp-audio`, `crates/rustamp-playlist` and every other crate were NOT touched.

**Status: fixed by inspection, gates green on host, VM run pending.**
`cargo test` was NOT run anywhere (Hyper-V orchestrator rule); only
`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`
and `cargo build --workspace` ran (receipts in §5). No download, no `cargo add`,
no new dependency, no git write, no window, no audio.

Sources (read-only except the owned paths above): `AGENTS.md`,
`docs/tasks/M28-rfc0014-impl.md` (fallible stage contract + its wave-83 merge record),
`docs/tasks/M27-reset-fence.md` (reset acknowledgement), `docs/tasks/X10-release-zip.md`
(portable archive + licence inventory + extended layout check),
`crates/rustamp-mediahost/src/decoder.rs`, `crates/rustamp-mediahost/src/control.rs`,
`xtask/src/package.rs`, plus git history of the same tree.

---

## 1. Mediahost red — verdict: the expectation was wrong, the code is right

**Test:** `decoder::tests::decoder_session_open_fails_sticky_as_disconnected_when_server_reports_decoder_error`
(`crates/rustamp-mediahost/src/decoder.rs:758`, added by M30 `1e8c0fe`).

**Guest evidence:** worker thread panics at `decoder.rs:774`
(`assert!(matches!(open_msg, Message::DecoderOpen { .. }))`), test then panics
at `decoder.rs:789` (the `Disconnected` arm's message assert).

**Mechanism (file:line):**

- `ControlPeer::start` (`control.rs:125-127`) queues one `Hello` before returning,
  and its monitor sends a `Ping` as soon as the handshake completes
  (`control.rs:148-157`: `hello && pending.is_none() && now >= next_ping`, with
  `next_ping` initialised to start time, so the first Ping fires within
  microseconds of the server's `Hello` arriving).
- `DecoderSession::open` (`decoder.rs:297-300`) sends `DecoderOpen` right after
  `start` returns. That send races the monitor's Ping down the same
  `outgoing` queue — one `thread::spawn` plus a mutex lock versus two thread
  hops (reader + monitor) plus `mpsc` delivery.
- The test helper (raw `StreamTransport`, no monitor) asserted the **second**
  server-side frame is `DecoderOpen`. When the Ping wins the race the helper
  panics at :774, never sends `DecoderError`, the pipe dies with it, and the
  client's `open` then fails with `Disconnected("control stream ended: …")`
  instead of the decoder text — hence the second panic at :789. Single root
  cause, downstream symptom.
- Under a starved guest scheduler (cf. `bdd9b68`: binaries run serially because
  the guest starves) the main thread is descheduled right after `spawn` while
  the reader/monitor threads run, so the Ping wins all but deterministically.

**Was the merge the cause? No — no merge defect in this path.**
`decoder.rs` was last changed by M30 (`1e8c0fe`, which also added this test);
`control.rs` by `0a22649`. Neither RFC 0014 (`6ec4cb8`, mediahost `dsp.rs` only)
nor RFC 0015 (`408863c`, `dsp.rs`/`lib.rs`/plughost only) touched either file,
and no merge ever conflicted in them. The strict "second frame is DecoderOpen"
assumption was already racy at the test's birth — the Ping behaviour predates
both lanes — and neither lane's merge record claims otherwise. Production is
unaffected: real residents answer Pong through their own monitor, and Ping/Pong
never reach a session's incoming queue.

**Fix (`decoder.rs:772-786`, test helper only, no production change):** the
server thread now skips `Ping`/`Pong` liveness chatter in a loop until
`DecoderOpen` arrives, and still panics on any *other* unexpected first message,
so a genuinely wrong first frame fails as loudly as before. The test's claim —
open fails sticky `Disconnected` carrying `resident decoder failed: …` — is
untouched; no assertion on it was weakened.

## 2. xtask red — verdict: layout logic is self-consistent in-tree; hardened two latent defects

**Test:** `package::tests::layout_check_rejects_missing_guide_stale_crt_and_wrong_licences`
(`xtask/src/package.rs:2181`, introduced by `f570f1d`, extended by X10 `fa1dc77`).

**Finding:** by exhaustive inspection every fixture write in this test derives
from the exact same function and arguments the audit regenerates
(`quickstart`/`readme`/`build_top_manifest`/`serde_manifest_player`/`serde_manifest_host`/`player|host_third_party_notices`
with `profile="release"`, the same `(player, host)` flags, the same `meta`;
root `THIRD_PARTY.md` is copied verbatim and re-read with no writer in between;
the player/host/root file lists match entry-for-entry, including the X10-added
`THIRD_PARTY.md` files, `README.md`, residents, bundled host, and per-package
`QUICKSTART.md`). Every negative case (missing guide/README/notices, stale
`vcruntime140.dll`/`unexpected.exe`/`LICENSE-GPL`, wrong-side licence,
`b"dynamic CRT"` manifest) deterministically yields `Err` from either the
unexpected/missing-entry check or the `audit_text` mismatch. There is **no
deterministic failure in this tree**: the lane updated fixture and audit
atomically (`fa1dc77`), no later commit touched `package.rs`, and the VM's
`main` (`bc828a7`) differs from this HEAD only in tray files.

**Consequence:** the VM red cannot be a stale-expectation-vs-new-layout
mismatch in the current bytes. The remaining candidates are environmental
(Windows temp-dir/AV file locking under a loaded guest, parallel-run
interference) or a misattributed location inside the xtask binary. That
localisation needs the guest's panic line, which the report did not include.

**Hardenings applied (no claim weakened, in-tree behaviour identical):**

- `package.rs:2285-2291` — the `THIRD_PARTY.md` restore after the missing-file
  case is now guarded by `is_file()` with the dummy fallback, mirroring the
  fixture setup at :2203. Previously the setup tolerated an absent repository
  file but the restore `copy_file(...).unwrap()` did not — a latent panic for
  a case the lane author already considered possible.
- `package.rs:2300-2303` — final `remove_dir_all(base).unwrap()` is now
  best-effort (`let _ = …`). Cleanup runs after every claim has passed or
  failed on its own merits; a transient Windows file lock must not flip a
  green test red. Unique temp names keep any leftover harmless.

## 3. What only the VM can prove

- Both reds green: the mediahost decoder binary (the Ping race is
  scheduler-dependent; the 5-run repeat discipline of M26 §5 applies) and the
  xtask binary (including the exact panic line of the reported red, to confirm
  §2's localisation — if the line is inside the layout test's asserts rather
  than cleanup/fs, this lane's conclusion must be revisited).
- No regression in the rest of the `rustamp-mediahost` and `xtask` suites.
- Real exit codes / timing margins are unchanged: no production code was
  modified in either crate.

## 4. Files and line counts

| File | Lines (after) | Change |
|---|---|---:|
| `crates/rustamp-mediahost/src/decoder.rs` | 807 | test helper skips Ping/Pong before DecoderOpen (+13/−2 in the helper) |
| `xtask/src/package.rs` | 2614 | guarded THIRD_PARTY restore, non-fatal temp cleanup (+9/−2, test only) |
| `docs/tasks/M31-merge-reds.md` | this card | new |

No `Cargo.toml`/`Cargo.lock`/`THIRD_PARTY.md` change. No production (non-test)
line changed in either crate.

## 5. Gate receipts (all inside this worktree)

- `cargo fmt --all -- --check`: **exit 0** (final check after the edits).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: **exit 0**
  (one finding fixed on the way: unused binding in the rewritten helper).
- `cargo build --workspace`: **exit 0** (final build after the edits).
- `cargo test`: **NOT RUN** (not the suite, not `--no-run`, not a single binary).
- No download, no `cargo add`, no dependency, no install, no commit, no git write,
  no window, no audio, no VM touched. Forbidden trees never accessed.

## 6. Boundary receipt

`git status --short` (proof elicited after writing this card) lists exactly the
two edited files plus this card, and nothing else:

```
 M crates/rustamp-mediahost/src/decoder.rs
 M xtask/src/package.rs
?? docs/tasks/M31-merge-reds.md
```
