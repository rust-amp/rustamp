//! # rustamp-plughost32
//!
//! The i686 helper process that loads classic Winamp plugin DLLs and talks to
//! `rustamp.exe` over `rustamp-ipc`. Responsibilities (later milestones): DLL
//! loading, the fake "Winamp v1.x" main window, a fake out-module for `in_`
//! plugins, embed-window frames, and the DSP round-trip.
//!
//! M0: stub on every target; it prints its protocol version, the shape of its
//! `WM_COMMAND` forwarding table (RFC 0005) and exits.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004); see `LICENSE-MIT` and
//! `LICENSE-APACHE` at the repository root. The host binary is a separate
//! program from the GPL player and imposes no copyleft on the plugins it loads.

mod commands;

use commands::Disposition;

fn main() {
    println!(
        "rustamp-plughost32 {} (ipc protocol v{}) - stub",
        env!("CARGO_PKG_VERSION"),
        rustamp_ipc::PROTOCOL_VERSION
    );

    // M3 replaces this with the WM_COMMAND message loop. Until then the stub
    // reports the table that loop will use and walks the S03 id list through it,
    // so a forgotten id shows up here instead of in the loop.
    let mut forwarded = 0;
    let mut dropped = 0;
    for id in commands::PUBLIC_IDS {
        match commands::disposition(*id) {
            Disposition::Forward(_) => forwarded += 1,
            Disposition::Drop(_) => dropped += 1,
        }
    }
    println!(
        "  WM_COMMAND: {forwarded} ids forwarded, {dropped} logged and dropped ({} in the S03 list)",
        commands::PUBLIC_IDS.len()
    );
    // TODO(M3, Windows only): connect to the control pipe, load plugins, run the message loop.
}
