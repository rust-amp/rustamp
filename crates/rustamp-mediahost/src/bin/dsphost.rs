//! Framework entry point. A real DSP chain lives in a separately licensed
//! resident binary using the same `serve_dsp` API, never in this permissive
//! binary.

#[cfg(windows)]
fn main() -> rustamp_mediahost::Result<()> {
    use rustamp_mediahost::control::{ControlPeer, Liveness};
    use rustamp_mediahost::dsp::{UnavailableDspResident, serve_dsp};
    use rustamp_mediahost::pcm::NamedPcmRings;
    let peer = ControlPeer::start(
        std::io::stdin(),
        std::io::stdout(),
        Liveness::default(),
        true,
    )?;
    serve_dsp(&peer, &NamedPcmRings, &UnavailableDspResident)
}

#[cfg(not(windows))]
fn main() -> rustamp_mediahost::Result<()> {
    Err(rustamp_mediahost::MediaHostError::NotImplemented(
        "cross-process PCM mappings require Windows; M6 is deferred".into(),
    ))
}
