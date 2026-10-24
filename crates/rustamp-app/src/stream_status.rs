// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! What the app shows about the stream that is playing (S19 §2).
//!
//! # What the decoder reports
//!
//! The decoder observes the transfer and publishes a snapshot of it: the
//! phase of the exchange, the bytes of the body that came in and the length
//! the server declared (`rustamp_decoders::http_progress`). The **texts** are
//! the app's: S19 §2 lists them, and this module spells them as the spec
//! spells them.
//!
//! # Two things, two ways of showing them
//!
//! * A **phase change** is a **note**: one text per change, appended to the
//!   notes the user reads. The phases come and go, and each of them says
//!   something new, so each of them is worth a line.
//! * The **counters** are a **transient status**: the byte count and the rate
//!   change with every sample, so they are not notes — a note per sample
//!   would bury the phase changes under thousands of lines. They are one
//!   entry that is replaced in place, and it is empty when no transfer runs.
//!
//! # The rate and the share are the app's own
//!
//! The decoder deliberately carries no clock: it would have to read one on
//! the hot path of the transfer. The rate therefore comes from **two samples**
//! of the byte count and the app's own clock:
//!
//! ```text
//! rate = (bytes2 - bytes1) / (t2 - t1)
//! ```
//!
//! The share of the body that came in needs no clock: it is the byte count
//! over the length the server declared. The length is there only when the
//! server declared one, and the two variants of the transfer display of
//! S19 §2 differ in exactly that.

use std::time::{Duration, Instant};

use rustamp_decoders::{HttpPhase, HttpProgressSnapshot};

/// The bytes a kilobyte holds.
///
/// S19 §2 writes the rate in KB/s and does not say which kilobyte it means.
/// This is the classic one: 1024 bytes.
const BYTES_PER_KB: f64 = 1024.0;

/// S19 §2: the text shown before the connection is made.
const CONNECTING: &str = "Connecting to host...";

/// S19 §2: the text shown while the request is on its way.
const SENDING_REQUEST: &str = "Sending HTTP request";

/// S19 §2: the text shown while the answer of the host is read.
const READING_RESPONSE: &str = "Reading HTTP response";

/// S19 §2: the text shown while the body is read and the server declared no
/// length.
const RETRIEVING: &str = "Retrieving file...";

/// S19 §2: the text shown while the body is read and the server declared a
/// length. The share is filled in: `Retrieving file (42% done)`.
const RETRIEVING_WITH_SHARE: &str = "Retrieving file";

/// S19 §2: the text shown when the host vanished.
const CONNECTION_LOST: &str = "Connection to host lost";

/// S19 §2: the text shown when the server closed the connection. The spec
/// spells it with a **leading newline**, and the newline is kept.
const CONNECTION_CLOSED: &str = "\nConnection closed.";

/// S19 §2: the text shown when the connection could not be established.
const CONNECT_FAILED: &str = "Error connecting to host";

/// The share of the body that came in, in percent.
///
/// A body that is longer than the length the server declared is counted at
/// 100: the transfer is over, whatever the extra bytes were.
///
/// The product is taken in `u128`: a `u64` count times a hundred overflows
/// above 180 PB, and a saturated product makes the share a wrong number — a
/// transfer that was over came out as `11% done`. A wide local is cheaper
/// than a wrong share.
fn percent(bytes_read: u64, total: u64) -> u64 {
    if total == 0 {
        return 0;
    }
    let share = u128::from(bytes_read) * 100 / u128::from(total);
    u64::try_from(share.min(100)).expect("a share below a hundred fits a u64")
}

/// The rate of the transfer in KB/s, from two samples.
fn rate_kb_per_sec(bytes: u64, elapsed: Duration) -> f64 {
    let seconds = elapsed.as_secs_f64();
    if seconds <= 0.0 {
        return 0.0;
    }
    (bytes as f64 / seconds) / BYTES_PER_KB
}

