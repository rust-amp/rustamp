# The ReplayGain filter and the sample rate

**Scope:** `crates/rustamp-meta` (the `replaygain` and `dsp` modules).

This note explains what the ReplayGain analysis of `rustamp-meta` measures
with, why it used to depend on the sample rate of the file it looked at,
and what it measures now.

## The short version

* The loudness filter is a pair of biquad sections whose reference design
  is given at 48 kHz.
* For every other rate the two sections are **retuned**: they are taken to
  the analog domain and realized again for the rate of the signal. The
  signal is **not** resampled any more.
* The 48 kHz numbers are **bit-identical** to the ones the analysis gave
  before.
* The same signal now measures the same at every rate. Before, a 20 kHz
  tone in a 44.1 kHz file measured **4.59 dB** louder than the same tone
  in a 48 kHz file; now the difference is **0.0002 dB**.

## What the ReplayGain 1.0 specification says

The classic proposal (David Robinson, "Replay Gain - a proposed standard",
2001-07-31) describes its loudness filter as

* a **tenth-order IIR filter designed by MATLAB's `yulewalk` function**, an
  approximation of the equal-loudness contour, in cascade with
* a **second-order Butterworth high-pass at 150 Hz**,

and publishes the coefficients of the two for **44.1 kHz and 48 kHz only**.
For every other rate the specification says: *"When using other sample
rates, coefficients must be transformed to maintain the same filter
response."*

Two things follow from that:

* The Butterworth part is fully specified in words - a second-order
  Butterworth high-pass at 150 Hz is an analog prototype. The published
  tables are exactly the bilinear transform of that prototype with the
  corner prewarped (verified here to 4·10⁻¹⁵ in the coefficients), so it
  can be designed for any rate.
* The `yulewalk` part has **no analog prototype in the specification** -
  only two coefficient tables. `yulewalk` designs its filter from a
  magnitude-response specification that the standard does not reproduce, so
  the equal-loudness filter cannot be derived from scratch for a rate the
  standard does not publish.

The rest of the proposal - the mono downmix, the 50 ms blocks, the RMS per
block, the 95th percentile, the 89 dB SPL pink-noise reference - is what
this crate implements.

## What this crate measures with

The crate has always measured with its **own** pair of sections, not with
the specification's tenth-order filter:

| Section | Numerator (48 kHz) | Denominator (48 kHz) |
|---|---|---|
| equal loudness | `0.995527971309, -1.991055942619, 0.995527971310` | `1, -1.991119453430, 0.991172964241` |
| high pass | `0.997862928247, -1.995725856494, 0.997862928247` | `1, -1.995789367305, 0.995789367305` |

The pair is a high-pass: the equal-loudness section has a gentle resonant
rise around 60 Hz (+0.32 dB there, -1.11 dB at 50 Hz), the high-pass
section rolls off below it, and the composite response is -2.2 dB at
60 Hz, -0.11 dB at 100 Hz and flat (0 dB) from about 150 Hz to the Nyquist
frequency. It is *not* the specification's
filter, and the module documentation used to call it "the standard Yule
shelf followed by a Butterworth high-pass", which was wrong.

Two consequences are worth stating plainly:

* The absolute gain of this filter is the crate's own: the analysis is
  anchored in the digital domain, so that **a full-scale sine wave
  measures 0.00 dB**. The specification's published `yulewalk` table has a
  passband gain of about **-45 dB**, so adopting the specification's
  filter as it stands would move every number the crate produces. That is
  a separate decision (see *What a separate decision would change*) and
  not part of this change.
* Because the crate's own filter is the one its tags were written with,
  its 48 kHz numbers are the reference this change keeps.

## The problem: the linear resampling

Until now the analysis resampled the signal to 48 kHz - the rate the
coefficient tables are given for - with a **linear interpolator**, and
measured the resampled signal.

That is wrong in two ways:

* A linear interpolator is a weak low-pass. Its response falls to about
  -4 dB at the Nyquist frequency of the *source*, so the top of the
  spectrum was attenuated before it was measured.
* It has no anti-aliasing. Content above the source's Nyquist frequency
  folds back into the band - a 44.1 kHz file's 20 kHz content landed at
  17.4 kHz in the 48 kHz signal.

Both effects hit the measured gain, and they grow with frequency. The same
one-second tone, measured with the old code:

| Tone | 44.1 kHz file | 48 kHz file | 96 kHz file | spread |
|---|---|---|---|---|
| 100 Hz | +0.114083 dB | +0.113936 dB | +0.113936 dB | 0.0001 dB |
| 440 Hz | -0.010067 dB | -0.012912 dB | -0.012912 dB | 0.0028 dB |
| 1 kHz | +0.012587 dB | -0.002103 dB | -0.002103 dB | 0.0147 dB |
| 4 kHz | +0.235458 dB | +0.000476 dB | +0.000476 dB | 0.2350 dB |
| 8 kHz | +0.937614 dB | +0.000597 dB | +0.000597 dB | 0.9370 dB |
| 16 kHz | +3.470441 dB | +0.000570 dB | +0.000570 dB | **3.4699 dB** |
| 20 kHz | +4.591210 dB | +0.000343 dB | +0.000343 dB | **4.5909 dB** |

A 44.1 kHz file of high-frequency material was measured up to 4.6 dB
louder than the same material at 48 kHz - the gain a player would apply
was wrong by that much, and the error grew with the frequency of the
material.

