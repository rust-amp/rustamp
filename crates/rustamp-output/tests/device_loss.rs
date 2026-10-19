// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! A real device change on a real endpoint: **VM-only, `#[ignore]`d**.
//!
//! The unit tests of the crate cover the parts of the recovery that need no
//! device: the latch, the classification of the failure and the decision
//! ([`rustamp_output::recovery`]). What they cannot cover is the device change
//! itself — Windows invalidating the audio client of a stream that is really
//! running.
//!
//! This test causes that change on a real endpoint: it opens the real default
//! endpoint in shared mode, starts a real (silent) stream, and then takes the
//! endpoint away by opening a second client on it in **exclusive mode**. The
//! exclusive-mode takeover is the one device change a program can cause
//! itself: Windows stops the shared stream and invalidates its client, which
//! is the `AUDCLNT_E_DEVICE_INVALIDATED` the backend classifies.
//!
//! What it asserts:
//!
//! * a healthy real stream latches nothing (the backend does not invent a
//!   failure);
//! * a real invalidation is reported — never silently — and the report agrees
//!   with the decision table: a device that changed while the stream follows
//!   the system default continues on the current default device;
//! * the recovery works on the real device: the same object reopens once the
//!   endpoint is free again, and the latch is cleared.
//!
//! What it cannot prove: that *every* device change behaves like the exclusive
//! takeover. A real unplug is the other way to invalidate a client, and it
//! needs a human or a runner to pull the plug. An endpoint that refuses an
//! exclusive stream (a virtual device need not support one) cannot be taken
//! away that way at all; the test then says so loudly instead of pretending it
//! saw a device change.
//!
//! It opens **no window** and plays **no audible audio** (the render callback
//! writes silence), but it takes a real endpoint, which is why it runs in the
//! headless test VM and never on the host:
//!
//! ```text
//! cargo test -p rustamp-output --test device_loss -- --ignored
//! ```
//!
//! If the VM has **no audio endpoint**, the test fails loudly — it never
//! passes vacuously and never skips silently.

#![cfg(windows)]

use std::time::{Duration, Instant};

use rustamp_core::audio::{AudioSpec, OutputDevice};
use rustamp_output::WasapiOutput;
use wasapi::{
    AudioClient, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat, initialize_mta,
};

/// How long the backend may take to notice the change.
const DEADLINE: Duration = Duration::from_secs(10);

/// How long a reopen may take once the endpoint is free again.
const REOPEN_DEADLINE: Duration = Duration::from_secs(5);

/// The interval between two checks of the latched error.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

