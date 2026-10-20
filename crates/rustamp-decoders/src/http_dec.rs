//! The decoder that plays an `http://` stream.
//!
//! The decoder is the crate's [`Decoder`] over a socket: the body of the
//! response is the byte source, and the containers and codecs of the crate
//! decode it exactly as they decode a file. A stream is not a file in one
//! respect - it cannot be seeked, and it can end in the middle - and both
//! are handled here:
//!
//! * **Seeking** is refused. A stream has no position to seek to, and the
//!   core contract asks for a live stream to say so
//!   ([`Error::Unsupported`]).
//! * **A connection that dies in the middle** is a **truncated stream**, not a
//!   clean end. The length the server declared (`Content-Length`) is the
//!   oracle: a body that ends before that length is a cut, and the decoder
//!   says so with [`Error::Decode`], in a call that writes no frames - the
//!   frames that did arrive are still delivered. A body that ends where the
//!   server said it ends is the end of the stream.
//!
//! What the transport does is in [`crate::http_source`], and what the
//! transfer is doing is observable in [`crate::http_progress`].

use std::time::Duration;

use rustamp_core::audio::{AudioSpec, Decoder, MediaSource};
use rustamp_core::error::{Error, Result};
use rustamp_core::meta::TrackMetadata;
use rustamp_core::state::StreamInfo;
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};

use crate::http_progress::HttpProgress;
use crate::http_source::{DEFAULT_TIMEOUT, HttpError, HttpSource};
use crate::symphonia_dec::{HeaderProbe, SymphoniaDecoder};

/// A decoder that plays an `http://` stream.
///
/// The stream is opened and the container is probed when the decoder is
/// opened, so a failure of the transport or of the container is reported
/// there and not later.
///
/// What the transfer is doing is observable: the caller hands an
/// [`HttpProgress`] to the open and reads a snapshot out of it whenever it
/// wants to draw the status line (S19 §2).
pub struct HttpDecoder {
    /// The decoder that reads the body of the response.
    inner: SymphoniaDecoder,
    /// What the body, this decoder and the application share: the phase of
    /// the transfer and the counters of the body.
    progress: HttpProgress,
}

impl HttpDecoder {
    /// Opens the stream at `url` and decodes it.
    ///
    /// The timeout is [`DEFAULT_TIMEOUT`]; use
    /// [`HttpDecoder::open_with_timeout`] for another one.
    ///
    /// The failures are typed ([`HttpError`]): a URL that names another
    /// scheme, a server that answers with a status other than 200, a host
    /// that does not answer in time and a body that is not media are four
    /// different things, and a caller can tell them apart.
    pub fn open(url: &str) -> Result<Self, HttpError> {
        Self::open_with_timeout(url, DEFAULT_TIMEOUT)
    }

    /// Opens the stream at `url` with an explicit timeout and decodes it.
    ///
    /// The timeout bounds the connect, the read of the response head and the
    /// gap between two reads of the body (see [`DEFAULT_TIMEOUT`]).
    pub fn open_with_timeout(url: &str, timeout: Duration) -> Result<Self, HttpError> {
        Self::open_with_progress(url, timeout, &HttpProgress::new())
    }

    /// Opens the stream at `url` and reports what it is doing to `progress`.
    ///
    /// The timeout bounds the connect, the read of the response head and the
    /// gap between two reads of the body (see [`DEFAULT_TIMEOUT`]).
    ///
    /// The observer is the caller's: it keeps its own handle for the whole
    /// life of the transfer, and it can read it even when the open fails -
    /// which is when the phase says that the host could not be reached.
    pub fn open_with_progress(
        url: &str,
        timeout: Duration,
        progress: &HttpProgress,
    ) -> Result<Self, HttpError> {
        let source = HttpSource::open(url, timeout, progress)?;
        // The extension of the URL is the hint for the container probe. A URL
        // without one is sniffed by the content, like a file without a name.
        let extension = MediaSource::Url(url.to_string()).extension();
        let stream = MediaSourceStream::new(
            Box::new(source.into_source()),
            MediaSourceStreamOptions::default(),
        );
        // The head of a stream is not known before the stream starts, so the
        // header probe decides nothing: the length the server declared is the
        // oracle (see `HeaderProbe::unknown`).
        let inner = SymphoniaDecoder::open_stream(
            stream,
            progress.total(),
            HeaderProbe::unknown(),
            extension,
        )
        .map_err(HttpError::Media)?;
        Ok(HttpDecoder {
            inner,
            progress: progress.clone(),
        })
    }
}

impl std::fmt::Debug for HttpDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpDecoder")
            .field("spec", &self.inner.spec())
            .field("declared", &self.progress.total())
            .finish_non_exhaustive()
    }
}

impl Decoder for HttpDecoder {
    fn spec(&self) -> AudioSpec {
        self.inner.spec()
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        let result = self.inner.read(out);
        if let Err(Error::Io(_)) = result
            && let Some(message) = cut_message(&self.progress)
        {
            // The connection died before the body was complete: that is a
            // truncated stream and not an I/O hiccup, and the crate names a
            // cut stream like this everywhere else.
            return Err(Error::Decode(message));
        }
        result
    }

    fn seek(&mut self, _to: Duration) -> Result<Duration> {
        Err(Error::Unsupported("seeking in an HTTP stream"))
    }

    fn duration(&self) -> Option<Duration> {
        self.inner.duration()
    }

    fn metadata(&self) -> TrackMetadata {
        self.inner.metadata()
    }

    fn stream_info(&self) -> StreamInfo {
        self.inner.stream_info()
    }

    fn is_seekable(&self) -> bool {
        // A stream has no position to seek to, whatever its container says.
        false
    }

    fn skipped_packets(&self) -> u64 {
        self.inner.skipped_packets()
    }
}

/// The message for a body that ended before the length the server declared.
///
/// `None` when the body was complete: a body that ends where the server said
/// it ends is the end of the stream, not a cut.
fn cut_message(progress: &HttpProgress) -> Option<String> {
    if !progress.is_cut() {
        return None;
    }
    let declared = progress.total().unwrap_or(0);
    Some(format!(
        "truncated stream: the connection closed after {} of the {declared} bytes the server declared",
        progress.bytes_read()
    ))
}
