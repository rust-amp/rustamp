# Rustamp

Rustamp is a media player that looks, feels and works like **Winamp 2.95**, written in Rust.

- **Look and feel:** the classic Main, Equalizer, Playlist and visualization windows, with docking, snapping, windowshade, double-size and the original keyboard shortcuts.
- **Modern pipeline:** 64-bit Rust, a `symphonia`-based decoder stack with FFI decoders, WASAPI output, and a native MilkDrop 2 port that runs on `wgpu`.
- **Classic plugins:** Winamp 2.x input, output, visualization, DSP and general plugins run in a separate 32-bit plugin host (`rustamp-plughost`), so a crashing plugin does not take down the player.
- **Platforms:** Windows first. Linux and macOS follow in a later milestone, with legacy plugins running under Wine there.

**Status: M0 — foundation.** The workspace, contracts, CI and clean-room process are being set up. There is no playable build yet.

See [docs/PLAN.md](docs/PLAN.md) for the full plan, milestones and architecture.

## Building

Requirements: a recent stable Rust toolchain (pinned in `rust-toolchain.toml`) and the MSVC build tools on Windows.

```sh
cargo build --workspace
cargo run -p rustamp-app
```

The classic plugin host is a 32-bit binary, because classic Winamp plugins are x86 DLLs:

```sh
rustup target add i686-pc-windows-msvc
cargo build -p rustamp-plughost --target i686-pc-windows-msvc
```

## Skins

Rustamp ships an **original look-alike default skin**. Its artwork was drawn from scratch to resemble the classic base skin, and it contains no pixels from Winamp.

The original base skin is a **separate, optional download** with its own package. It is not bundled with the installer. Rustamp can load any classic `.wsz` skin.

## Trademark notice

Winamp is a trademark of its respective owner. Rustamp is an independent project and is **not affiliated with, endorsed by or sponsored by** the owner of Winamp.

## Contributing

Please read [CONTRIBUTING.md](CONTRIBUTING.md) and [CLEANROOM.md](CLEANROOM.md) before opening a pull request. The clean-room rules are binding: contributors must not use Winamp source code, SDK material or disassembly for implementation work.

## License

Rustamp is licensed under the **GNU General Public License v3.0**. See [LICENSE](LICENSE). Third-party code and its licenses are listed in the THIRD_PARTY notice.
