# Third-party code and assets

Rustamp is licensed under GPL-3.0-or-later. Every piece of code or asset taken from another project must be listed here, and its original license header must be kept in place (see [CLEANROOM.md](CLEANROOM.md)).

| Component | Source | License | Used in | Notes |
|---|---|---|---|---|
| webamp classic-skin tables | [captbaritone/webamp](https://github.com/captbaritone/webamp), commit `79117c66d79fdac690fc732a0ce8df543ccd97a0` | MIT | `crates/rustamp-skin/src/webamp.rs` | The sprite rectangles (`packages/webamp/js/skinSprites.ts`), the `text.bmp` glyph grid, the cursor name list, the default analyser colours and the default playlist colours (`packages/webamp/js/baseSkin.json`, `constants.ts`), rewritten in Rust. The MIT notice is at the top of the module. Every ported value carries a `// provisional: webamp, pending S01/S17` marker. |

Projects we expect to draw from: DeaDBeeF (zlib / GPL / LGPL), webamp (MIT), butterchurn (MIT), WDL/EEL2 (zlib), libopenmpt (BSD-3-Clause).

Not bundled: the original Winamp base skin. It is distributed separately as an optional download, and it is used in tests.
