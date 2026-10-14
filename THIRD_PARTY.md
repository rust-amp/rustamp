# Third-party code and assets

Rustamp is licensed under GPL-3.0-or-later, except for the plugin-facing crates
(see "Licensing of Rustamp itself" below). Every piece of code or asset taken
from another project must be listed here, and its original license header must be
kept in place (see [CLEANROOM.md](CLEANROOM.md)).

| Component | Source | License | Used in | Notes |
|---|---|---|---|---|
| webamp classic-skin tables | [captbaritone/webamp](https://github.com/captbaritone/webamp), commit `79117c66d79fdac690fc732a0ce8df543ccd97a0` | MIT | `crates/rustamp-skin/src/webamp.rs` | The sprite rectangles (`packages/webamp/js/skinSprites.ts`), the `text.bmp` glyph grid, the cursor name list, the default analyser colours and the default playlist colours (`packages/webamp/js/baseSkin.json`, `constants.ts`), rewritten in Rust. The MIT notice is at the top of the module. Every ported value carries a `// provisional: webamp, pending S01/S17` marker. |

Projects we expect to draw from: DeaDBeeF (zlib / GPL / LGPL), webamp (MIT), butterchurn (MIT), WDL/EEL2 (zlib), libopenmpt (BSD-3-Clause).

Not bundled: the original Winamp base skin. It is distributed separately as an optional download, and it is used in tests.

## Licensing of Rustamp itself

RFC 0004 splits the license. The player keeps its copyleft; the crates that
face plugins are permissive, so that no copyleft reaches a plugin of any
license, and a GPL-3.0 section 7 additional permission covers the player's own
code on the player ↔ plugin-host edge.

| Part | Crates | License |
|---|---|---|
| The player and every other crate | `rustamp-core`, `rustamp-audio`, `rustamp-decoders`, `rustamp-dsp`, `rustamp-output`, `rustamp-meta`, `rustamp-playlist`, `rustamp-skin`, `rustamp-gfx`, `rustamp-ui`, `rustamp-vis`, `rustamp-milkdrop`, `rustamp-platform`, `rustamp-prefs`, `rustamp-library`, `rustamp-app`, `rustamp-testkit` | GPL-3.0-or-later (see [LICENSE](LICENSE)) |
| Plugin-facing | `rustamp-proto`, `rustamp-plugin-abi`, `rustamp-ipc`, `rustamp-plughost` | MIT OR Apache-2.0 (see [LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)) |

`rustamp-plughost32.exe` is the process that loads classic plugin DLLs. It links
only the permissive crates above, so the plugin edge carries no copyleft
whatsoever — for plugins of any license, including GPL-2.0-only ones, which are
incompatible with GPL-3.0.

**Section 7 plugin exception.** [PLUGIN-EXCEPTION.md](PLUGIN-EXCEPTION.md)
records the GPL-3.0 section 7 additional permission for the GPL-3.0-or-later
crates that contain only Rustamp's own code: it permits combining the covered
work with independent modules that communicate solely through the documented
classic-plugin ABI (`rustamp-plugin-abi`) and the documented IPC protocol
(`rustamp-ipc`, protocol version 1), regardless of those modules' licenses, and
requires the interfaces to stay unmodified. It does **not** apply to
third-party GPL material; `rustamp-decoders` and any other crate that carries
third-party GPL code therefore does not carry it.

**No third-party code in the permissive crates.** `rustamp-proto`,
`rustamp-plugin-abi`, `rustamp-ipc` and `rustamp-plughost` contain only
Rustamp's own code; nothing in the table above lives in them. Their license
texts are the canonical MIT and Apache-2.0 texts, copied unmodified from a crate
in the local cargo registry.