/// The rate as S19 §2 writes it: `%d.%02d KB/sec`.
fn format_rate(kb_per_sec: f64) -> String {
    let hundredths = (kb_per_sec * 100.0).round().max(0.0);
    // Clamp to u64::MAX before casting to avoid UB when the rate is
    // astronomically large (e.g. u64::MAX bytes in 1 ns).
    let hundredths = hundredths.min(u64::MAX as f64) as u64;
    format!("{}.{:02}", hundredths / 100, hundredths % 100)
}

/// The counters line of S19 §2.
///
/// The server declared no length: `%d bytes read @ %d.%02d KB/sec`. It
/// declared one: `%d/%d bytes read @ %d.%02d KB/sec`.
fn counters_line(snapshot: &HttpProgressSnapshot, rate: f64) -> String {
    match snapshot.total {
        Some(total) => format!(
            "{}/{} bytes read @ {} KB/sec",
            snapshot.bytes_read,
            total,
            format_rate(rate)
        ),
        None => format!(
            "{} bytes read @ {} KB/sec",
            snapshot.bytes_read,
            format_rate(rate)
        ),
    }
}

/// The status text of a phase, as S19 §2 spells it.
///
/// The transfer display is a **two-line shape** (S19 §2): the head names the
/// phase and the counters follow it. The note carries the head — the
/// counters change with every sample and belong to the transient status.
fn phase_text(phase: HttpPhase, snapshot: &HttpProgressSnapshot) -> String {
    match phase {
        HttpPhase::Connecting => CONNECTING.to_string(),
        HttpPhase::SendingRequest => SENDING_REQUEST.to_string(),
        HttpPhase::ReadingResponse => READING_RESPONSE.to_string(),
        HttpPhase::Retrieving => match snapshot.total {
            Some(total) => format!(
                "{RETRIEVING_WITH_SHARE} ({}% done)",
                percent(snapshot.bytes_read, total)
            ),
            None => RETRIEVING.to_string(),
        },
        HttpPhase::ConnectionLost => CONNECTION_LOST.to_string(),
        HttpPhase::Closed => CONNECTION_CLOSED.to_string(),
        HttpPhase::ConnectFailed => CONNECT_FAILED.to_string(),
    }
}

/// The bracketed prefix the title scroller carries while a stream is on its
/// way (`docs/spec-requests/2026-10-24-s19-status-prefixes.md`, measured on the
/// reference; **not yet in an audited spec**).
///
/// Own choices, marked as such: the phases map as `Connecting` and
/// `SendingRequest` to `[Connecting]`, `ReadingResponse` to `[reading id3]`,
/// the body to `[Buffer: N%]` with `N` the share of the declared length (a
/// body with no declared length has no share, so it gets no prefix), and a
/// refused connection to `[could not connect]`, which stays. A lost or closed
/// connection shows no prefix.
fn title_prefix_of(snapshot: &HttpProgressSnapshot) -> Option<String> {
    match snapshot.phase {
        HttpPhase::Connecting | HttpPhase::SendingRequest => Some("[Connecting]".to_string()),
        HttpPhase::ReadingResponse => Some("[reading id3]".to_string()),
        HttpPhase::Retrieving => snapshot
            .total
            .map(|total| format!("[Buffer: {}%]", percent(snapshot.bytes_read, total))),
        HttpPhase::ConnectFailed => Some("[could not connect]".to_string()),
        HttpPhase::ConnectionLost | HttpPhase::Closed => None,
    }
}

/// What the app shows about the stream that is playing.
///
/// The app keeps the phase it reported last, the sample it needs for the
/// rate and the transient status line.
#[derive(Debug, Default)]
pub(crate) struct StreamStatus {
    /// The phase the app reported last.
    phase: Option<HttpPhase>,
    /// The bytes of the last sample and the clock reading it was taken at:
    /// the rate needs two of them.
    sample: Option<Sample>,
    /// The transient status: the transfer display, empty when no transfer
    /// runs.
    status: Option<String>,
    /// The prefix of the title scroller, if the transfer has one.
    prefix: Option<String>,
}

