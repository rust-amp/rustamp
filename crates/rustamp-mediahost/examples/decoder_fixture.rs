//! VM-only deterministic resident binary for the ignored two-process tests.
//! No codec or audio device; never install/package this example as a decoder.

#[cfg(windows)]
#[path = "../tests/support/resident.rs"]
mod resident;

#[cfg(windows)]
fn main() -> rustamp_mediahost::Result<()> {
    use rustamp_mediahost::control::{ControlPeer, Liveness};
    use rustamp_mediahost::decoder::serve_decoder;
    use rustamp_mediahost::pcm::NamedPcmRings;
    let peer = ControlPeer::start(
        std::io::stdin(),
        std::io::stdout(),
        Liveness::default(),
        true,
    )?;
    serve_decoder(&peer, &NamedPcmRings, &resident::StubResident)
}

#[cfg(not(windows))]
fn main() {}
