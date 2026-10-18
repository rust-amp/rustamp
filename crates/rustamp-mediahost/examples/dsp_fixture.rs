//! VM-only deterministic DSP resident binary for the ignored two-process
//! tests. No codec or audio device; never install/package this example as a
//! DSP resident.

#[cfg(windows)]
#[path = "../tests/support/dsp_resident.rs"]
mod dsp_resident;

#[cfg(windows)]
fn main() -> rustamp_mediahost::Result<()> {
    use std::time::Duration;

    use rustamp_mediahost::control::{ControlPeer, Liveness};
    use rustamp_mediahost::dsp::serve_dsp;
    use rustamp_mediahost::pcm::NamedPcmRings;

    // Liveness policy matched to the parent test's peer (30 s timeout):
    // the framework default (5 s) is a wall-clock bound a healthy-but-slow
    // VM schedule can trip under load. A dead parent process closes stdin/stdout
    // and is caught by EOF regardless.
    const LIVENESS: Liveness = Liveness {
        interval: Duration::from_millis(100),
        timeout: Duration::from_secs(30),
    };

    let peer = ControlPeer::start(std::io::stdin(), std::io::stdout(), LIVENESS, true)?;
    serve_dsp(&peer, &NamedPcmRings, &dsp_resident::StubDspResident)
}

#[cfg(not(windows))]
fn main() {}
