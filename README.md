# Rustamp

Rustamp is a media player that looks, feels and works like **Winamp 2.95**, on a
modern backend written in Rust.

![version](https://img.shields.io/badge/version-1.2.0-blue)

- **Look and feel:** the classic Main, Equalizer and Playlist windows, with
  docking, windowshade, double-size, the original keyboard shortcuts and
  classic `.wsz` skins. Rendering is checked pixel-for-pixel against the real
  Winamp 2.95 with its original base skin (currently 99.5 % of the pixels over
  eleven window states; what differs is documented, see
  [docs/notes/base291-skin-acceptance.md](docs/notes/base291-skin-acceptance.md)).
- **Modern pipeline:** 64-bit Rust, a [Symphonia](https://github.com/pdeljanov/Symphonia)
  decoder stack (MP3, FLAC, Ogg Vorbis, WAV, AIFF, AAC/ALAC, MKA), gapless
  playback, a 10-band equalizer, WASAPI output, HTTP/HTTPS streaming.
- **Classic plugins:** Winamp 2.x input, output, visualization, DSP and general
  purpose plugins run in a separate 32-bit helper process
  (`rustamp-plughost`), so a crashing plugin cannot take the player down. The
  loader works with our own fixtures, `in_openmpt` and one third-party DSP;
  Winamp 5.x plug-ins load, and the player runs MilkDrop 2.25c end to end in the
  test VM (see Status below for the limits).
- **Platforms:** Windows first. Linux and macOS come later, with classic plugins
  under Wine.

## Status

Version **1.2.0** plays music and supports classic skins and plugins. The
following capabilities and verification still have limitations:

| Works | Not yet |
|---|---|
| Main, equalizer and playlist windows, skins, shade and double-size; the spectrum analyser and oscilloscope (real PCM through analysis to pixels, covered by headless tests, not yet confirmed in a captured live window) | Native MilkDrop: `rustamp-milkdrop` is a skeleton; the projectM sources are vendored in `third_party/` but not wired in |
| Playback, seeking, gapless, EQ, volume and balance | Winamp 5.x plug-ins: of the 11 extracted from the 5.66 installer, 10 load in the plug-in host. **The player starts MilkDrop 2.25c end to end in the test VM** (approve with `rustamp --approve-plugin`, start with Start/Stop plug-in; the player's own decoded audio feeds it; software renderer, fixed-function path). Not yet: a GPU run, a first-run flow that fetches or ships the MilkDrop data and presets, and the other classic plug-in kinds beyond start-up; see [docs/notes/real-plugin-survey.md](docs/notes/real-plugin-survey.md) |
| Playlists: M3U, M3U8, PLS, XSPF; Add file / folder / URL (`Ctrl+L`) | Plug-ins are listed, switched on and off and configured (`Configure`) in Preferences → Plug-ins; the first real-world check of that path is the survey above |
| Streaming over `http://` and `https://` with Winamp's status prefixes in the title (`[Connecting]`, `[Buffer: 42%]`) | Crossfade, the media library window, the Preferences pages (partly) |
| Media library database, tag editing | Real-hardware audio verification, a clean-machine install test |

The binaries are **unsigned**, so Windows SmartScreen may warn on first start.
There is no installer and no auto-update: unzip and run.

## Download

Releases are built by CI from version tags and published on the
[releases page](https://gitlab.com/rustamp/rustamp/-/releases). Each release has
a portable zip and a `.sha256` file next to it:

```powershell
# verify, then unzip anywhere and run rustamp.exe
(Get-FileHash .\rustamp-1.2.0+*.zip -Algorithm SHA256).Hash
Get-Content .\rustamp-1.2.0+*.zip.sha256
```

The zip bundles the 64-bit player, its helper processes, the 32-bit plugin host,
the licenses and a quickstart (`QUICKSTART.md`). The C runtime is linked statically, so no Visual C++
redistributable is needed.

## Using it

```text
rustamp [<file|url>] [--null] [--no-audio] [--volume <0..=255>] [--skin <path>]
        [--seconds <n>] [--enqueue] [--import-winamp-ini [path]] [-h|--help]
rustamp [/CLASS=<name>] [/CONFIG=<path>] [/BOOKMARK <path>] [/ADD <path>]
        [/INSTALL] [/UNREG] [/NOREG] [/ONLYINSTALL]
```

- A file path or an `http(s)://` URL starts playback. `--enqueue` adds without
  playing; `/ADD` appends to the list the way the reference does it: a running
  player only appends, a first start then plays the list from its first entry.
- `Ctrl+L` opens a URL, `L` opens files, `Shift+L` a folder, the same as in Winamp.
- `--skin` takes a folder, `.wsz` or `.zip`; the default is the Rust-themed
  skin, and `--skin lookalike` selects the classic look-alike.
- `--import-winamp-ini` reads (never writes) a Winamp 2.95 `winamp.ini`.
- Release builds are GUI programs; add `--console` to see the player's notes.

## Building from source

Requirements: the stable Rust toolchain (pinned in `rust-toolchain.toml`) and the
MSVC build tools on Windows.

```sh
cargo build --workspace
cargo run -p rustamp-app
```

The plugin host is 32-bit, because classic Winamp plugins are x86 DLLs:

```sh
rustup target add i686-pc-windows-msvc
cargo build -p rustamp-plughost --target i686-pc-windows-msvc
```

The portable release zip is built with the same command CI uses:

```sh
cargo xtask package --release --zip     # output in target/package/
```

## Testing

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

These are the **headless** tests; they open no window and need no audio device.
Tests that open real windows, spawn the plugin host or touch an audio endpoint
are `#[ignore]`d and run inside a disposable Hyper-V VM (`docs/notes/vm-*.ps1`),
never on a developer machine.

The skin acceptance tests compare against the original Winamp base skin
(`base-2.91.wsz`, from the MIT-licensed [webamp](https://github.com/captbaritone/webamp)
repository). It is not part of this repository: point `RUSTAMP_BASE291_SKIN` at
it, or the tests are skipped (with `RUSTAMP_REQUIRE_BASE291=1` a missing skin is
a failure, which is what CI uses).

## Continuous integration

[CircleCI](.circleci/config.yml) runs on a Windows executor:

- **every push:** `fmt`, `clippy -D warnings`, the headless tests (the base skin
  is downloaded from webamp and checked against a pinned SHA-256) and the
  32-bit plugin host build;
- **every `v*` tag:** the same tests, then `cargo xtask package --release --zip`.
  The zip and its checksum are stored as CircleCI artifacts, uploaded to the
  package registry and attached to a GitLab release.

To cut a release: bump the workspace version, then `git tag -a v0.1.1 -m "Rustamp 0.1.1"`
and `git push origin v0.1.1`.

The project settings need `GITLAB_TOKEN` (a token with the `api` scope) and
`GITLAB_PROJECT_ID` as environment variables.

## How the code is organised

A Cargo workspace of small crates; the contracts between them live in
`rustamp-core` and change only through an RFC in [docs/rfc/](docs/rfc/).

| Area | Crates |
|---|---|
| Player and windows | `rustamp-app` (the program), `rustamp-ui` (windows and behaviour), `rustamp-gfx`, `rustamp-skin`, `rustamp-platform` (Win32) |
| Audio | `rustamp-audio` (engine), `rustamp-decoders`, `rustamp-dsp`, `rustamp-output`, `rustamp-meta` (tags) |
| Plugins and processes | `rustamp-plughost` (32-bit host), `rustamp-plugin-abi`, `rustamp-ipc`, `rustamp-proto`, `rustamp-mediahost`, `rustamp-dechost`, `rustamp-dsphost` |
| Data | `rustamp-playlist`, `rustamp-library`, `rustamp-prefs` |
| Visualization | `rustamp-vis`, `rustamp-milkdrop` |
| Tooling | `rustamp-testkit`, `xtask` |

Behaviour is specified, not guessed: [specs/](specs/) holds the audited
specifications (window geometry, menus, shortcuts, the plugin ABI, streaming,
...), [docs/PLAN.md](docs/PLAN.md) the architecture and milestones and
[docs/HANDOFF.md](docs/HANDOFF.md) with the dated ledgers the current state.

## Clean-room process

Rustamp reproduces the behaviour of a closed-source program, so the work is
split in two rooms. A *dirty room* observes the real Winamp 2.95 (runs it,
captures screenshots and timings) and writes **specifications in its own
words**; an auditor checks them for copied wording; only audited specs reach
`specs/`. The *clean room* (everyone who writes Rustamp code) works from those
specs and from freely licensed sources only, and never reads Winamp source,
SDK headers or disassembly. The binding rules are in [CLEANROOM.md](CLEANROOM.md).

## Skins

Rustamp ships a **Rust-themed default skin** and a selectable look-alike skin.
Their artwork was drawn from scratch and contains no pixels from Winamp.

The original base skin is a **separate, optional download** with its own
package. It is not bundled. Rustamp can load any classic `.wsz` skin.

## Trademark notice

Winamp is a trademark of its respective owner. Rustamp is an independent project
and is **not affiliated with, endorsed by or sponsored by** the owner of Winamp.

## Contributing

Please read [CONTRIBUTING.md](CONTRIBUTING.md) and [CLEANROOM.md](CLEANROOM.md)
before opening a pull request. The clean-room rules are binding: contributors
must not use Winamp source code, SDK material or disassembly for implementation
work.

## License

Rustamp is **split-licensed**, because the process that loads classic plugin
DLLs must not carry copyleft onto plugins of other licenses:

| Part | License |
|---|---|
| `rustamp-proto`, `rustamp-plugin-abi`, `rustamp-ipc`, `rustamp-plughost`, `rustamp-mediahost` | **MIT OR Apache-2.0** |
| everything else (the player and its crates) | **GPL-3.0-or-later**; additional permission only within the scope below |
| a future per-resident helper crate (`rustamp-mediahost-<resident>`) | **the resident's own license obligations** — the sidplayfp host is GPL; Symphonia is **MPL-2.0 (weak copyleft), not permissive** |

The GPL crates listed in the scope of `PLUGIN-EXCEPTION.md` carry a GPL-3.0 §7
additional permission for Rustamp's own code only, so that combining them
with modules that speak the documented plugin ABI or the documented IPC protocol
needs no further grant for that covered code: see [PLUGIN-EXCEPTION.md](PLUGIN-EXCEPTION.md).
It cannot grant rights in third-party copyleft material; `rustamp-decoders`,
`rustamp-skin` and per-resident helpers are not covered by its scope. The
decision and its reasoning are in
[docs/rfc/0004-plugin-licensing.md](docs/rfc/0004-plugin-licensing.md) and
[docs/notes/L-licensing-plugins.md](docs/notes/L-licensing-plugins.md); the
Winamp plugin SDK's own terms are read in
[docs/notes/SDK-licensing.md](docs/notes/SDK-licensing.md). The GPL text is in
[LICENSE](LICENSE), the permissive texts in
[LICENSE-MIT](LICENSE-MIT)/[LICENSE-APACHE](LICENSE-APACHE). Third-party code
and its licenses are listed in [THIRD_PARTY.md](THIRD_PARTY.md).
