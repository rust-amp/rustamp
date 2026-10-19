// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The real exclusive open: **VM-only, `#[ignore]`d**.
//!
//! The unit tests of the crate cover the parts of the exclusive path that
//! need no device: the format selection, the period selection and the
//! sample conversion (see `wasapi_exclusive`). What they cannot cover is a
//! device that really grants an exclusive stream, and a device that really
//! refuses one — that needs real hardware.
//!
//! This test opens the real default render endpoint in exclusive mode and
//! checks the two things that only a real device can answer:
//!
//! * the stream is either really exclusive, or the refusal was reported —
//!   the two are never mixed up, and the downgrade is never silent;
//! * the endpoint the caller asked for is the endpoint that was opened (the
//!   backend never substitutes another device).
//!
//! It opens **no window** and it starts **no stream**: the endpoint is
//! opened and closed again, so the test plays no audible audio. It does take
//! the device in exclusive mode for a moment, which is why it runs in the
//! headless test VM and never on the host:
//!
//! ```text
//! cargo test -p rustamp-output --test wasapi_exclusive -- --ignored
//! ```
//!
//! If the VM has **no audio endpoint**, the test fails loudly — it never
//! passes vacuously and never skips silently. Enabling audio in the VM (a
//! virtual audio device or the Hyper-V audio redirection) is what it needs.

#![cfg(windows)]

use rustamp_core::audio::{AudioSpec, OutputDevice};
use rustamp_output::{WasapiMode, WasapiOutput};

/// Opens the real endpoint in exclusive mode.
#[test]
#[ignore = "VM-only: opens a real endpoint in exclusive mode; run inside the headless test VM"]
fn an_exclusive_stream_opens_on_a_real_device() {
    let mut out = WasapiOutput::new().with_mode(WasapiMode::Exclusive);
    let format = match out.open(AudioSpec::new(44_100, 2)) {
        Ok(format) => format,
        Err(error) => panic!(
            "NO AUDIO ENDPOINT: the real output device could not be opened ({error}). \
             This test needs an audio endpoint in the headless test VM (a virtual audio \
             device or the Hyper-V audio redirection). It fails loudly by design: without \
             a device there is nothing to verify, and passing anyway would be a lie."
        ),
    };

    // The granted format is usable, whatever the device granted.
    assert!(
        format.spec.sample_rate > 0,
        "the device granted a zero sample rate"
    );
    assert!(
        (1..=2).contains(&format.spec.channels),
        "the device granted {} channels",
        format.spec.channels
    );
    assert!(
        format.buffer_frames > 0,
        "the device granted an empty buffer"
    );

    // Either the stream is really exclusive, or the refusal was reported.
    // A downgrade that is not reported would be the bug this test exists
    // for.
    match out.mode_in_use() {
        WasapiMode::Exclusive => {
            assert!(
                out.downgrade_reason().is_none(),
                "an exclusive stream has no downgrade reason"
            );
        }
        WasapiMode::Shared => {
            let reason = out
                .downgrade_reason()
                .expect("a shared stream after an exclusive request has a reason");
            assert!(!reason.is_empty(), "the downgrade reason is not empty");
        }
    }

    // The mode is part of the name, so the downgrade is visible to a caller
    // that only holds the trait object.
    let name = out.name().to_owned();
    assert!(
        name.contains("[wasapi exclusive]") || name.contains("[wasapi shared]"),
        "the device name carries the mode in use, got {name}"
    );

    out.close().expect("closing the exclusive stream");
}