## The fix: the filter is designed for the rate of the signal

The sections are retuned for the rate of the signal, the same way the
K-weighting of the R128 measurement (`r128.rs`) is designed for the rate of
the signal:

1. The reference design is taken to the **analog** domain with the inverse
   bilinear transform at 48 kHz.
2. The analog prototype is realized again for the rate of the signal with
   the bilinear transform.

The analog transfer function - the shape of the filter, its cutoff
frequencies, its gain - is the same in both cases, so the retuned section
does to a signal what the original one does, only at the other sample
rate. This is what the specification asks for when it says that the
coefficients of other rates "must be transformed to maintain the same
filter response".

The signal itself is not touched any more, and the 50 ms blocks are
computed from the rate of the signal (`round(rate · 50 ms)` samples
instead of a fixed 2400 samples).

### The high-pass section is stored in its reduced form

The high-pass section of the reference design has the numerator
`0.997862928247 · (1 - z⁻¹)²` and the denominator
`(1 - z⁻¹) · (1 - 0.995789367305 z⁻¹)`: the two share the factor
`(1 - z⁻¹)` **exactly**, so the section is a first-order high-pass and is
stored that way.

The reason is numerical. The uncancelled pair puts a pole exactly on the
unit circle (at `z = 1`). Any rounding in the rate transformation could
push that pole outside the unit circle, and a filter with a pole outside
it grows without bound - the analysis of a long file would return
`inf`. The reduced form has its pole strictly inside, at every rate.

The reduction is exact, so the filter does the same as before; the test
`the_reduced_high_pass_section_filters_like_the_uncancelled_one` filters
the same signal through both forms and compares them.

## The numbers after the change

The same one-second tones, measured with the new code:

| Tone | 44.1 kHz file | 48 kHz file | 96 kHz file | spread |
|---|---|---|---|---|
| 100 Hz | +0.113933 dB | +0.113936 dB | +0.113948 dB | 0.000015 dB |
| 440 Hz | -0.012910 dB | -0.012912 dB | -0.012917 dB | 0.000007 dB |
| 1 kHz | -0.002104 dB | -0.002103 dB | -0.002104 dB | 0.000001 dB |
| 4 kHz | +0.000430 dB | +0.000476 dB | +0.000478 dB | 0.000048 dB |
| 8 kHz | +0.000602 dB | +0.000597 dB | +0.000612 dB | 0.000015 dB |
| 16 kHz | -0.000107 dB | +0.000570 dB | +0.000551 dB | 0.000677 dB |
| 20 kHz | +0.000578 dB | +0.000343 dB | +0.000342 dB | 0.000236 dB |

The largest spread over all rates and tones is **0.0007 dB** (a 16 kHz
tone, where the sampled sine at 44.1 kHz carries the most harmonic
distortion - a property of the test signal, not of the analysis). The
old spread was up to 4.59 dB.

## The 48 kHz equivalence

At 48 kHz the retuning is the identity: the design at 48 kHz *is* the
reference design, and the measured values are **bit-identical** to the
ones the analysis gave before the change.

The test `the_48_khz_numbers_are_the_ones_the_reference_design_gives`
pins them:

| Signal (48 kHz, one second) | Gain before | Gain after |
|---|---|---|
| 1 kHz, full scale | -0.002103164 dB | -0.002103164 dB |
| 440 Hz, full scale | -0.012911906 dB | -0.012911906 dB |
| 16 kHz, full scale | +0.00056985137 dB | +0.00056985137 dB |
| 440 Hz, half scale | +6.007688 dB | +6.007688 dB |
| 100 Hz, full scale | +0.113936365 dB | +0.113936365 dB |

The per-block values are pinned as well (the 1 kHz tone's blocks:
-0.0011514138, 0.0020970486, ... 0.0021031366 dB).

## Stability

The retuned sections have all their poles strictly inside the unit circle
at every rate from 4 kHz to 384 kHz
(`the_designed_filter_has_its_poles_inside_the_unit_circle`), and five
seconds of tone at every rate from 8 kHz to 384 kHz come out of the
analysis finite and on the same scale
(`the_filter_is_stable_at_every_rate`).

## What a separate decision would change

Adopting the specification's filter - the tenth-order `yulewalk` filter
plus the Butterworth high-pass - would change **every** number this crate
produces, and with it every `REPLAYGAIN_*` tag it has written. It would
also need a decision about the reference level, because the specification's
published filter has a passband gain of about -45 dB and the crate's
anchor is "a full-scale sine measures 0.00 dB".

The specification's equal-loudness filter has no analog prototype, so for
rates it does not publish (anything but 44.1 kHz and 48 kHz) it would have
to be transformed from its 48 kHz table - the same kind of transformation
this change uses for the crate's own sections.

That decision is not this change's business. This change fixes the rate
dependence and keeps every published number.

## Provenance

* The facts about the ReplayGain 1.0 specification (the description of the
  loudness filter, the published coefficient tables, the sentence about
  transforming the coefficients for other rates) come from the public
  "ReplayGain 1.0 specification" page of the Hydrogenaudio Knowledgebase,
  which summarizes David Robinson's proposal of 2001-07-31. No code and no
  wording was taken from it; the tables are numbers, and the filter
  mathematics is the textbook bilinear transform.
* No Winamp source, no SDK material, no disassembly and nothing from
  `rustamp-research` was read for this change.
* No third-party code was copied, so `THIRD_PARTY.md` is unchanged.
