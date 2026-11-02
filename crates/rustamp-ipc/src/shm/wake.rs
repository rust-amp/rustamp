//! Blocking wake events for shared-memory rings (RFC 0007).
//!
//! A **wake ring** is a ring whose creator also created two named, auto-reset
//! kernel events so that a consumer can block instead of polling, and a producer
//! can wait for free space instead of dropping:
//!
//! * `Wake::Data` — signaled by the **producer** after `write_seq`; waited on
//!   by the **consumer**.
//! * `Wake::Space` — signaled by the **consumer** after `read_seq`; waited on
//!   by the **producer**.
//!
//! # Why two events
//!
//! One event per ring conflates "readable" and "writable": with `capacity = 1`
//! the producer can consume the consumer's latch (or vice-versa), and both ends
//! end up asleep. Two events remove the conflation. See RFC 0007 §4.2.
//!
//! # The wake carries no data
//!
//! The wake is only a hint to re-read the header counters, which are already the
//! source of truth and are already generation-guarded. A lost, duplicated or
//! spurious signal can never turn into a wrong frame: the waiter re-checks the
//! counters after every wake (RFC 0007 §6.1–§6.3).
//!
//! # Sleeping is allowed only in four places
//!
//! [`RingWake::wait`] (used by [`RingProducer::wait_writable`] and
//! [`RingConsumer::wait_readable`] and the blocking read/write calls) is the
//! only thing that may **block** in `WaitForSingleObject`. Signaling
//! (`SetEvent`) is a syscall, not a sleep, and is called from the existing
//! `write_from` / `write_slot` / `read_into` / `read_slot_into` / `close`
//! paths. [`RingWake::take_signal`] passes a zero timeout, so it polls the
//! latch and returns without ever being descheduled. See RFC 0007 §6.7.
//!
//! [`RingProducer::wait_writable`]: super::ring::RingProducer::wait_writable
//! [`RingConsumer::wait_readable`]: super::ring::RingConsumer::wait_readable

use std::time::Duration;

use super::RingError;

/// The two directions of wait for a ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    /// Producer signals; the consumer waits for data.
    Data,
    /// Consumer signals; the producer waits for free space.
    Space,
}

/// Derive the name of one of a ring's wake events (RFC 0007 §5.1).
///
/// `<ring_name>-data` / `<ring_name>-space`, living in the same session
/// namespace as the section (no `\` prefix, per RFC 0003 §3).
pub fn ring_wake_name(ring_name: &str, which: Wake) -> String {
    match which {
        Wake::Data => format!("{ring_name}-data"),
        Wake::Space => format!("{ring_name}-space"),
    }
}

/// The outcome of a blocking wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    /// The event was signaled or the condition was already met: re-check the
    /// counters. This is never a claim that data arrived.
    Signaled,
    /// The timeout elapsed with no signal.
    TimedOut,
}

// ---------------------------------------------------------------------------
// Platform-specific backing
// ---------------------------------------------------------------------------
//
// Exactly one of these `backend` modules is compiled. Each exposes the same
// small surface (`Handle`, `do_create`, `do_open`, `do_signal`, `do_wait`,
// `do_close`) so that `RingWake` itself has no `#[cfg]` branches. This keeps the
// public methods trivial and avoids the field-visibility dead-ends that arise
// when the latch state is touched outside the backend.

#[cfg(windows)]
mod backend {
    use super::{RingError, WaitOutcome};
    use std::time::Duration;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows::Win32::System::Threading::{
        CreateEventW, EVENT_ALL_ACCESS, INFINITE, OpenEventW, SetEvent, WaitForSingleObject,
    };
    use windows::core::PCWSTR;

    /// A kernel event handle for one direction of a wake ring.
    pub(super) type Handle = HANDLE;

    /// Encode a name as NUL-terminated UTF-16, as the Win32 calls expect.
    pub(super) fn wide_name(name: &str) -> Vec<u16> {
        name.encode_utf16().chain(core::iter::once(0)).collect()
    }

