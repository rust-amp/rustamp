# Equalizer windowshade (wave194)

## Audit result

The equalizer **already shaded**; what was missing was the *shade-aware
input* and a way to reach the shade at all.

| Part | State on `main` before this lane | Source |
|---|---|---|
| Shaded size 275 x 14 | present (`docking::window_size(WindowKind::Equalizer, true, false)`) | S01 §1 (the shade height is 14 px for main, EQ and playlist); open in S05 §1.1, which never shaded the equalizer |
| Shaded pixels | present (`eq_ex.bmp` row 0 focused / row 15 unfocused) | S01 §3 (provisional) |
| Shaded input | **missing**: `EqualizerWindow::mouse_down`/`mouse_up` hit-tested the *unshaded* layout, so the On/Auto/Presets buttons and the eleven sliders answered clicks the shaded window does not show | S05 §1.2 |
| Reaching the shade | **missing**: `EqualizerState::windowshade` existed but nothing set it at runtime; the app answered `ToggleWindowShade` for the main window and the playlist editor only | RFC 0005 F2 |

## What the specs show in the shade

* **S07 has no shade wording at all.** Nothing in the audited equalizer spec
  shows sliders in the windowshade form.
* **S05 §1.2** settles for the windowshade in general: "the shade is a single
  14-px title-bar row: no body rows are drawn", and the strip is a distinct
  rendering, not a crop of the unshaded title band (settled for the main
  window by comparisons I and J).
* **S01 §3** gives the equalizer's shade band: `eq_ex.bmp` (0,0,275,14)
  focused and (0,15,275,14) unfocused — both *provisional*.
* **S05 §1.5** lists the equalizer's shade drawing as **open**: the equalizer
  was never shaded in the capture runs.

So the shaded equalizer is the **title strip alone**: the On/Auto/Presets
buttons and the sliders belong to the body, which the shade does not draw.
They are neither drawn nor hit-tested there.

The webamp cross-check that `rustamp-skin` carries names mini
volume/balance cells in the same `eq_ex.bmp` (rows 30-36) and three shade
strip buttons (rows 38-55). No spec settles *where* they sit in the strip, so
they are not drawn and not hit-tested — an implementation agent that needs
that fact files a spec request (CLEANROOM §5) instead of measuring it.

## What this lane implements

`crates/rustamp-ui`

* `equalizer::EqWidget::SHADE` — the widget list of the windowshade form:
  the title band alone (S05 §1.2).
* `equalizer::hit_test(layout, x, y, shaded)` — takes the shade flag; in the
  shade only the band answers. **Signature change**: the third argument
  `shaded: bool` is new, every in-tree caller is updated.
* `EqualizerWindow::set_windowshade(shaded)` — the shade flag becomes
  reachable, and a running slider drag is dropped with the body.
* `EqualizerWindow::mouse_down`/`mouse_up` — shade-aware, so no body control
  acts while the window is shaded. The keyboard path is unchanged (the window
  is still the equalizer, so `N`/`A`/`S` still act, S03 §3).
* `eq_render` — the shade branch now states its sources (S05 §1.2, S01 §3)
  and uses the shared `SHADE_HEIGHT`.

`crates/rustamp-app` (minimal)

* `Command::ToggleWindowShade(WindowId::Equalizer)` toggles the equalizer's
  shade — the same command the main window and the playlist editor already
  answer (RFC 0005 F2).
* The equalizer's shade state is written back into the configuration, like
  the main window's and the playlist editor's, so a shaded equalizer comes
  back shaded.
* The presenter's two equalizer hit-test call sites (cursor, title-bar drag)
  pass the shade flag.

No title-bar **shade button** was added for the equalizer: S01 §3 lists no
equalizer window buttons at all and S05 §1.5 says their positions are
unmeasured ("needs one of their title-bar button positions first"). The
title-bar **double-click** (S03 §5, verified in the reference) is not
implemented either — the player has no double-click gesture on any title bar
yet, and that is a presenter-level feature beyond this lane's minimal app
arm.

## Spec requests (dirty room)

1. The equalizer's title-bar button positions, so the shade can be toggled by
   a click (S05 §1.5, §9).
2. What the equalizer's shade strip draws besides the band, and where: the
   mini volume/balance cells and the shade strip's buttons that the webamp
   cross-check names in `eq_ex.bmp` (rows 30-55) (S05 §1.2 open).

## Tests

Headless, no window:

* `crates/rustamp-ui/tests/eq_shade.rs` — the shaded size (275 x 14 and the
  scaled 550 x 28), the shaded frame against the `eq_ex.bmp` band, the shaded
  frame against the unshaded render, no body controls in the shade, the
  shaded window answering only its band, and the unshaded path unchanged.
* `crates/rustamp-ui/src/equalizer.rs` — unit tests for the shade-aware hit
  test, the model answering neither the buttons nor the sliders in the shade,
  the unshaded path still answering, and a drag being dropped by the shade.
* `crates/rustamp-app/tests/config.rs` — the equalizer's shade round-trips
  through the configuration.

## VM-only

Nothing new: the shaded equalizer's *pixels* against the 2.95 reference need
a capture run (S05 §9's method), because S05 §1.2 never captured a shaded
equalizer. The headless tests here pin the skin's own artwork, not the
reference's.
