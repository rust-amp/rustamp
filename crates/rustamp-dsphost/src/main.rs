//! `rustamp-dsphost-stages` — the first real DSP resident (RFC 0006 §6).
//!
//! GPL-3.0-or-later, no §7 exception. Builds gain/passthrough `DspStage`
//! chains and serves them over the unchanged v3 wire plus two shared-memory
//! PCM rings. Headless: stdio control, no window, no audio output.
//!
//! The binary name differs from the framework placeholder `rustamp-dsphost`
//! on purpose: that binary reports that no resident is linked and stays for
//! the framework tests, while this one actually processes. Packaging picks
//! this binary when it stops being a stub.

#[cfg(windows)]
fn main() -> rustamp_mediahost::Result<()> {
    use rustamp_dsphost::StageResident;
    use rustamp_mediahost::control::{ControlPeer, Liveness};
    use rustamp_mediahost::dsp::serve_dsp;
    use rustamp_mediahost::pcm::NamedPcmRings;
    let peer = ControlPeer::start(
        std::io::stdin(),
        std::io::stdout(),
        Liveness::default(),
        true,
    )?;
    serve_dsp(&peer, &NamedPcmRings, &StageResident)
}

#[cfg(not(windows))]
fn main() -> rustamp_mediahost::Result<()> {
    Err(rustamp_mediahost::MediaHostError::NotImplemented(
        "cross-process PCM mappings require Windows; M6 is deferred".into(),
    ))
}