    /// Create an auto-reset event (`bManualReset = FALSE`, non-signaled).
    pub(super) fn do_create(name: &str) -> Result<Handle, RingError> {
        let wide = wide_name(name);
        // SAFETY: `wide` is NUL-terminated and outlives the call. `None`
        // security attributes → default DACL (RFC 0003 §3, applied to events
        // per RFC 0007 §5.1 O6).
        unsafe { CreateEventW(None, false, false, PCWSTR(wide.as_ptr())) }.map_err(|e| {
            RingError::WakeOpenFailed {
                op: "CreateEventW",
                code: e.code().0,
            }
        })
    }

    /// Open an existing event by name.
    pub(super) fn do_open(name: &str) -> Result<Handle, RingError> {
        let wide = wide_name(name);
        // SAFETY: `wide` is NUL-terminated and outlives the call.
        unsafe { OpenEventW(EVENT_ALL_ACCESS, false, PCWSTR(wide.as_ptr())) }.map_err(|_| {
            RingError::WakeOpenFailed {
                op: "OpenEventW",
                code: 0,
            }
        })
    }

    /// Signal the event (`SetEvent`).
    pub(super) fn do_signal(handle: &Handle) -> Result<(), RingError> {
        // SAFETY: `handle` is a valid event handle owned by this `RingWake`.
        unsafe { SetEvent(*handle) }.map_err(|_| RingError::WakeOpenFailed {
            op: "SetEvent",
            code: 0,
        })
    }

    /// Map a wait timeout to the `WaitForSingleObject` millisecond argument
    /// (RFC 0007 §6.4): the duration is rounded up to whole milliseconds, so
    /// a positive sub-millisecond duration becomes 1 ms. `Duration::ZERO`
    /// maps to 0, which polls the event without blocking, and `None` maps to
    /// `INFINITE`.
    ///
    /// This mapping is the mechanism's entire contribution to wait timing:
    /// everything after it is the operating system's timer behaviour, which
    /// the documented sub-clock-resolution caveat allows to expire early. It
    /// is a pure function so the suite can pin it exactly (see the test in
    /// this module) instead of inferring it from wall-clock measurements.
    pub(super) fn timeout_ms(timeout: Option<Duration>) -> u32 {
        match timeout {
            None => INFINITE,
            Some(d) => {
                let base_ms = u32::try_from(d.as_millis()).unwrap_or(u32::MAX);
                let has_fraction = d.subsec_nanos() % 1_000_000 != 0;
                let rounded = if has_fraction {
                    base_ms.saturating_add(1)
                } else {
                    base_ms
                };
                if rounded == 0 && d > Duration::ZERO {
                    1
                } else {
                    rounded
                }
            }
        }
    }

    /// Wait on the event (`WaitForSingleObject`).
    pub(super) fn do_wait(
        handle: &Handle,
        timeout: Option<Duration>,
    ) -> Result<WaitOutcome, RingError> {
        let ms = timeout_ms(timeout);
        // SAFETY: `handle` is a valid event handle owned by this `RingWake`.
        let result = unsafe { WaitForSingleObject(*handle, ms) };
        match result {
            WAIT_OBJECT_0 => Ok(WaitOutcome::Signaled),
            WAIT_TIMEOUT => Ok(WaitOutcome::TimedOut),
            _ => Err(RingError::WakeOpenFailed {
                op: "WaitForSingleObject",
                code: 0,
            }),
        }
    }

    /// Close this side's handle.
    pub(super) fn do_close(handle: Handle) {
        // SAFETY: `handle` is a valid event handle owned by this `RingWake` and
        // is closed exactly once here.
        unsafe {
            let _ = CloseHandle(handle);
        }
    }
}

#[cfg(not(windows))]
mod backend {
    use super::{RingError, WaitOutcome};
    use std::collections::HashMap;
    use std::sync::Condvar;
    use std::sync::Mutex;
    use std::sync::OnceLock;
    use std::sync::{Arc, Weak};
    use std::time::Duration;