#[test]
#[ignore = "VM-only: takes a real audio endpoint away from a running stream; run inside the headless test VM"]
fn a_real_device_change_is_reported_with_its_recovery() {
    let mut out = WasapiOutput::new();
    let format = match out.open(AudioSpec::new(44_100, 2)) {
        Ok(format) => format,
        Err(error) => panic!(
            "NO AUDIO ENDPOINT: the real output device could not be opened ({error}). \
             This test needs an audio endpoint in the headless test VM (a virtual audio \
             device or the Hyper-V audio redirection). It fails loudly by design: without \
             a device there is nothing to verify, and passing anyway would be a lie."
        ),
    };
    assert!(
        format.buffer_frames > 0,
        "the device granted an empty buffer"
    );
    out.start(Box::new(|buf: &mut [f32]| buf.fill(0.0)))
        .expect("starting the real stream");

    // A healthy real device latches nothing: the backend must not invent a
    // failure.
    assert!(
        out.stream_error().is_none(),
        "a healthy real stream reported {:?}",
        out.stream_error()
    );

    // Take the endpoint away from the running stream.
    let takeover = take_the_endpoint_exclusively();

    // Wait for the backend to notice the change.
    let start = Instant::now();
    let latched = loop {
        if let Some(error) = out.stream_error() {
            break Some(error);
        }
        if start.elapsed() > DEADLINE {
            break None;
        }
        std::thread::sleep(POLL_INTERVAL);
    };

    match (&takeover, latched) {
        // The takeover was accepted and the shared client was invalidated:
        // the backend must report it, and the report must agree with the
        // decision table.
        (Some(_), Some(error)) => {
            let message = error.to_string();
            assert!(
                message.contains("invalidated") || message.contains("disappeared"),
                "a real device change must name the fault: {message}"
            );
            // The stream follows the system default, so the recovery is the
            // device that is the default now.
            assert!(
                message.contains("press Play to continue on the current default device"),
                "the report must name the recovery: {message}"
            );
        }
        // The endpoint refused the exclusive stream: nothing was taken away,
        // so there is no device change to report.
        (None, None) => {
            eprintln!(
                "the endpoint refused the exclusive stream: the device change could not be \
                 caused here, and the healthy stream correctly reported nothing"
            );
        }
        // The takeover was accepted but the shared client kept running: either
        // the platform does not invalidate shared clients on a takeover, or the
        // backend does not see the invalidation. Both are worth knowing, and
        // neither may pass unnoticed.
        (Some(_), None) => panic!(
            "the exclusive takeover was accepted, but the shared stream was still healthy \
             after {DEADLINE:?}: the device change was not seen. Either the platform does \
             not invalidate a shared client on a takeover, or the backend does not notice \
             the invalidation."
        ),
        // The endpoint refused the exclusive stream, yet the backend reported
        // a failure: a healthy stream must not report anything.
        (None, Some(error)) => panic!(
            "the endpoint refused the exclusive stream, but the backend reported a failure: \
             {error}"
        ),
    }

    // The recovery on the real device: the endpoint is free again, and the
    // same object reopens with a cleared latch.
    drop(takeover);
    let closed = out.close();
    eprintln!("the close after the device change reported {closed:?}");

    let start = Instant::now();
    loop {
        match out.open(AudioSpec::new(44_100, 2)) {
            Ok(_) => break,
            Err(error) => {
                if start.elapsed() > REOPEN_DEADLINE {
                    panic!(
                        "the output did not reopen within {REOPEN_DEADLINE:?} after the device \
                         change ({error}); the recovery needs a fresh open"
                    );
                }
                std::thread::sleep(POLL_INTERVAL);
            }
        }
    }
    assert!(
        out.stream_error().is_none(),
        "a successful open must clear the latched failure, got {:?}",
        out.stream_error()
    );
    let _ = out.close();
}

/// Open a second client on the default endpoint in exclusive mode.
///
/// The exclusive-mode takeover is the device change a test can cause itself:
/// Windows stops the shared stream and invalidates its client. `None` when
/// the endpoint refuses an exclusive stream — a virtual device need not
/// support one, and then the test cannot cause a device change at all.
fn take_the_endpoint_exclusively() -> Option<AudioClient> {
    // `initialize_mta` answers with the HRESULT of the COM initialisation.
    if initialize_mta().ok().is_err() {
        return None;
    }
    let enumerator = DeviceEnumerator::new().ok()?;
    let device = enumerator.get_default_device(&Direction::Render).ok()?;
    let mut client = device.get_iaudioclient().ok()?;
    let mix = client.get_mixformat().ok()?;
    let (default_period, _min_period) = client.get_device_period().ok()?;

    // The device's own mix format first, then a plain integer format at the
    // same rate: an endpoint that accepts the first refuses the second (or
    // the other way round).
    let integer = WaveFormat::new(
        16,
        16,
        &SampleType::Int,
        mix.get_samplespersec() as usize,
        mix.get_nchannels() as usize,
        None,
    );
    let formats = [mix, integer];
    for format in formats {
        if client
            .initialize_client(
                &format,
                &Direction::Render,
                &StreamMode::EventsExclusive {
                    period_hns: default_period,
                },
            )
            .is_ok()
        {
            // The takeover happens when the client is initialised; starting it
            // makes the exclusive stream real. A refusal to start does not
            // undo the takeover.
            let _ = client.start_stream();
            return Some(client);
        }
    }
    None
}
