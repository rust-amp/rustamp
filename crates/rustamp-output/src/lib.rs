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
//! allocate, lock or log inside the render callback.
//!
//! `default_output()` picks WASAPI on Windows when a shared-mode client can be
//! initialised and `cpal` otherwise. `out_disk` (the `out_disk` equivalent of
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
/// On Windows this is [`WasapiOutput`] in shared mode; if no endpoint can be
/// opened (headless CI, no audio service) it falls back to [`CpalOutput`], and
/// never fails — the caller can still fall back to [`NullOutput`] explicitly.
pub fn default_output() -> Box<dyn OutputDevice> {
    #[cfg(windows)]
    {
        if is_shared_mode_available() {
            return Box::new(WasapiOutput::new());
        }
    }
    Box::new(CpalOutput::new())
}