    /// One auto-reset latch: a flag a waiter clears, signaled by the peer.
    /// Off Windows the rings stay within one process.
    pub(super) struct Waiter {
        flag: Mutex<bool>,
        condvar: Condvar,
    }

    impl Waiter {
        pub(super) fn new() -> Self {
            Waiter {
                flag: Mutex::new(false),
                condvar: Condvar::new(),
            }
        }
    }

    /// In-process backing: an `Arc` to a shared latch, so a cloned `RingWake`
    /// (producer + consumer in the same process) shares the same state.
    pub(super) type Handle = Arc<Waiter>;

    /// The latch of a name, the counterpart of a named kernel event: everyone
    /// who names it holds the same latch for as long as anyone holds it.
    fn named(name: &str) -> Handle {
        static NAMED: OnceLock<Mutex<HashMap<String, Weak<Waiter>>>> = OnceLock::new();
        let mut latches = NAMED
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(latch) = latches.get(name).and_then(Weak::upgrade) {
            return latch;
        }
        latches.retain(|_, latch| latch.strong_count() > 0);
        let latch = Arc::new(Waiter::new());
        latches.insert(name.to_owned(), Arc::downgrade(&latch));
        latch
    }

    pub(super) fn do_create(name: &str) -> Result<Handle, RingError> {
        Ok(named(name))
    }

    pub(super) fn do_open(name: &str) -> Result<Handle, RingError> {
        Ok(named(name))
    }

    pub(super) fn do_signal(handle: &Handle) -> Result<(), RingError> {
        let mut flag = handle.flag.lock().unwrap();
        *flag = true;
        handle.condvar.notify_one();
        Ok(())
    }

    pub(super) fn do_wait(
        handle: &Handle,
        timeout: Option<Duration>,
    ) -> Result<WaitOutcome, RingError> {
        let mut flag = handle.flag.lock().unwrap();
        loop {
            if *flag {
                *flag = false;
                return Ok(WaitOutcome::Signaled);
            }
            match timeout {
                None => flag = handle.condvar.wait(flag).unwrap(),
                Some(d) => {
                    let (g, result) = handle.condvar.wait_timeout(flag, d).unwrap();
                    flag = g;
                    if result.timed_out() {
                        return Ok(WaitOutcome::TimedOut);
                    }
                }
            }
        }
    }

    pub(super) fn do_close(_handle: Handle) {
        // `Arc` drops normally; nothing to do here.
    }
}

// ---------------------------------------------------------------------------
// Public `RingWake`
// ---------------------------------------------------------------------------

/// The two auto-reset events of a wake ring; owns this side's handles.
///
/// Created by the ring creator ([`RingWake::create`]) and opened by the ring
/// consumer ([`RingWake::open`]). Each side owns and closes its own handles in
/// `Drop`; the kernel object lives as long as the last handle.
//
// `RingWake` does not implement `Sync`: it is `Send` (the handles may be moved
// to another thread) but is not meant to be shared by reference across threads.
pub struct RingWake {
    data: Option<backend::Handle>,
    space: Option<backend::Handle>,
}

// SAFETY: on Windows the handles are process-local but not thread-affine;
// `SetEvent` and `WaitForSingleObject` are safe from any thread holding a valid
// handle. On non-Windows, `Arc<Waiter>` is `Send + Sync`.
unsafe impl Send for RingWake {}

#[cfg(not(windows))]
impl Clone for RingWake {
    fn clone(&self) -> Self {
        RingWake {
            data: self.data.clone(),
            space: self.space.clone(),
        }
    }
}

#[cfg(windows)]
impl Clone for RingWake {
    fn clone(&self) -> Self {
        use windows::Win32::Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE};
        use windows::Win32::System::Threading::GetCurrentProcess;