/// A sample of the byte count, with the clock reading it was taken at.
#[derive(Debug, Clone, Copy)]
struct Sample {
    /// The bytes of the body that had come in.
    bytes: u64,
    /// When the sample was taken.
    at: Instant,
}

impl StreamStatus {
    /// Reads a snapshot of the transfer.
    ///
    /// Returns the note the phase change makes, if the phase changed. The
    /// transient status is kept up to date either way.
    pub(crate) fn update(
        &mut self,
        snapshot: HttpProgressSnapshot,
        now: Instant,
    ) -> Option<String> {
        // A transfer counts its bytes from zero: a count that goes back is a
        // new transfer, and the phase of the old one is stale.
        if self
            .sample
            .is_some_and(|sample| sample.bytes > snapshot.bytes_read)
        {
            self.reset();
        }
        let note = if self.phase == Some(snapshot.phase) {
            None
        } else {
            self.phase = Some(snapshot.phase);
            Some(phase_text(snapshot.phase, &snapshot))
        };
        // The rate needs two samples and the app's own clock.
        let rate = self.sample.map(|sample| {
            let elapsed = now.duration_since(sample.at);
            let bytes = snapshot.bytes_read.saturating_sub(sample.bytes);
            rate_kb_per_sec(bytes, elapsed)
        });
        self.prefix = title_prefix_of(&snapshot);
        self.sample = Some(Sample {
            bytes: snapshot.bytes_read,
            at: now,
        });
        // The counters are the transfer's own: they exist while the body is
        // read, and nowhere else.
        self.status = match rate {
            Some(rate) if snapshot.phase == HttpPhase::Retrieving => {
                let head = phase_text(HttpPhase::Retrieving, &snapshot);
                Some(format!("{head}\n{}", counters_line(&snapshot, rate)))
            }
            _ => None,
        };
        note
    }

    /// Forgets the transfer: no stream is running.
    ///
    /// The phase of the last transfer is stale with it, so the next transfer
    /// starts with a phase that is a change again.
    pub(crate) fn reset(&mut self) {
        self.phase = None;
        self.sample = None;
        self.status = None;
        self.prefix = None;
    }

    /// The bracketed prefix of the title scroller, if the transfer has one.
    pub(crate) fn title_prefix(&self) -> Option<&str> {
        self.prefix.as_deref()
    }

    /// The transient status: the transfer display of S19 §2.
    ///
    /// It is the entry the window shows in its status area: one text that is
    /// replaced while the transfer runs, and nothing at all when no transfer
    /// runs.
    pub(crate) fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A snapshot of a transfer in the given phase.
    fn snapshot(phase: HttpPhase, bytes_read: u64, total: Option<u64>) -> HttpProgressSnapshot {
        HttpProgressSnapshot {
            phase,
            bytes_read,
            total,
        }
    }

    /// The scroller prefix follows the phase (measured words, own mapping).
    #[test]
    fn the_title_prefix_follows_the_phase() {
        let cases = [
            (
                snapshot(HttpPhase::Connecting, 0, Some(100)),
                Some("[Connecting]"),
            ),
            (
                snapshot(HttpPhase::SendingRequest, 0, None),
                Some("[Connecting]"),
            ),
            (
                snapshot(HttpPhase::ReadingResponse, 0, Some(100)),
                Some("[reading id3]"),
            ),
            (
                snapshot(HttpPhase::Retrieving, 22, Some(100)),
                Some("[Buffer: 22%]"),
            ),
            (snapshot(HttpPhase::Retrieving, 5, None), None),
            (
                snapshot(HttpPhase::ConnectFailed, 0, None),
                Some("[could not connect]"),
            ),
            (snapshot(HttpPhase::ConnectionLost, 9, Some(100)), None),
            (snapshot(HttpPhase::Closed, 100, Some(100)), None),
        ];
        for (snapshot, expected) in cases {
            assert_eq!(
                title_prefix_of(&snapshot).as_deref(),
                expected,
                "{snapshot:?}"
            );
        }
    }

