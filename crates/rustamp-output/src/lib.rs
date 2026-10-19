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
//!
//! # Mid-run device failures (RFC 0012)
//!
//! A device that disappears, loses its format or refuses a write mid-run latches
//! the failure; the engine learns it through
//! [`OutputDevice::stream_error`] and ends the run with a reported error. The
//! latch and its mapping into an [`Error`](rustamp_core::error::Error) are
//! covered by deterministic unit tests: the real backends are driven through
//! their own atomic latch, and [`NullOutput`] models the failure without any
//! hardware.
//!
//! The **real** failure path — an endpoint that is really removed, a mix format
//! that is really renegotiated, a write that really fails on a real endpoint —
//! needs real hardware and is therefore **VM-only**. It is deliberately *not*
//! faked here: the unit tests cover the latch and the mapping, and nothing
//! pretends to have lost a device.
//!
//! # Windows: COM stays off the caller's thread
//!
//! Every COM initialisation this crate performs happens on a thread this crate
//! owns: [`WasapiOutput`] creates and uses its endpoint interfaces only on its
//! render thread, and the shared-mode probe behind [`default_output`] runs on a
//! short-lived thread of its own. The caller's apartment is therefore never
//! consumed, which is what lets the player build its device on the same thread
//! that afterwards creates the window — a thread in the multi-threaded
//! apartment can no longer enter a single-threaded one, and `winit`'s
//! `OleInitialize` (drag and drop) would panic with `RPC_E_CHANGED_MODE`.

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
///
/// # COM apartment (Windows)
///
/// This function **leaves the caller's COM apartment untouched**, so it is safe
/// to call it from the thread that later owns the GUI. The Windows probe needs
/// COM in the multi-threaded apartment ([`is_shared_mode_available`] calls
/// `CoInitializeEx(COINIT_MULTITHREADED)` on *its* thread), and an MTA thread
/// can never become a single-threaded apartment afterwards: `OleInitialize`
/// then fails with `RPC_E_CHANGED_MODE`, which is what `winit` calls when it
/// creates a window with drag-and-drop support. The probe therefore runs on a
/// short-lived thread of its own (see [`probe_shared_mode_off_thread`]), and
/// [`WasapiOutput`] keeps every COM object on its own render thread. Keep it
/// that way: a caller that does the probe inline on the main thread cannot
/// open a window any more.
pub fn default_output() -> Box<dyn OutputDevice> {
    #[cfg(windows)]
    {
        if probe_shared_mode_off_thread() {
            return Box::new(WasapiOutput::new());
        }
    }
    Box::new(CpalOutput::new())
}

/// [`is_shared_mode_available`] on a short-lived thread of its own.
///
/// The probe initialises COM as multi-threaded on the thread that runs it, and
/// that is irreversible for the lifetime of the thread. Running it on a
/// dedicated thread keeps the apartment of the caller — in the player, the
/// thread that goes on to create the window — free, while the answer is
/// exactly the one the inline probe gave: the same process, the same endpoint,
/// the same client parameters.
///
/// The thread is joined, so the backend choice is still made before
/// [`default_output`] returns and nothing is left running behind the caller's
/// back. It exits without `CoUninitialize` (as the inline probe did on the
/// caller's thread); Windows releases the apartment state of a terminated
/// thread, and the per-process MTA that [`WasapiOutput`] needs is entered by
/// its own render thread. A thread that cannot be spawned or that panics is
/// reported as "not available", which selects the [`CpalOutput`] fallback —
/// the same answer a failing probe gives.
#[cfg(windows)]
fn probe_shared_mode_off_thread() -> bool {
    std::thread::Builder::new()
        .name("rustamp-wasapi-probe".into())
        .spawn(is_shared_mode_available)
        .ok()
        .and_then(|probe| probe.join().ok())
        .unwrap_or(false)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// The regression guard for the OLE apartment defect: choosing the backend
    /// must not consume the caller's apartment.
    ///
    /// `winit` creates its window with drag-and-drop support (the player uses
    /// `WindowEvent::DroppedFile`), so it calls `OleInitialize` on the thread
    /// that runs the event loop — which fails with `RPC_E_CHANGED_MODE`, and
    /// panics, once that thread is in the multi-threaded apartment. The player
    /// builds its output device on exactly that thread, before the window
    /// exists, so entering an STA afterwards is the property that has to hold.
    ///
    /// This needs **no audio endpoint**: the probe initialises COM before it
    /// looks for a device, so the test fails on a machine without one too —
    /// which is what makes it the guard the device-bound tests could not be.
    #[test]
    fn default_output_leaves_the_calling_thread_free_for_an_sta() {
        let device = default_output();
        drop(device);
        let entered = wasapi::initialize_sta();
        if entered.is_ok() {
            // Balance our own initialisation; the thread is left as found.
            wasapi::deinitialize();
        }
        assert!(
            entered.is_ok(),
            "default_output() left this thread in the multi-threaded apartment \
             (CoInitializeEx(COINIT_APARTMENTTHREADED) returned {entered:?}); a window \
             created on it would panic in OleInitialize with RPC_E_CHANGED_MODE"
        );
    }
}