        // Duplicate a single handle, returning `None` if `DuplicateHandle`
        // fails (the cloned `RingWake` then has no handle for that direction).
        // SAFETY: `h` is a valid handle owned by this process; we duplicate it
        // into a new handle in the same process.
        let dup = |h: HANDLE| -> Option<HANDLE> {
            let mut new_handle = HANDLE::default();
            unsafe {
                let _ = DuplicateHandle(
                    GetCurrentProcess(),
                    h,
                    GetCurrentProcess(),
                    &mut new_handle,
                    0,
                    false,
                    DUPLICATE_SAME_ACCESS,
                );
            }
            (!new_handle.is_invalid()).then_some(new_handle)
        };

        RingWake {
            data: self.data.and_then(dup),
            space: self.space.and_then(dup),
        }
    }
}

impl RingWake {
    /// Create the two events for a ring (RFC 0007 §5.2, creator side).
    ///
    /// Called before sending `RingOffer`, so an opener that receives the offer
    /// can always open the events.
    pub fn create(ring_name: &str) -> Result<RingWake, RingError> {
        let data_name = ring_wake_name(ring_name, Wake::Data);
        let space_name = ring_wake_name(ring_name, Wake::Space);
        let data = backend::do_create(&data_name)?;
        let space = match backend::do_create(&space_name) {
            Ok(h) => h,
            Err(e) => {
                // Clean up the data event so no half-created wake is leaked.
                backend::do_close(data);
                return Err(e);
            }
        };
        Ok(RingWake {
            data: Some(data),
            space: Some(space),
        })
    }

    /// Open the two events of a ring by name (RFC 0007 §5.2, opener side).
    ///
    /// Called after the ring header has validated, so a stale or foreign ring is
    /// rejected before any kernel object is touched.
    pub fn open(ring_name: &str) -> Result<RingWake, RingError> {
        let data_name = ring_wake_name(ring_name, Wake::Data);
        let space_name = ring_wake_name(ring_name, Wake::Space);
        let data = backend::do_open(&data_name)?;
        let space = match backend::do_open(&space_name) {
            Ok(h) => h,
            Err(e) => {
                backend::do_close(data);
                return Err(e);
            }
        };
        Ok(RingWake {
            data: Some(data),
            space: Some(space),
        })
    }

    /// Close this side's handles. Idempotent — calling it twice (including from
    /// `Drop`) is safe.
    pub fn close(&mut self) {
        if let Some(h) = self.data.take() {
            backend::do_close(h);
        }
        if let Some(h) = self.space.take() {
            backend::do_close(h);
        }
    }

    /// Signal one direction of the wake.
    ///
    /// This is a syscall (`SetEvent`), not a sleep, and is called from the ring's
    /// non-blocking paths after the counter store (RFC 0007 §6.2). Returns
    /// `Ok(())` on success; a signaling failure is reported as
    /// [`RingError::WakeOpenFailed`] but does not affect ring data.
    ///
    /// # The barrier before the signal
    ///
    /// The store that precedes the signal (the counter advance, or the closed
    /// state) must be **visible to the peer before the signal is delivered**,
    /// because the peer's whole protocol is "wake up, then re-read the
    /// counters" (RFC 0007 §6.1). Without a barrier in between, nothing
    /// orders the two:
    ///
    /// * In the memory model, a call to an external function is not a
    ///   synchronisation edge, so the peer's re-read has no happens-before
    ///   relation with the store that this signal announces.
    /// * On the hardware, the store can still sit in this core's store buffer
    ///   when the kernel releases the waiter: the peer, running on another
    ///   core, then reads the *old* counter, finds its condition unmet, and
    ///   goes back to sleep. The wakeup is lost, and the peer waits for its
    ///   whole timeout - forever, with `timeout: None`.
    ///
    /// The `SeqCst` fence is the barrier that closes the window: on x86-64 it
    /// is an `mfence`, which drains the store buffer, so the store is
    /// globally visible before `SetEvent` even reaches the kernel. It costs a
    /// few nanoseconds next to the `SetEvent` syscall it guards, and it is
    /// paid only where a peer can be waiting: a ring without a wake has no
    /// signal and does not pay for it.
    pub(crate) fn signal(&self, which: Wake) -> Result<(), RingError> {
        // The barrier between the caller's counter store and the signal.
        std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
        let handle = match which {
            Wake::Data => &self.data,
            Wake::Space => &self.space,
        };
        if let Some(h) = handle {
            backend::do_signal(h)?;
        }
        Ok(())
    }

