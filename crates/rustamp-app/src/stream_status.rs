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
fn percent(bytes_read: u64, total: u64) -> u64 {
    if total == 0 {
        return 0;
    }
    (bytes_read.saturating_mul(100) / total).min(100)
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
    let hundredths = (kb_per_sec * 100.0).round().max(0.0) as u64;
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
}
