# S08 — Classic visualization: the spec to write, and how to measure it

**Status:** open. `specs/README.md` lists S08 as **not started**, and it is the
reason `rustamp-vis` is still a skeleton and the engine's vis tap publishes
*neutral* data (`crates/rustamp-core/src/vis.rs`: waveform as `f32` in `[-1, 1]`,
spectrum zero, "the exact Winamp derivation … is defined by spec S08").

## What S08 must fix (from clean sources only)

`docs/PLAN.md` §S08: *"Classic vis: spectrum computation, bar and peak falloff
speeds, analyzer modes, oscilloscope styles, refresh rate"*. The core contract's
own note names the same gaps: **8-bit scaling of the waveform, FFT size and
window, spectrum bin mapping and scaling, refresh cadence**. `specs/S09-plugin-abi.md`
adds the plugin-facing side: `vis_feed_pcm` (id 29), `analyzer_feed_pcm` (26),
`analyzer_get_mode` (27), `vis_get_mode` (30), `vis_feed_frame` (31) and
`vis_set_format` (32), and states that S08 is what "gives the numerical
processing" behind them.

So the spec has to settle, with evidence:

1. **The waveform path**: how many samples, their scaling (the plugin-facing
   8-bit form and our internal `f32` form), the sign convention, and what
   happens at the start of a track when fewer samples exist.
2. **The spectrum path**: the transform (FFT size, window function, overlap),
   the bin count and how bins map to bars, the magnitude scaling and the
   logarithmic mapping if any, and the value range.
3. **The analyzer modes** (`analyzer_get_mode` / `vis_get_mode`): which modes
   exist, what each one returns, and what the falloff/peak-hold does per mode.
4. **The cadence**: how often frames are produced and at what timestamp, and
   whether it follows the audio clock or a wall clock.
5. **The built-in window's rendering**: bar and peak falloff speeds,
   oscilloscope styles, and the refresh rate, as far as they can be measured
   from captures rather than from code.

## How it must be measured (dirty room, owner-approved 2026-10-17)

Two instruments, in this order:

- **A probe plugin (the primary instrument).** A 32-bit `vis_` DLL that the
  *reference* player loads and that logs exactly what it is handed:
  `vis_set_format`, every `vis_feed_pcm` / `analyzer_feed_pcm` call (the PCM it
  receives), `vis_get_mode` / `analyzer_get_mode` results, and `vis_feed_frame`
  payloads with their timestamps. That is the only way to see the numbers
  directly; it needs the plugin ABI facts S09 already audited, and the probe
  itself is our own code, so nothing dirty is read to write it.
- **Captures (the secondary instrument).** The built-in analyzer and
  oscilloscope drawn in the reference's own window, captured frame by frame,
  for the falloff speeds, the peak hold and the refresh rate — the parts that
  are *visual* and have no plugin-facing number.

**Clean-room rules that bind this work.** The reference may be **run and
observed**, and its own window may be captured. It must never be read: no
disassembly, no SDK headers, no source, and nothing from the dirty room may be
copied into `specs/` except measured numbers with their provenance. The spec is
the only artefact that crosses back, and every value in it carries the run it
came from.

## Staging

1. **Design and probe (a dirty-room lane, no GUI run).** Read the harness and the
   reference installation, decide exactly what is measurable and how, write the
   probe DLL, write the capture jobs, and draft `specs/S08-vis.md` with every
   value left as an explicit placeholder plus the probe that would fill it.
2. **Run (the orchestrator).** Execute the jobs in the headless test VM, feed
   the artefacts back, and have the lane fill the placeholders.
3. **Audit.** An independent lane re-derives each number from the run's own
   bytes before the spec is marked audited — the same rule every other S-spec
   went through.

## Acceptance

`specs/S08-vis.md` exists, every value in it traces to a named run, the probe
plugin and the jobs are in the tree with their provenance, and an auditor has
re-derived the load-bearing numbers from the artefacts rather than from the
report.