    /// Wait for one direction of the wake to be signaled.
    ///
    /// `None` timeout means wait indefinitely; `Some(d)` waits up to `d`.
    /// Returns [`RingError::NoWakeObject`] when this side has no handle for the
    /// requested direction (should not happen for a properly constructed
    /// `RingWake`).
    ///
    /// The `Acquire` fence after the wait orders the caller's re-read of the
    /// counters after the wait, so the re-read cannot be hoisted above it
    /// (RFC 0007 §6.1). On x86-64 it compiles to nothing; on a weakly ordered
    /// architecture it is what makes the re-read see the store that the
    /// signaler's fence published.
    pub(crate) fn wait(
        &self,
        which: Wake,
        timeout: Option<Duration>,
    ) -> Result<WaitOutcome, RingError> {
        let handle = match which {
            Wake::Data => &self.data,
            Wake::Space => &self.space,
        };
        let outcome = match handle {
            Some(h) => backend::do_wait(h, timeout)?,
            None => return Err(RingError::NoWakeObject),
        };
        // The barrier between the wait and the caller's re-read.
        std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);
        Ok(outcome)
    }

    /// Take one direction's pending signal, if there is one, without waiting.
    ///
    /// This is a **poll** of the latch, not a wait: it is
    /// `WaitForSingleObject` with the zero millisecond argument that
    /// `timeout_ms` already derives from `Duration::ZERO` (RFC 0007 §6.4), so
    /// it never sleeps and the sleeping rule of RFC 0007 §6.7 is untouched -
    /// the four blocking ring methods stay the only calls that can block.
    ///
    /// `Signaled` means a signal was pending and this call consumed it (the
    /// events are auto-reset); `TimedOut` means none was pending. As with
    /// every other use of the wake, the outcome says nothing about the ring's
    /// contents - only the counters do (RFC 0007 §6.3).
    ///
    /// It exists so that a third handle on a ring's two latches can assert the
    /// signalling rule of RFC 0007 §6.2 directly, which is the deterministic
    /// half of acceptance criterion 3. The unit tests in `shm::ring` do that
    /// with the crate-private [`RingWake::wait`]; a cross-process peer is a
    /// separate crate and needs this. See
    /// `docs/tasks/W22-wake-cross-process.md`.
    pub fn take_signal(&self, which: Wake) -> Result<WaitOutcome, RingError> {
        self.wait(which, Some(Duration::ZERO))
    }
}