    /// A reset forgets the prefix with the transfer.
    #[test]
    fn a_reset_forgets_the_prefix() {
        let mut status = StreamStatus::default();
        let now = Instant::now();
        status.update(snapshot(HttpPhase::Connecting, 0, None), now);
        assert_eq!(status.title_prefix(), Some("[Connecting]"));
        status.reset();
        assert_eq!(status.title_prefix(), None);
    }

    /// The texts of S19 §2, one per phase.
    #[test]
    fn every_phase_has_the_text_the_spec_spells() {
        let cases = [
            (HttpPhase::Connecting, "Connecting to host..."),
            (HttpPhase::SendingRequest, "Sending HTTP request"),
            (HttpPhase::ReadingResponse, "Reading HTTP response"),
            (HttpPhase::ConnectionLost, "Connection to host lost"),
            (HttpPhase::Closed, "\nConnection closed."),
            (HttpPhase::ConnectFailed, "Error connecting to host"),
        ];
        for (phase, text) in cases {
            let snapshot = snapshot(phase, 0, None);
            assert_eq!(phase_text(phase, &snapshot), text);
        }
    }

    /// The transfer display has two variants, and the total is what tells
    /// them apart.
    #[test]
    fn the_transfer_text_depends_on_the_total() {
        let unknown = snapshot(HttpPhase::Retrieving, 100, None);
        assert_eq!(
            phase_text(HttpPhase::Retrieving, &unknown),
            "Retrieving file..."
        );
        let known = snapshot(HttpPhase::Retrieving, 100, Some(1000));
        assert_eq!(
            phase_text(HttpPhase::Retrieving, &known),
            "Retrieving file (10% done)"
        );
    }

    /// The share never passes a hundred.
    #[test]
    fn the_share_stops_at_a_hundred() {
        let over = snapshot(HttpPhase::Retrieving, 2000, Some(1000));
        assert_eq!(
            phase_text(HttpPhase::Retrieving, &over),
            "Retrieving file (100% done)"
        );
    }

    /// The rate is the app's own: two samples and the app's clock.
    #[test]
    fn the_rate_comes_from_two_samples() {
        let start = Instant::now();
        let mut status = StreamStatus::default();
        // The first sample has no rate yet.
        assert!(
            status
                .update(snapshot(HttpPhase::Retrieving, 0, None), start)
                .is_some()
        );
        assert_eq!(status.status(), None);
        // A second sample a second later, with 1024 bytes more.
        status.update(
            snapshot(HttpPhase::Retrieving, 1024, None),
            start + Duration::from_secs(1),
        );
        assert_eq!(
            status.status(),
            Some("Retrieving file...\n1024 bytes read @ 1.00 KB/sec")
        );
    }

    /// The counters line names the ratio when the server declared a length.
    #[test]
    fn the_counters_line_carries_the_ratio_when_the_total_is_known() {
        let start = Instant::now();
        let mut status = StreamStatus::default();
        status.update(snapshot(HttpPhase::Retrieving, 0, Some(2000)), start);
        status.update(
            snapshot(HttpPhase::Retrieving, 1000, Some(2000)),
            start + Duration::from_secs(1),
        );
        assert_eq!(
            status.status(),
            Some("Retrieving file (50% done)\n1000/2000 bytes read @ 0.98 KB/sec")
        );
    }

    // -----------------------------------------------------------------
    // The seeded generator the property cases draw from
    // -----------------------------------------------------------------

    /// How many random scenarios one property runs.
    const CASES: usize = 512;

    /// A seeded xorshift64 generator.
    ///
    /// A failing property has to be replayable from its seed and the crate
    /// takes no dependency, so the generator lives here. It is not a quality
    /// generator: it only has to spread the drawn values widely enough for
    /// the properties to mean something.
    struct Rng(u64);

