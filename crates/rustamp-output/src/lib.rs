// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! # rustamp-output
//!
//! Concrete [`OutputDevice`](rustamp_core::audio::OutputDevice)
//! implementations:
//!
//! * [`NullOutput`] — deterministic virtual device for tests and `--null`
//!   playback. The caller drives its clock with [`NullClock::pump`].
//! * [`WasapiOutput`] — Windows shared-mode WASAPI (`windows` only), event
//!   driven, with automatic format conversion so the endpoint grants the
//!   requested format.
//! * [`CpalOutput`] — the cross-platform fallback behind the same trait.
//!
//! All three fill the whole buffer, write silence on underrun and do not
//! allocate, lock or log inside the render callback. A failed `open` names the
//! device it tried and lists the devices that exist; no backend substitutes a
//! different device behind the caller's back.
//!
//! `default_output()` picks WASAPI on Windows when a shared-mode client can be
//! initialised and `cpal` otherwise. That choice is made **before** any open and
//! is visible to the caller through [`OutputDevice::name`]; the selected backend
//! then never falls back to another device — if its device is absent, busy or
//! unusable, `open` fails loudly and the caller decides, for example by falling
//! back to [`NullOutput`] explicitly. `out_disk` (the `out_disk` equivalent of
//! the original player) is an M2 item.

#![warn(missing_docs)]

mod null;

#[cfg(windows)]
mod wasapi_out;

mod cpal_out;

pub use cpal_out::CpalOutput;
pub use null::{DEFAULT_CHUNK_FRAMES, NullClock, NullOutput};

#[cfg(windows)]
pub use wasapi_out::{WasapiOutput, is_shared_mode_available};

use rustamp_core::audio::OutputDevice;

/// The best output device for this machine.
///
/// On Windows this creates [`WasapiOutput`] (shared mode) when a shared-mode
/// client can be initialised for the default endpoint and [`CpalOutput`]
/// otherwise; elsewhere it is always [`CpalOutput`].
///
/// The *backend* is chosen here, before any device is opened; it is visible to
/// the caller through [`OutputDevice::name`]. Once chosen, the backend does not
/// silently switch devices: an `open` that finds the device absent, busy or
/// unable to grant the format fails with a message that names the device and
/// lists the alternatives. This function itself never opens a stream and never
/// returns an error; the caller can fall back to [`NullOutput`] explicitly (the
/// `--null` mode does).
pub fn default_output() -> Box<dyn OutputDevice> {
    #[cfg(windows)]
    {
        if is_shared_mode_available() {
            return Box::new(WasapiOutput::new());
        }
    }
    Box::new(CpalOutput::new())
}