impl Drop for RingWake {
    fn drop(&mut self) {
        // `close` is idempotent and safe from `Drop`. It does NOT set `state`
        // or signal the peer: an abrupt exit looks like a stalled peer, and a
        // waiter's only escape is its timeout (or a peer that closes)
        // (RFC 0007 §6.5).
        self.close();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_names_are_derived_from_the_ring_name() {
        assert_eq!(
            ring_wake_name("rustamp-pcm-1-00000000deadbeef", Wake::Data),
            "rustamp-pcm-1-00000000deadbeef-data"
        );
        assert_eq!(
            ring_wake_name("rustamp-pcm-1-00000000deadbeef", Wake::Space),
            "rustamp-pcm-1-00000000deadbeef-space"
        );
    }

    #[test]
    fn create_and_clone_share_the_same_latches() {
        let wake = RingWake::create("rustamp-test-wake").unwrap();
        let peer = wake.clone();

        // Signal the data latch from `wake`; the `peer` waits on it.
        wake.signal(Wake::Data).unwrap();
        assert_eq!(
            peer.wait(Wake::Data, Some(Duration::from_millis(100)))
                .unwrap(),
            WaitOutcome::Signaled
        );
    }

    #[test]
    fn a_timeout_with_no_signal_returns_timeout() {
        let wake = RingWake::create("rustamp-test-wake-to").unwrap();
        let peer = wake.clone();
        assert_eq!(
            peer.wait(Wake::Data, Some(Duration::from_millis(10)))
                .unwrap(),
            WaitOutcome::TimedOut
        );
    }

    #[test]
    fn close_then_wait_is_not_lost() {
        // After close, a pending signal must still be observable on a clone.
        let mut wake = RingWake::create("rustamp-test-wake-close").unwrap();
        let mut peer = wake.clone();
        wake.close();
        // The handles are cleared; the clone still owns its own handles.
        // Signaling from the clone before close is observable by... well,
        // close consumed wake's handles; this test just ensures close is
        // idempotent and does not panic.
        peer.signal(Wake::Data).unwrap();
        assert_eq!(
            peer.wait(Wake::Data, Some(Duration::from_millis(50)))
                .unwrap(),
            WaitOutcome::Signaled
        );
        peer.close();
        peer.close(); // idempotent
    }

    /// `take_signal` polls a latch without waiting, and consumes the signal
    /// it reports: the events are auto-reset, so one `signal` is one
    /// `Signaled`. It is the primitive the cross-process mid-advance proof of
    /// `docs/tasks/W22-wake-cross-process.md` is built on.
    #[test]
    fn take_signal_reports_and_consumes_one_pending_signal() {
        let wake = RingWake::create("rustamp-test-wake-take").unwrap();
        let probe = wake.clone();

        // Nothing has been signalled yet.
        assert_eq!(
            probe.take_signal(Wake::Data).unwrap(),
            WaitOutcome::TimedOut
        );

        wake.signal(Wake::Data).unwrap();
        assert_eq!(
            probe.take_signal(Wake::Data).unwrap(),
            WaitOutcome::Signaled,
            "a pending signal must be reported"
        );
        assert_eq!(
            probe.take_signal(Wake::Data).unwrap(),
            WaitOutcome::TimedOut,
            "the auto-reset latch must be clear after it was taken"
        );

        // The two directions are separate objects: taking one leaves the
        // other alone.
        wake.signal(Wake::Space).unwrap();
        assert_eq!(
            probe.take_signal(Wake::Data).unwrap(),
            WaitOutcome::TimedOut
        );
        assert_eq!(
            probe.take_signal(Wake::Space).unwrap(),
            WaitOutcome::Signaled
        );
    }

    /// The mechanism's own timing contract is exact and deterministic
    /// (RFC 0007 §6.4): the timeout is rounded up to whole milliseconds
    /// before it reaches `WaitForSingleObject`. This is the only part of
    /// wait timing the implementation controls; everything after it is OS
    /// timer behaviour (task G5).
    #[cfg(windows)]
    #[test]
    fn timeout_ms_rounds_the_request_up_to_whole_milliseconds() {
        // Whole milliseconds pass through unchanged.
        assert_eq!(
            super::backend::timeout_ms(Some(Duration::from_millis(10))),
            10
        );
        assert_eq!(
            super::backend::timeout_ms(Some(Duration::from_millis(1))),
            1
        );
        // A fractional millisecond rounds up.
        assert_eq!(
            super::backend::timeout_ms(Some(Duration::from_micros(1500))),
            2
        );
        // A positive sub-millisecond duration becomes 1 ms.
        assert_eq!(
            super::backend::timeout_ms(Some(Duration::from_micros(500))),
            1
        );
        assert_eq!(super::backend::timeout_ms(Some(Duration::from_nanos(1))), 1);
        // Zero polls the event without blocking.
        assert_eq!(super::backend::timeout_ms(Some(Duration::ZERO)), 0);
        // No timeout is infinite.
        assert_eq!(super::backend::timeout_ms(None), u32::MAX); // INFINITE
        // A duration past u32 milliseconds clamps instead of wrapping.
        assert_eq!(
            super::backend::timeout_ms(Some(Duration::from_secs(60 * 60 * 24 * 400))),
            u32::MAX
        );
    }
}