    impl Rng {
        /// A generator for `seed`. The state must not be zero.
        fn new(seed: u64) -> Self {
            Rng(seed | 1)
        }

        /// The next 64 random bits.
        fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }

        /// A uniform index in `0..n`.
        fn below(&mut self, n: u64) -> u64 {
            self.next_u64() % n
        }

        /// A byte count: zero, one, a file's worth, or anywhere in the range
        /// a `u64` counter can reach.
        fn bytes(&mut self) -> u64 {
            match self.below(8) {
                0 => 0,
                1 => self.below(8),
                2 => self.below(1 << 20),
                3 => u64::MAX - self.below(3),
                4 => u64::MAX / 100,
                5 => u64::MAX / 3,
                _ => self.next_u64(),
            }
        }

        /// A declared length: unknown, zero, a file's worth or the whole
        /// range.
        fn total(&mut self) -> Option<u64> {
            match self.below(8) {
                0 | 1 => None,
                2 => Some(0),
                3 => Some(self.below(1 << 20)),
                4 => Some(u64::MAX),
                _ => Some(self.next_u64()),
            }
        }

        /// The clock step between two samples, from none at all to years.
        fn step(&mut self) -> Duration {
            match self.below(6) {
                0 => Duration::ZERO,
                1 => Duration::from_nanos(self.below(1_000)),
                2 => Duration::from_micros(self.below(1_000)),
                3 => Duration::from_millis(self.below(1_000)),
                4 => Duration::from_secs(self.below(1_000)),
                _ => Duration::from_nanos(self.next_u64()),
            }
        }

