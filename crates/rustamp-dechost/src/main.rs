//! `rustamp-dechost-symphonia` — the first real decoder resident (RFC 0006 §5).
//!
//! GPL-3.0-or-later, no §7 exception. Opens real files through the decoder
//! registry and serves them over the unchanged v3 wire plus the shared-memory
//! PCM ring. Headless: stdio control, no window, no audio output.
//!
//! The binary name differs from the framework placeholder `rustamp-dechost`
//! on purpose: that binary reports that no resident is linked and stays for
//! the framework tests, while this one actually decodes. Packaging picks this
//! binary when it stops being a stub.

#[cfg(windows)]
fn main() -> rustamp_mediahost::Result<()> {
    use rustamp_dechost::RegistryResident;
    use rustamp_mediahost::control::{ControlPeer, Liveness};
    use rustamp_mediahost::decoder::serve_decoder;
    use rustamp_mediahost::pcm::NamedPcmRings;
    let peer = ControlPeer::start(
        std::io::stdin(),
        std::io::stdout(),
        Liveness::default(),
        true,
    )?;
    serve_decoder(&peer, &NamedPcmRings, &RegistryResident)
}

#[cfg(not(windows))]
fn main() -> rustamp_mediahost::Result<()> {
    Err(rustamp_mediahost::MediaHostError::NotImplemented(
        "cross-process PCM mappings require Windows; M6 is deferred".into(),
    ))
}
