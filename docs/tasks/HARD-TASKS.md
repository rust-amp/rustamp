# The hard list

Owner instruction (2026-10-17): remember the things other models struggle with,
and do those with Codex. "Codex" here means the newest OpenAI model the catalog
offers (`openai/gpt-6.1-sol`, variants up to `xhigh`): no model in the catalog is
named codex. Owner rule for that source: spend until Sunday 2026-10-18, then stop
until approval.

## 1. Where the sources themselves fail (the log)

| Source | What happened |
|---|---|
| Muse (`opencode-go/muse-spark-1.3-contributor`) | Ran the packaging lane to an **empty worktree** and ended with no text at all: silent death. The same family delivered two good lanes (decoder resident, tray) the same evening. |
| Claude (`claude-balanced`, Claude Max) | `rate_limit_error` twice in one evening: the plugin-DLL lane and the packaging lane both died on it. |
| Free tier | `free-ling-3.1`, `free-muse-spark`, `free-step-5` unavailable; `free-laguna` transient error; `free-nemotron-ultra` 503; `free-deepseek-v4.1` once finished **silently with no output**; `free-big-pickle` and `free-nemotron-lightning` worked. |
| agy (Google) | Reliable, but **four concurrent jobs** is a hard cap, and its job API hands back an empty result text: the worktree is the evidence, not the report. |
| OpenAI | Worked before (`gpt-6.1-sol`); unused so far this wave. |
| Subagents generally | A subagent lane **notifies**; an agy lane does not. Finished agy lanes sat unnoticed until the poll rule was written down. |
| A subagent lane (ReplayGain, free tier) | Wrote into the **canonical tree** instead of its own worktree - one no-op edit to `crates/rustamp-meta/src/write.rs`, both branches doing the same thing - while its worktree stayed untouched. Reverted; the canonical tree is clean again. Rule: check a lane's worktree, not only its report, before believing either. |

## 2. The hard engineering, where models demonstrably got it wrong

1. **The ring drain and the clock re-base (A8).** A predecessor failed in the VM
   because `clock_origin` was set before the drain, so the position ran one
   callback ahead: the 64-frame settle pump. It took a retry lane and an
   adversarial review that reproduced three VM failures by tracing.
2. **The audio DSP correctness.** An adversarial review of that wave concluded
   *three claims hold, four do not*. Still open and named: the equaliser cannot
   stop the resampler from rebuilding peaks.
3. **The equaliser law (RFC 0011).** The dB-to-classic mapping had to become
   injective over the measured anchors, and the renderer had to be rebuilt with
   it (18 golden images).
4. **The engine lifecycle (F1/F2).** Finish a run on a *drained ring*, not on the
   device clock; count consumed frames on the consumer side. Both were wrong in a
   way that only a VM run showed.
5. **The out-of-process hosting (M10/M12/M13).** Handshake, a sticky end of
   stream only after the drain, a loud refusal before any ring, a blocked writer
   woken when the player dies. One test is still flaky because it waits on a
   wall-clock bound instead of a condition.
6. **The device-error hook (RFC 0012).** Landed as a polled accessor; what is
   left needs real hardware.
7. **The plugin DLL path.** 32-bit FFI, wrong architecture, hostile DLL, and an
   acceptance test that can only run in the VM.
8. **The single instance and the CLI forwarding.** A named mutex and a pipe
   across processes, with two lanes touching one file.
9. **The wake timeout measurement.** Honesty about the kernel timer, not a guess.
10. **The reference capture / OCR / VM harness.** Session 0 versus the
    interactive session, `subst`, ejecting the install DVD, pinning the display.
11. **The visualiser (S08).** Unmeasured; the probe plugin and the captures are
    the hard part, and the rendering is a skeleton until they exist.
12. **ReplayGain.** Tags read and written losslessly, analysis as a pure
    function, and the gain must actually reach the chain.
13. **Shuffle and repeat (BETA-08).** Blocked on S15.
14. **The process, not the code.** A lane may not run tests, so every new test is
    an unverified claim: seven failures in one batch came from exactly that.

## 3. What goes to Codex

Owner confirmation (2026-10-17): the OpenAI GPT-6 models **are** Codex. The
source for these lanes is therefore `openai/gpt-6.1-sol` with variant `xhigh`.

Running now, on the adversarial review's proven findings:

1. **The plugin loader race** (`wave57-codex-plughost`): the loader preflights one
   set of bytes and then loads the file again by path, so a hostile or
   wrong-architecture library can still reach its entry point (W10 F2, F8).
2. **The flush contract** (`wave57-codex-flush`): the cpal backend's flush does
   nothing, so the stale-replay fix does not hold there, and the engine flushes
   even when a seek failed, dropping audio that was still playable (W10 F3, F4).
3. **The host concurrency findings** (`wave57-codex-mediahost`): the test that
   waits on a five-second bound, a reset that does not wake a blocked reader, the
   truncated-file contract, and the prose-only gaps (M14 F2, F3).

Taken by the orchestrator rather than a lane, because it was small, sharp and a
beta blocker: **the configuration wrote a window id its own loader refuses**
(W10 F1) - fixed on the writer through serde, with a round-trip test that uses a
generic window (`c32cf7b`).

### Queued after these

1. **The DSP stage correctness and robustness** (`rustamp-dsp`, free). The stage
   must not let a boost come back as a peak it cannot bound; numeric tests over
   the measured anchors; a defined NaN, denormal and DC policy; and a written
   report of what the engine side must do about it.
2. **The decoder side** (`rustamp-decoders`, free). A real second factory so RFC
   0013's tie and refusal rules are reachable with two decoders rather than a
   test factory, plus hostile-input hardening.
3. **Then, as the crates free up**: the flaky mediahost test on a condition
   instead of a bound; the engine-side resident adapter; RFC 0014 once the owner
   decides; the S08 measurement; the ReplayGain wiring.

Rule of thumb for the next waves: anything where the **ordering** of threads,
clocks or buffers decides correctness, anything numeric and easy to be subtly
wrong about, and anything whose test needs a second process or real hardware is
Codex work, not free-tier work. Free-tier lanes are for well-specified, local,
checkable-by-reading changes.