        /// One of the seven phases of S19 §2.
        fn phase(&mut self) -> HttpPhase {
            match self.below(7) {
                0 => HttpPhase::Connecting,
                1 => HttpPhase::SendingRequest,
                2 => HttpPhase::ReadingResponse,
                3 => HttpPhase::Retrieving,
                4 => HttpPhase::ConnectionLost,
                5 => HttpPhase::Closed,
                _ => HttpPhase::ConnectFailed,
            }
        }
    }

    /// Every rate S19 §2 writes is `%d.%02d KB/sec`: two decimals, digits and
    /// a point, and never `NaN`, `inf` or `-0.00` — whatever the counters and
    /// the clock did.
    #[test]
    fn the_rate_text_always_has_two_decimals_and_no_non_finite_word() {
        let mut rng = Rng::new(0x5eed_0001);
        for case in 0..CASES {
            let bytes = rng.bytes();
            let elapsed = rng.step();
            let rate = rate_kb_per_sec(bytes, elapsed);
            assert!(
                rate.is_finite(),
                "case {case}: {bytes} bytes in {elapsed:?}"
            );
            assert!(rate >= 0.0, "case {case}: {bytes} bytes in {elapsed:?}");
            let text = format_rate(rate);
            let (whole, fraction) = text.split_once('.').expect("a decimal point");
            assert!(!whole.is_empty(), "case {case}: {text:?}");
            assert!(
                whole.bytes().all(|b| b.is_ascii_digit()),
                "case {case}: {text:?}"
            );
            assert_eq!(fraction.len(), 2, "case {case}: {text:?}");
            assert!(
                fraction.bytes().all(|b| b.is_ascii_digit()),
                "case {case}: {text:?}"
            );
            assert_ne!(text, "-0.00", "case {case}");
        }
    }

    /// An astronomically large rate does not cause UB in `format_rate`.
    ///
    /// The previous implementation cast a potentially huge f64 directly to
    /// `u64`, which is undefined behaviour when the value exceeds `u64::MAX`.
    /// This test exercises the extreme corner: `u64::MAX` bytes in one
    /// nanosecond.
    #[test]
    fn format_rate_handles_astronomical_rates_without_ub() {
        let rate = rate_kb_per_sec(u64::MAX, Duration::from_nanos(1));
        let text = format_rate(rate);
        // The value is clamped to u64::MAX hundredths, so the whole part is
        // u64::MAX / 100 and the fraction is u64::MAX % 100.
        let expected_whole = u64::MAX / 100;
        let expected_frac = u64::MAX % 100;
        assert_eq!(
            text,
            format!("{expected_whole}.{expected_frac:02}"),
            "rate text was clamped correctly"
        );
        // Also verify the general format invariants.
        let (whole, fraction) = text.split_once('.').expect("a decimal point");
        assert!(!whole.is_empty());
        assert!(whole.bytes().all(|b| b.is_ascii_digit()));
        assert_eq!(fraction.len(), 2);
        assert!(fraction.bytes().all(|b| b.is_ascii_digit()));
        assert_ne!(text, "-0.00");
    }

    /// The share is the count over the declared length, and it is a hundred
    /// at most — for every pair of counts a `u64` can hold.
    #[test]
    fn the_share_is_exact_for_every_count_and_total() {
        // 200 PB of a 1 EB body is 20%, not "done": the product used to
        // saturate at `u64::MAX` and report a share of 100.
        assert_eq!(
            percent(200_000_000_000_000_000, 1_000_000_000_000_000_000),
            20
        );
        assert_eq!(percent(0, 0), 0, "no total, no share");
        assert_eq!(percent(u64::MAX, 0), 0, "no total, no share");
        assert_eq!(percent(u64::MAX, 1), 100, "a share stops at a hundred");
        let mut rng = Rng::new(0x5eed_0002);
        for case in 0..CASES {
            let bytes = rng.bytes();
            let total = rng.bytes();
            let expected = if total == 0 {
                0
            } else {
                u128::from(bytes) * 100 / u128::from(total)
            };
            assert_eq!(
                u128::from(percent(bytes, total)),
                expected.min(100),
                "case {case}: {bytes}/{total}"
            );
        }
    }

    /// The share truncates: it never rounds up, so it does not say "done"
    /// while bytes are still missing.
    #[test]
    fn the_share_truncates_at_the_boundaries() {
        let cases = [
            (0u64, "Retrieving file (0% done)"),
            (994, "Retrieving file (99% done)"),
            (995, "Retrieving file (99% done)"),
            (999, "Retrieving file (99% done)"),
            (1000, "Retrieving file (100% done)"),
            (1040, "Retrieving file (100% done)"),
        ];
        for (bytes, text) in cases {
            let snapshot = snapshot(HttpPhase::Retrieving, bytes, Some(1000));
            assert_eq!(phase_text(HttpPhase::Retrieving, &snapshot), text);
        }
        let third = snapshot(HttpPhase::Retrieving, 1, Some(3));
        assert_eq!(
            phase_text(HttpPhase::Retrieving, &third),
            "Retrieving file (33% done)"
        );
        let two_thirds = snapshot(HttpPhase::Retrieving, 2, Some(3));
        assert_eq!(
            phase_text(HttpPhase::Retrieving, &two_thirds),
            "Retrieving file (66% done)"
        );
    }

    /// A server that declares a length of zero breaks nothing: no share to
    /// divide by, and the counters still name the ratio the spec spells.
    #[test]
    fn a_declared_length_of_zero_breaks_nothing() {
        let start = Instant::now();
        let mut status = StreamStatus::default();
        assert_eq!(
            status.update(snapshot(HttpPhase::Retrieving, 0, Some(0)), start),
            Some("Retrieving file (0% done)".to_string())
        );
        assert_eq!(
            status.update(
                snapshot(HttpPhase::Retrieving, 10, Some(0)),
                start + Duration::from_secs(1)
            ),
            None,
            "the phase did not change"
        );
        assert_eq!(
            status.status(),
            Some("Retrieving file (0% done)\n10/0 bytes read @ 0.01 KB/sec")
        );
    }

    /// A body that is longer than the length the server declared is counted
    /// at a hundred; the counters still name the bytes that came in.
    #[test]
    fn a_body_longer_than_its_total_is_counted_at_a_hundred() {
        let start = Instant::now();
        let mut status = StreamStatus::default();
        assert_eq!(
            status.update(snapshot(HttpPhase::Retrieving, 2000, Some(1000)), start),
            Some("Retrieving file (100% done)".to_string())
        );
        assert_eq!(
            status.update(
                snapshot(HttpPhase::Retrieving, 3000, Some(1000)),
                start + Duration::from_secs(1)
            ),
            None
        );
        assert_eq!(
            status.status(),
            Some("Retrieving file (100% done)\n3000/1000 bytes read @ 0.98 KB/sec")
        );
    }

    /// Two samples at the same clock reading have no rate to report, and the
    /// line says so instead of `inf` or `NaN`.
    #[test]
    fn two_samples_with_no_time_between_them_report_a_zero_rate() {
        let start = Instant::now();
        let mut status = StreamStatus::default();
        status.update(snapshot(HttpPhase::Retrieving, 0, None), start);
        status.update(snapshot(HttpPhase::Retrieving, 100_000, None), start);
        assert_eq!(
            status.status(),
            Some("Retrieving file...\n100000 bytes read @ 0.00 KB/sec")
        );
    }

    /// A count that goes back is a new transfer: its phase is reported again
    /// and its rate starts over.
    #[test]
    fn a_count_that_goes_back_starts_a_fresh_transfer() {
        let start = Instant::now();
        let mut status = StreamStatus::default();
        assert_eq!(
            status.update(snapshot(HttpPhase::Retrieving, 5000, None), start),
            Some("Retrieving file...".to_string())
        );
        assert_eq!(
            status.update(
                snapshot(HttpPhase::Retrieving, 6000, None),
                start + Duration::from_secs(1)
            ),
            None,
            "the same phase makes no new note"
        );
        assert_eq!(
            status.update(
                snapshot(HttpPhase::Retrieving, 100, None),
                start + Duration::from_secs(2)
            ),
            Some("Retrieving file...".to_string()),
            "the transfer started over, so its phase is new"
        );
        assert_eq!(
            status.status(),
            None,
            "a new transfer has no rate before its second sample"
        );
        assert_eq!(
            status.update(
                snapshot(HttpPhase::Retrieving, 1124, None),
                start + Duration::from_secs(3)
            ),
            None
        );
        assert_eq!(
            status.status(),
            Some("Retrieving file...\n1124 bytes read @ 1.00 KB/sec")
        );
    }

    /// Every terminal phase is reported once, and its counters go with it.
    #[test]
    fn every_terminal_phase_is_reported_once() {
        let start = Instant::now();
        for phase in [
            HttpPhase::Closed,
            HttpPhase::ConnectionLost,
            HttpPhase::ConnectFailed,
        ] {
            let mut status = StreamStatus::default();
            status.update(snapshot(HttpPhase::Retrieving, 10, None), start);
            let text = phase_text(phase, &snapshot(phase, 10, None));
            assert_eq!(
                status.update(snapshot(phase, 10, None), start + Duration::from_secs(1)),
                Some(text.clone()),
                "the end of the transfer is a note"
            );
            for tick in 2..4 {
                assert_eq!(
                    status.update(snapshot(phase, 10, None), start + Duration::from_secs(tick)),
                    None,
                    "the end of the transfer is reported once"
                );
            }
            assert_eq!(
                status.status(),
                None,
                "the counters exist only while the body is read"
            );
        }
    }

    /// A transfer that comes after a finished one starts a fresh chain: its
    /// first phase is a note even though the last transfer already reported
    /// that same phase.
    #[test]
    fn a_new_transfer_after_a_finished_one_reports_its_first_phase() {
        let start = Instant::now();
        let mut status = StreamStatus::default();
        // A transfer that the user stops while the body is read: the app
        // calls `reset` then, and the last phase it saw was `Retrieving`.
        status.update(snapshot(HttpPhase::Retrieving, 0, None), start);
        status.update(
            snapshot(HttpPhase::Retrieving, 1024, None),
            start + Duration::from_secs(1),
        );
        status.reset();
        assert_eq!(status.status(), None, "a reset withdraws the status");
        // The next transfer is on its way before the app reads it again.
        assert_eq!(
            status.update(
                snapshot(HttpPhase::Retrieving, 0, None),
                start + Duration::from_secs(2)
            ),
            Some("Retrieving file...".to_string()),
            "the old chain must not suppress the new transfer"
        );
    }

    /// A phase that came and went between two ticks is not invented.
    #[test]
    fn a_phase_that_came_and_went_between_two_ticks_is_not_invented() {
        let start = Instant::now();
        let mut status = StreamStatus::default();
        assert_eq!(
            status.update(snapshot(HttpPhase::Connecting, 0, None), start),
            Some("Connecting to host...".to_string())
        );
        // The request and the head were over before the app read again.
        assert_eq!(
            status.update(
                snapshot(HttpPhase::Retrieving, 0, None),
                start + Duration::from_millis(16)
            ),
            Some("Retrieving file...".to_string()),
            "only the phase that is still current is reported"
        );
    }

    /// A note is made exactly when the phase changes or the transfer starts
    /// over, it carries the text of the phase it names, and the status is the
    /// two-line transfer display of the phase that is running.
    #[test]
    fn a_note_is_made_only_on_a_phase_change() {
        let mut rng = Rng::new(0x5eed_0003);
        for case in 0..64 {
            let mut status = StreamStatus::default();
            let mut now = Instant::now();
            let mut last_phase: Option<HttpPhase> = None;
            let mut last_bytes: Option<u64> = None;
            for tick in 0..64 {
                let snapshot = snapshot(rng.phase(), rng.bytes(), rng.total());
                let backwards = last_bytes.is_some_and(|bytes| bytes > snapshot.bytes_read);
                let changed = last_phase != Some(snapshot.phase);
                let note = status.update(snapshot, now);
                assert_eq!(
                    note.is_some(),
                    changed || backwards,
                    "case {case}, tick {tick}: {note:?} for {snapshot:?} after {last_phase:?}/{last_bytes:?}"
                );
                if let Some(note) = note {
                    assert_eq!(
                        note,
                        phase_text(snapshot.phase, &snapshot),
                        "case {case}, tick {tick}: the text of the phase it names"
                    );
                }
                if let Some(line) = status.status() {
                    assert_eq!(
                        snapshot.phase,
                        HttpPhase::Retrieving,
                        "case {case}, tick {tick}: counters only while the body is read"
                    );
                    let (head, counters) = line.split_once('\n').expect("the two-line display");
                    assert_eq!(head, phase_text(HttpPhase::Retrieving, &snapshot));
                    let expected = match snapshot.total {
                        Some(total) => {
                            format!("{}/{} bytes read @ ", snapshot.bytes_read, total)
                        }
                        None => format!("{} bytes read @ ", snapshot.bytes_read),
                    };
                    assert!(
                        counters.starts_with(&expected),
                        "case {case}, tick {tick}: {counters:?}"
                    );
                    let rate = &counters[expected.len()..];
                    let rate = rate.strip_suffix(" KB/sec").expect("the unit");
                    let (whole, fraction) = rate.split_once('.').expect("two decimals");
                    assert!(
                        whole.bytes().all(|b| b.is_ascii_digit())
                            && fraction.len() == 2
                            && fraction.bytes().all(|b| b.is_ascii_digit()),
                        "case {case}, tick {tick}: {rate:?}"
                    );
                }
                last_phase = Some(snapshot.phase);
                last_bytes = Some(snapshot.bytes_read);
                now = now.checked_add(rng.step()).unwrap_or(now);
            }
        }
    }
}
