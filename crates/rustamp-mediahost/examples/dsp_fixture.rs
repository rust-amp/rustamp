//! VM-only deterministic DSP resident binary for the ignored two-process
//! tests. No codec or audio device; never install/package this example as a
//! DSP resident.

#[cfg(windows)]
#[path = "../tests/support/dsp_resident.rs"]
mod dsp_resident;

#[cfg(windows)]
fn main() -> rustamp_mediahost::Result<()> {
    use rustamp_mediahost::control::{ControlPeer, Liveness};
    use rustamp_mediahost::dsp::serve_dsp;
    use rustamp_mediahost::pcm::NamedPcmRings;
    let peer = ControlPeer::start(
        std::io::stdin(),
        std::io::stdout(),
        Liveness::default(),
        true,
    )?;
    serve_dsp(&peer, &NamedPcmRings, &dsp_resident::StubDspResident)
}

#[cfg(not(windows))]
fn main() {}
