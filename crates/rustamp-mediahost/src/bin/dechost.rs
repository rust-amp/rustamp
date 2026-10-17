//! Framework entry point. A real codec lives in a separately licensed resident
//! binary using the same `serve_decoder` API, never in this permissive binary.

#[cfg(windows)]
fn main() -> rustamp_mediahost::Result<()> {
    use rustamp_mediahost::control::{ControlPeer, Liveness};
    use rustamp_mediahost::decoder::{UnavailableResident, serve_decoder};
    use rustamp_mediahost::pcm::NamedPcmRings;
    let peer = ControlPeer::start(
        std::io::stdin(),
        std::io::stdout(),
        Liveness::default(),
        true,
    )?;
    serve_decoder(&peer, &NamedPcmRings, &UnavailableResident)
}

#[cfg(not(windows))]
fn main() -> rustamp_mediahost::Result<()> {
    Err(rustamp_mediahost::MediaHostError::NotImplemented(
        "cross-process PCM mappings require Windows; M6 is deferred".into(),
    ))
}
