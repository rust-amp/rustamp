//! # rustamp-plughost32
//!
//! The i686 helper process that loads classic Winamp plugin DLLs and talks to
//! `rustamp.exe` over `rustamp-ipc`. Responsibilities (later milestones): DLL
//! loading, the fake "Winamp v1.x" main window, a fake out-module for `in_`
//! plugins, embed-window frames, and the DSP round-trip.
//!
//! M0: stub on every target; it prints its protocol version and exits.

fn main() {
    println!(
        "rustamp-plughost32 {} (ipc protocol v{}) - stub",
        env!("CARGO_PKG_VERSION"),
        rustamp_ipc::PROTOCOL_VERSION
    );
    // TODO(M3, Windows only): connect to the control pipe, load plugins, run the message loop.
}
