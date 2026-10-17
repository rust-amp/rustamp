//! Format conversion on the decode path: engine format -> output format.
//!
//! The output device may grant a different sample rate or channel count than
//! the decoder produced (RFC 0001: "the device may grant another rate/channels;
//! the engine adapts"). The converter sits between the DSP chain and the ring,
//! so the ring always holds audio in the **device** format and the render
//! callback never resamples.
//!
//! * **Equal sample rates** take an exact remix path: no filter, no buffering,
//!   no held-back frame. With equal channel counts it is a bit-exact copy,
//!   which the engine's bit-transparency tests depend on.
//! * **Different sample rates** go through the band-limited resampler of the
//!   `rubato` crate (MIT OR Apache-2.0, recorded in `THIRD_PARTY.md`). It
//!   replaces the linear interpolation this module used before, which let
//!   every component above the output Nyquist fold back as an audible alias
//!   and left images of the input spectrum above the input Nyquist.
//! * **Channels:** mono -> stereo duplicates, stereo -> mono averages, extra
//!   input channels are dropped, missing output channels are zero-filled. The
//!   same remix runs after the resampler, so a resampled stereo -> mono stream
//!   is averaged too (the linear path used to take the left channel only).
//!
//! # What is fixed and what is chosen here
//!
//! Fixed by the contracts: the converter adapts to the device format (RFC 0001),
//! the ring holds device-format audio, and an equal-rate conversion is exact.
//! No spec measures the reference player's resampler; **everything below is an
//! own engineering choice**, not 2.95 behaviour:
//!
//! * **Engine: rubato's synchronous FFT resampler** ([`Fft`]) with its default
//!   preset — a `BlackmanHarris2` (squared Blackman-Harris) windowed-sinc
//!   anti-aliasing filter, applied by FFT overlap-add, with FFT blocks of
//!   roughly [`CHUNK_FRAMES`] input frames (rounded up to an even number of
//!   minimum blocks of the reduced ratio, 294 frames for 44.1 kHz -> 48 kHz,
//!   640 for 32 kHz -> 44.1 kHz; see `fft_chunk_hint` for why even). The
//!   device rate and the track rate are both fixed for the life of a run, so
//!   a synchronous resampler fits; rubato documents it as both its fastest and
//!   its highest-quality option for a fixed ratio. The squared window's
//!   sidelobes sit far below the `f32` noise floor, so the stopband is limited
//!   by arithmetic, not by the filter. The trade-off is the transition band:
//!   with this block size the cutoff is about 0.95 of the lower Nyquist (about
//!   21 kHz for 44.1 kHz audio), and the delay stays at a few milliseconds
//!   instead of the tens a longer block would cost.
//! * **Fallback: rubato's asynchronous sinc resampler** ([`Async`]) for rate
//!   pairs whose reduced ratio has a side above [`MAX_FFT_UNIT`] frames
//!   (44100 -> 44101, say). An FFT resampler must transform whole multiples of
//!   that unit, which would cost a huge transform and half a second of delay.
//!   The sinc engine uses rubato's recommended high-quality parameters
//!   ([`SINC_LEN`]-tap `BlackmanHarris2` sinc, cubic interpolation between
//!   [`SINC_OVERSAMPLING`] sub-phases, automatic cutoff), which rubato states
//!   are equivalent in quality to the FFT engine; it is just slower. Its ratio
//!   is fixed (never adjusted), so it behaves as a synchronous converter.
//! * **Delay compensation.** Both engines delay their output by a fixed number
//!   of output frames ([`Resampler::output_delay`]). The converter discards
//!   exactly that many frames after every [`FormatConverter::prepare`] and
//!   [`FormatConverter::reset`], so output frame `k` lines up with input time
//!   `k * src_rate / dst_rate` and the timeline maths downstream stays exact:
//!   the converter introduces **no group delay** into the stream. For the FFT
//!   engine the trimmed delay is the filter's delay exactly, because both FFT
//!   sizes are kept even (`fft_chunk_hint`). The sinc fallback reports its
//!   delay truncated to whole frames, so its output can lead by less than one
//!   frame; that is inaudible and not corrected. What remains is look-ahead:
//!   the last [`FormatConverter::latency_frames`] input frames of a stream
//!   have not produced their output yet; [`FormatConverter::flush`] emits
//!   them at the end of a stream.
//! * **Gapless joins.** A track that follows another in the same format does
//!   not get a fresh resampler: the decode thread neither flushes nor resets
//!   the converter at that seam ([`FormatConverter::continues`]), so the two
//!   tracks are resampled as one stream and the seam is exactly as smooth as
//!   the joined signal. Flushing and re-priming instead would cut track A's
//!   filter ringing off at the seam and fade track B in from silence, which
//!   is a dip to about half the level at the seam. A seam between two formats
//!   still flushes the first and primes the second: two rates cannot share a
//!   resampler.
//! * **Fixed-size chunks.** The resampler is driven with chunks of a fixed
//!   number of input frames ([`CHUNK_FRAMES`] for most pairs, see
//!   `fft_chunk_hint`) collected in an internal FIFO, so the
//!   sequence of resampler calls — and hence every output sample — is the same
//!   however the caller splits the input into blocks. The phase is carried by
//!   rubato's own state across those calls, so no frame is dropped or repeated
//!   at a block seam.
//! * **Precision:** the resampler runs in `f32`, the format of the whole path.
//! * **Final ceiling (own choice).** On the resampled path every output frame
//!   is passed through [`finish_sample`]: non-finite samples become silence,
//!   subnormals become zero, and anything above full scale is clamped to
//!   `±1.0` (0 dBFS). The limiter before the converter can only promise a
//!   *sample*-peak ceiling at its own output, and the band-limited resampler
//!   rebuilds inter-sample peaks above it (A6 finding 3: about +2.7 dBFS on a
//!   simple tone, more on pathological sequences). Volume and balance after
//!   the converter only attenuate (see `gain`), so this is the last point
//!   that can raise the level — a bound here is a bound at the device, and the
//!   ring never holds a sample above full scale. The bound is `1.0`, not the
//!   limiter's −0.3 dB ceiling: the limiter's ceiling is a dynamics choice, and
//!   legitimate near-full-scale peaks must not be touched by the safety net.
//!   The equal-rate path is untouched, so its bit-exact promise stands.
//!
//! # Real-time safety
//!
//! [`FormatConverter::prepare`] builds the resampler and allocates its FIFO and
//! output block; it may allocate. [`FormatConverter::convert`],
//! [`FormatConverter::flush`] and [`FormatConverter::reset`] allocate nothing
//! and take no lock: rubato's `process_into_buffer` is documented as
//! allocation-free and non-blocking, and the buffers handed to it are borrowed
//! views of the converter's own storage.

use std::fmt;

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Async, Fft, FixedAsync, FixedSync, Indexing, Resampler, SincInterpolationParameters,
    WindowFunction,
};
use rustamp_core::audio::AudioSpec;

/// Input frames handed to the resampler per call (own choice). It is also the
/// FFT block size the FFT engine aims for, which sets its filter length.
pub const CHUNK_FRAMES: usize = 256;

/// Largest reduced-ratio side (`rate / gcd(src, dst)`) the FFT engine accepts;
/// above it the sinc engine is used (own choice). All common pairs among
/// 8, 11.025, 16, 22.05, 32, 44.1, 48, 88.2, 96, 176.4 and 192 kHz stay well
/// below it (44.1 kHz -> 48 kHz reduces to 147 -> 160).
pub const MAX_FFT_UNIT: u64 = 2048;

/// Sinc filter length of the fallback engine (rubato's recommended value).
pub const SINC_LEN: usize = 256;

/// Sub-phase count of the fallback engine's sinc table (own choice, twice
/// rubato's default, for a lower interpolation floor).
pub const SINC_OVERSAMPLING: usize = 256;

/// The rubato engine in use for a resampling conversion.
enum Engine {
    Fft(Box<Fft<f32>>),
    Sinc(Box<Async<f32>>),
}

impl Engine {
    /// Build the engine for `rate_in -> rate_out`; `up / down` is the ratio in
    /// lowest terms.
    fn new(rate_in: u32, rate_out: u32, up: u64, down: u64, channels: usize) -> Option<Engine> {
        if up.max(down) <= MAX_FFT_UNIT
            && let Ok(fft) = Fft::<f32>::new_custom(
                rate_in as usize,
                rate_out as usize,
                fft_chunk_hint(down),
                1,
                channels,
                WindowFunction::BlackmanHarris2,
                FixedSync::Input,
            )
        {
            return Some(Engine::Fft(Box::new(fft)));
        }
        let params = SincInterpolationParameters::new(SINC_LEN, WindowFunction::BlackmanHarris2)
            .oversampling_factor(SINC_OVERSAMPLING);
        Async::<f32>::new_sinc(
            up as f64 / down as f64,
            1.0,
            &params,
            CHUNK_FRAMES,
            channels,
            FixedAsync::Input,
        )
        .ok()
        .map(|sinc| Engine::Sinc(Box::new(sinc)))
    }

    fn resampler(&mut self) -> &mut dyn Resampler<f32> {
        match self {
            Engine::Fft(fft) => fft.as_mut(),
            Engine::Sinc(sinc) => sinc.as_mut(),
        }
    }

    fn resampler_ref(&self) -> &dyn Resampler<f32> {
        match self {
            Engine::Fft(fft) => fft.as_ref(),
            Engine::Sinc(sinc) => sinc.as_ref(),
        }
    }

    /// Input frames the engine can hold internally beyond its reported delay:
    /// the FFT engine buffers up to one FFT block, the sinc engine up to one
    /// filter length.
    fn internal_frames(&self) -> usize {
        match self {
            Engine::Fft(fft) => fft.fft_size_in(),
            Engine::Sinc(_) => SINC_LEN,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Engine::Fft(_) => "fft",
            Engine::Sinc(_) => "sinc",
        }
    }
}

/// Converts interleaved `f32` frames from one [`AudioSpec`] to another.
pub struct FormatConverter {
    src: AudioSpec,
    dst: AudioSpec,
    /// True when no resampling is needed (only a possible channel remap).
    same_rate: bool,
    /// The rate ratio in lowest terms: `up` output frames per `down` input
    /// frames.
    up: u64,
    down: u64,
    /// The resampler; `None` for equal rates.
    engine: Option<Engine>,
    /// Input frames per resampler call (constant for the configured engine).
    chunk: usize,
    /// Largest number of output frames one resampler call can produce.
    block_max: usize,
    /// The engine's delay in output frames, trimmed after every reset.
    delay: usize,
    /// See [`Engine::internal_frames`].
    internal: usize,
    /// Interleaved input FIFO, `chunk * src.channels` values.
    fifo: Vec<f32>,
    /// Frames currently held in `fifo`.
    fifo_frames: usize,
    /// Interleaved resampler output, `block_max * src.channels` values.
    block: Vec<f32>,
    /// Start-up delay frames still to be discarded.
    to_trim: usize,
    /// Input frames accepted since the last reset.
    in_total: u64,
    /// Output frames emitted since the last reset.
    out_total: u64,
}

impl fmt::Debug for FormatConverter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FormatConverter")
            .field("src", &self.src)
            .field("dst", &self.dst)
            .field("engine", &self.engine.as_ref().map(Engine::name))
            .field("chunk", &self.chunk)
            .field("block_max", &self.block_max)
            .field("delay", &self.delay)
            .field("fifo_frames", &self.fifo_frames)
            .field("in_total", &self.in_total)
            .field("out_total", &self.out_total)
            .finish()
    }
}

/// The chunk size to ask the FFT engine for, with one FFT sub-chunk.
///
/// rubato makes the FFT block `ceil(chunk / down)` minimum blocks long, that
/// is `blocks * down` input and `blocks * up` output frames, and delays the
/// output by `fft_size_out / 2` frames around a filter centred at
/// `floor(fft_size_in / 2)` input frames. Both halves are whole and agree
/// only when both sizes are even. `up` and `down` are coprime, so at least
/// one of them is odd, and that takes an even block count. [`CHUNK_FRAMES`]
/// already gives one for most pairs (44.1 -> 48 kHz: two blocks of 147); for
/// the others (32 -> 44.1 kHz: one block of 320) the block count is rounded
/// up to the next even number, which doubles that pair's FFT block and delay.
fn fft_chunk_hint(down: u64) -> usize {
    let blocks = (CHUNK_FRAMES as u64).div_ceil(down.max(1));
    if blocks.is_multiple_of(2) {
        CHUNK_FRAMES
    } else {
        ((blocks + 1) * down) as usize
    }
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

impl FormatConverter {
    /// A converter for `src -> dst`.
    pub fn new(src: AudioSpec, dst: AudioSpec) -> Self {
        let mut this = FormatConverter {
            src: AudioSpec::new(0, 0),
            dst,
            same_rate: true,
            up: 1,
            down: 1,
            engine: None,
            chunk: 0,
            block_max: 0,
            delay: 0,
            internal: 0,
            fifo: Vec::new(),
            fifo_frames: 0,
            block: Vec::new(),
            to_trim: 0,
            in_total: 0,
            out_total: 0,
        };
        this.prepare(src, dst);
        this
    }

    /// (Re)configure for a new pair of formats. Called at track start and on a
    /// mid-stream format change; builds the resampler and may allocate.
    pub fn prepare(&mut self, src: AudioSpec, dst: AudioSpec) {
        let src = AudioSpec::new(src.sample_rate.max(1), src.channels.max(1));
        let dst = AudioSpec::new(dst.sample_rate.max(1), dst.channels.max(1));
        self.src = src;
        self.dst = dst;
        self.same_rate = src.sample_rate == dst.sample_rate;
        let rate_in = u64::from(src.sample_rate);
        let rate_out = u64::from(dst.sample_rate);
        let g = gcd(rate_in, rate_out);
        self.down = rate_in / g;
        self.up = rate_out / g;

        self.engine = None;
        self.chunk = 0;
        self.block_max = 0;
        self.delay = 0;
        self.internal = 0;
        self.fifo = Vec::new();
        self.block = Vec::new();
        if !self.same_rate {
            let channels = usize::from(src.channels);
            if let Some(engine) = Engine::new(
                src.sample_rate,
                dst.sample_rate,
                self.up,
                self.down,
                channels,
            ) {
                let resampler = engine.resampler_ref();
                self.chunk = resampler.input_frames_next().max(1);
                self.block_max = resampler.output_frames_max();
                self.delay = resampler.output_delay();
                self.internal = engine.internal_frames();
                self.fifo = vec![0.0; self.chunk * channels];
                self.block = vec![0.0; self.block_max * channels];
                self.engine = Some(engine);
            }
        }
        self.reset();
    }

    /// Formats in use.
    pub fn specs(&self) -> (AudioSpec, AudioSpec) {
        (self.src, self.dst)
    }

    /// Whether `prepare(src, dst)` would configure exactly the conversion in
    /// use. Then a new track can continue the running stream (a gapless join)
    /// instead of flushing it and starting a fresh one; see the module docs.
    pub fn continues(&self, src: AudioSpec, dst: AudioSpec) -> bool {
        let src = AudioSpec::new(src.sample_rate.max(1), src.channels.max(1));
        let dst = AudioSpec::new(dst.sample_rate.max(1), dst.channels.max(1));
        src == self.src && dst == self.dst
    }

    /// Output frames the input since the last reset maps to:
    /// `ceil(input_frames * dst_rate / src_rate)`, or the input frame count
    /// for equal rates. This is where the next input frame lands on the
    /// output timeline, counted from the last reset, whether or not its
    /// output has left the converter yet; [`Self::flush`] stops exactly here.
    pub fn stream_frames(&self) -> u64 {
        if self.same_rate {
            self.in_total
        } else {
            self.expected_total()
        }
    }

    /// Upper bound of output frames for `in_frames` input frames.
    ///
    /// This is the one place that predicts the output length. A call appends
    /// its input to a FIFO that holds at most `chunk - 1` frames, so it runs
    /// the resampler at most `(chunk - 1 + in_frames) / chunk` times — that is,
    /// `ceil(in_frames / chunk)` — and one run produces at most the engine's
    /// `output_frames_max`. The trimmed start-up delay only ever lowers the
    /// count.
    pub fn max_out_frames(&self, in_frames: usize) -> usize {
        if self.same_rate || self.engine.is_none() {
            return in_frames;
        }
        in_frames.div_ceil(self.chunk) * self.block_max
    }

    /// Maximum number of output frames a single call can need for `in_frames`.
    pub fn capacity_frames(&self, in_frames: usize) -> usize {
        self.max_out_frames(in_frames)
    }

    /// Input frames whose output can still be pending inside the converter:
    /// the FIFO, the engine's internal buffer and its delay. `0` for equal
    /// rates, which hold nothing back.
    ///
    /// This is look-ahead, not group delay: the output is time-aligned with
    /// the input (see the module docs), these frames just come out later — or
    /// from [`Self::flush`] at the end of a stream.
    pub fn latency_frames(&self) -> usize {
        if self.same_rate || self.engine.is_none() {
            return 0;
        }
        let delay_in = (self.delay as u64 * self.down).div_ceil(self.up) as usize;
        self.chunk - 1 + self.internal + delay_in + 1
    }

    /// Clear the resampler state (after a seek): the next output frame is
    /// again aligned with the first frame of the next input block, and nothing
    /// of the previous audio leaks into it. Allocation-free.
    pub fn reset(&mut self) {
        if let Some(engine) = self.engine.as_mut() {
            engine.resampler().reset();
        }
        self.fifo_frames = 0;
        self.to_trim = self.delay;
        self.in_total = 0;
        self.out_total = 0;
    }

    /// Convert `input` (whole frames in the source format) into `out` (device
    /// frames). Returns the number of output frames written; `out` must hold at
    /// least [`Self::max_out_frames`] frames for the input length.
    ///
    /// When resampling, input is collected into fixed chunks and the
    /// resampler's look-ahead is held back (see [`Self::latency_frames`]); it
    /// comes out of later calls or out of [`Self::flush`]. The output is the
    /// same, sample for sample, however the input is split into blocks.
    ///
    /// Allocation-free and lock-free.
    pub fn convert(&mut self, input: &[f32], out: &mut [f32]) -> usize {
        let in_ch = usize::from(self.src.channels);
        let out_ch = usize::from(self.dst.channels);
        let in_frames = input.len() / in_ch;
        if in_frames == 0 {
            return 0;
        }
        if self.same_rate {
            let out_capacity = out.len() / out_ch;
            let n = in_frames.min(out_capacity);
            remix(&input[..n * in_ch], &mut out[..n * out_ch], in_ch, out_ch);
            self.in_total += n as u64;
            return n;
        }
        if self.engine.is_none() {
            return 0;
        }

        let mut written = 0usize;
        let mut consumed = 0usize;
        while consumed < in_frames {
            let take = (self.chunk - self.fifo_frames).min(in_frames - consumed);
            let at = self.fifo_frames * in_ch;
            let in_slice = &input[consumed * in_ch..(consumed + take) * in_ch];
            for (dst, &s) in self.fifo[at..at + take * in_ch].iter_mut().zip(in_slice) {
                *dst = if s.is_finite() { s } else { 0.0 };
            }
            self.fifo_frames += take;
            self.in_total += take as u64;
            consumed += take;
            if self.fifo_frames == self.chunk {
                written += self.run_block(self.chunk, &mut out[written * out_ch..], None);
                self.fifo_frames = 0;
            }
        }
        written
    }

    /// Emit the held-back tail at the end of a stream, then reset. The input
    /// is padded with silence and the output stops at exactly
    /// `ceil(input_frames * dst_rate / src_rate)` frames for the whole stream,
    /// so a resampled stream has the length its duration implies.
    ///
    /// `out` must hold `max_out_frames(2 * latency_frames())` frames. Returns
    /// the number of frames written; `0` for equal rates. Allocation-free.
    pub fn flush(&mut self, out: &mut [f32]) -> usize {
        if self.same_rate || self.engine.is_none() {
            self.reset();
            return 0;
        }
        let out_ch = usize::from(self.dst.channels);
        let end = self.expected_total();
        let mut written = 0usize;
        if self.fifo_frames > 0 {
            let valid = self.fifo_frames;
            written += self.run_block(valid, &mut out[written * out_ch..], Some(end));
            self.fifo_frames = 0;
        }
        // Every pump feeds a chunk of silence and produces about
        // `chunk * up / down` frames; the bound only guards against an engine
        // that stops producing.
        let max_pumps = 2 * self.latency_frames() / self.chunk + 4;
        let mut pumps = 0;
        while self.out_total < end && pumps < max_pumps && written * out_ch < out.len() {
            written += self.run_block(0, &mut out[written * out_ch..], Some(end));
            pumps += 1;
        }
        debug_assert_eq!(self.out_total, end, "flush output buffer too small");
        self.reset();
        written
    }

    /// Output frames the whole stream since the last reset maps to.
    fn expected_total(&self) -> u64 {
        (u128::from(self.in_total) * u128::from(self.up)).div_ceil(u128::from(self.down)) as u64
    }

    /// Run the resampler on the FIFO, of which the first `valid` frames are
    /// real input (rubato pads the rest with silence), trim the start-up delay
    /// and remix into `out`. With `end`, stop at that many output frames since
    /// the reset.
    fn run_block(&mut self, valid: usize, out: &mut [f32], end: Option<u64>) -> usize {
        let in_ch = usize::from(self.src.channels);
        let out_ch = usize::from(self.dst.channels);
        let Some(engine) = self.engine.as_mut() else {
            return 0;
        };
        let produced = {
            let Ok(input) = InterleavedSlice::new(&self.fifo[..], in_ch, self.chunk) else {
                return 0;
            };
            let Ok(mut output) =
                InterleavedSlice::new_mut(&mut self.block[..], in_ch, self.block_max)
            else {
                return 0;
            };
            let mut indexing = Indexing::new();
            if valid < self.chunk {
                indexing = indexing.partial_len(valid);
            }
            // The buffers are sized from the engine's own limits in `prepare`,
            // so an error here is a broken invariant; losing one chunk is the
            // only answer that keeps the decode thread alive.
            match engine
                .resampler()
                .process_into_buffer(&input, &mut output, Some(&indexing))
            {
                Ok((_, produced)) => produced.min(self.block_max),
                Err(_) => 0,
            }
        };

        let skip = self.to_trim.min(produced);
        self.to_trim -= skip;
        let mut n = produced - skip;
        if let Some(end) = end {
            n = n.min(end.saturating_sub(self.out_total) as usize);
        }
        let room = out.len() / out_ch;
        debug_assert!(n <= room, "output buffer smaller than max_out_frames");
        n = n.min(room);
        remix(
            &self.block[skip * in_ch..(skip + n) * in_ch],
            &mut out[..n * out_ch],
            in_ch,
            out_ch,
        );
        // The final ceiling (see the module docs). This path only runs when
        // resampling: the equal-rate path returns before reaching `run_block`
        // and keeps its bit-exact promise for every input, including
        // subnormals.
        for s in &mut out[..n * out_ch] {
            *s = finish_sample(*s);
        }
        self.out_total += n as u64;
        n
    }
}

/// The last level decision in the chain, on the resampled path only: silence
/// for non-finite samples, zero for subnormals, and a hard clamp at full
/// scale. Finite in-band samples pass through bit for bit.
fn finish_sample(x: f32) -> f32 {
    if !x.is_finite() || x.is_subnormal() {
        0.0
    } else {
        x.clamp(-1.0, 1.0)
    }
}

/// Channel remap without resampling.
fn remix(input: &[f32], out: &mut [f32], in_ch: usize, out_ch: usize) {
    if in_ch == out_ch {
        out.copy_from_slice(&input[..out.len()]);
        return;
    }
    for (i, frame) in input.chunks_exact(in_ch).enumerate() {
        let dst = &mut out[i * out_ch..(i + 1) * out_ch];
        if in_ch == 1 {
            dst.fill(frame[0]);
        } else if out_ch == 1 {
            let sum: f32 = frame.iter().sum();
            dst[0] = sum / in_ch as f32;
        } else {
            for (c, v) in dst.iter_mut().enumerate() {
                *v = frame.get(c).copied().unwrap_or(0.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn ramp(n: usize) -> Vec<f32> {
        (0..n).map(|i| i as f32).collect()
    }

    /// Deterministic white-ish noise in `-0.5..0.5`, with a few exact edge
    /// values (signed zeros, full scale) mixed in.
    fn noise(samples: usize) -> Vec<f32> {
        let mut seed: u32 = 0x1234_5678;
        let mut out: Vec<f32> = (0..samples)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
            })
            .collect();
        for (i, edge) in [0.0, -0.0, 1.0, -1.0].into_iter().enumerate() {
            if let Some(slot) = out.get_mut(i * 7) {
                *slot = edge;
            }
        }
        out
    }

    fn bits(buf: &[f32]) -> Vec<u32> {
        buf.iter().map(|s| s.to_bits()).collect()
    }

    /// One second of a mono sine at `rate`, starting at phase zero.
    fn sine(freq: f64, amplitude: f64, rate: u32) -> Vec<f32> {
        (0..rate as usize)
            .map(|n| (amplitude * (2.0 * PI * freq * n as f64 / f64::from(rate)).sin()) as f32)
            .collect()
    }

    /// Run `input` through `c` in blocks of `block` frames, checking every
    /// call against `max_out_frames`, then flush.
    fn run(c: &mut FormatConverter, input: &[f32], block: usize) -> Vec<f32> {
        let (src, dst) = c.specs();
        let in_ch = usize::from(src.channels);
        let out_ch = usize::from(dst.channels);
        let mut all = Vec::new();
        let mut buf = vec![0.0; c.max_out_frames(block) * out_ch];
        for chunk in input.chunks(block * in_ch) {
            let n = c.convert(chunk, &mut buf);
            assert!(
                n <= c.max_out_frames(chunk.len() / in_ch),
                "{n} frames exceed the predicted bound"
            );
            all.extend_from_slice(&buf[..n * out_ch]);
        }
        let mut tail = vec![0.0; c.max_out_frames(2 * c.latency_frames()) * out_ch];
        let n = c.flush(&mut tail);
        all.extend_from_slice(&tail[..n * out_ch]);
        all
    }

    fn resample(src_rate: u32, dst_rate: u32, input: &[f32], block: usize) -> Vec<f32> {
        let mut c = FormatConverter::new(AudioSpec::new(src_rate, 1), AudioSpec::new(dst_rate, 1));
        run(&mut c, input, block)
    }

    /// The settled middle half of one second of output at `rate`: a whole
    /// number of periods for every even frequency used below.
    fn settled(out: &[f32], rate: u32) -> &[f32] {
        let rate = rate as usize;
        &out[rate / 4..rate / 4 + rate / 2]
    }

    /// Correlation estimate of `a * sin(w n + phase)` at `freq`, with `n`
    /// counted from `offset` frames into the stream: `(amplitude, phase)`.
    fn fit(y: &[f32], offset: usize, freq: f64, rate: u32) -> (f64, f64) {
        let w = 2.0 * PI * freq / f64::from(rate);
        let (mut s, mut c) = (0.0, 0.0);
        for (i, v) in y.iter().enumerate() {
            let (sin, cos) = (w * (offset + i) as f64).sin_cos();
            s += f64::from(*v) * sin;
            c += f64::from(*v) * cos;
        }
        let n = y.len() as f64;
        (2.0 * (s * s + c * c).sqrt() / n, c.atan2(s))
    }

    /// Level in dB of what is left after removing the fitted sine at `freq`,
    /// relative to a sine of `amplitude`.
    fn residual_db(y: &[f32], offset: usize, freq: f64, rate: u32, amplitude: f64) -> f64 {
        let (a, phase) = fit(y, offset, freq, rate);
        let w = 2.0 * PI * freq / f64::from(rate);
        let power = y
            .iter()
            .enumerate()
            .map(|(i, v)| (f64::from(*v) - a * (w * (offset + i) as f64 + phase).sin()).powi(2))
            .sum::<f64>()
            / y.len() as f64;
        10.0 * (power / (amplitude * amplitude / 2.0)).log10()
    }

    /// Level in dB of `y` relative to a sine of `amplitude`.
    fn rms_db(y: &[f32], amplitude: f64) -> f64 {
        let power = y.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / y.len() as f64;
        10.0 * (power / (amplitude * amplitude / 2.0)).log10()
    }

    #[test]
    fn same_format_is_a_bit_exact_passthrough() {
        let spec = AudioSpec::new(44100, 2);
        let mut c = FormatConverter::new(spec, spec);
        let input = ramp(20);
        let mut out = vec![0.0; c.max_out_frames(10) * 2];
        let n = c.convert(&input, &mut out);
        assert_eq!(n, 10);
        assert_eq!(&out[..20], &input[..]);
    }

    #[test]
    fn equal_rate_path_is_bit_exact_and_holds_nothing_back() {
        let spec = AudioSpec::new(48_000, 2);
        let input = noise(4096 * 2);
        for block in [1, 7, 1024, 4096] {
            let mut c = FormatConverter::new(spec, spec);
            assert_eq!(c.latency_frames(), 0);
            assert_eq!(c.max_out_frames(block), block);
            let out = run(&mut c, &input, block);
            assert_eq!(bits(&out), bits(&input), "block {block}");
        }
    }

    #[test]
    fn mono_to_stereo_duplicates() {
        let mut c = FormatConverter::new(AudioSpec::new(8000, 1), AudioSpec::new(8000, 2));
        let input = ramp(4);
        let mut out = vec![0.0; 16];
        let n = c.convert(&input, &mut out);
        assert_eq!(n, 4);
        assert_eq!(&out[..8], &[0.0, 0.0, 1.0, 1.0, 2.0, 2.0, 3.0, 3.0]);
    }

    #[test]
    fn stereo_to_mono_averages() {
        let mut c = FormatConverter::new(AudioSpec::new(8000, 2), AudioSpec::new(8000, 1));
        let input = vec![1.0, 3.0, 2.0, 4.0];
        let mut out = vec![0.0; 8];
        let n = c.convert(&input, &mut out);
        assert_eq!(n, 2);
        assert_eq!(&out[..2], &[2.0, 3.0]);
    }

    /// Restated from `upsample_2x_interpolates_the_midpoint`, which pinned the
    /// linear interpolator's straight-line midpoints (`0, 5, 10, 15, ...` for
    /// the input `0, 10, 20, 30`). A straight line between two samples is not
    /// the band-limited signal they represent: for a sine it leaves an image
    /// at `fs_in - f` that is only about 15 dB down. The quality bound asks for
    /// the band-limited waveform itself: a clean sine at the right level and
    /// time, with everything else at least 70 dB down.
    #[test]
    fn upsample_2x_reproduces_the_band_limited_waveform() {
        let out = resample(4_000, 8_000, &sine(1_000.0, 0.5, 4_000), 1024);
        assert_eq!(out.len(), 8_000, "exactly twice the input length");
        let y = settled(&out, 8_000);
        let offset = 2_000;
        let (a, phase) = fit(y, offset, 1_000.0, 8_000);
        assert!((20.0 * (a / 0.5).log10()).abs() < 0.05, "level {a}");
        // Time-aligned within a quarter of an output frame.
        let quarter_frame = 2.0 * PI * 1_000.0 / 8_000.0 / 4.0;
        assert!(phase.abs() < quarter_frame, "phase {phase}");
        let residual = residual_db(y, offset, 1_000.0, 8_000, 0.5);
        assert!(residual < -70.0, "residual {residual:.1} dB");
    }

    /// Restated from `downsample_2x_keeps_every_other_frame`, which pinned
    /// plain decimation of a ramp (`0, 2, 4, 6`). Dropping every other frame
    /// keeps an in-band tone but folds everything between the new and the old
    /// Nyquist back into the band at full level. The quality bound keeps the
    /// in-band tone clean and rejects the one that would alias.
    #[test]
    fn downsample_2x_keeps_the_band_and_rejects_the_alias() {
        let band = resample(8_000, 4_000, &sine(500.0, 0.5, 8_000), 1024);
        assert_eq!(band.len(), 4_000, "exactly half the input length");
        let y = settled(&band, 4_000);
        let (a, _) = fit(y, 1_000, 500.0, 4_000);
        assert!((20.0 * (a / 0.5).log10()).abs() < 0.05, "in-band level {a}");
        let residual = residual_db(y, 1_000, 500.0, 4_000, 0.5);
        assert!(residual < -70.0, "in-band residual {residual:.1} dB");

        // 3 kHz is above the 2 kHz output Nyquist: decimation would put it at
        // 1 kHz at full level.
        let alias = resample(8_000, 4_000, &sine(3_000.0, 0.5, 8_000), 1024);
        let level = rms_db(settled(&alias, 4_000), 0.5);
        assert!(level < -70.0, "alias at {level:.1} dB");
    }

    #[test]
    fn passband_is_flat_from_44100_to_48000() {
        // Tolerance: +-0.05 dB up to 16 kHz (0.73 of the 22.05 kHz Nyquist).
        for freq in [100.0, 1_000.0, 5_000.0, 10_000.0, 16_000.0] {
            let out = resample(44_100, 48_000, &sine(freq, 0.5, 44_100), 1024);
            assert_eq!(out.len(), 48_000);
            let (a, _) = fit(settled(&out, 48_000), 12_000, freq, 48_000);
            let db = 20.0 * (a / 0.5).log10();
            assert!(db.abs() < 0.05, "{freq} Hz: {db:.4} dB");
        }
    }

    #[test]
    fn a_tone_above_the_output_nyquist_is_rejected() {
        // 48 kHz -> 44.1 kHz: 23.5 kHz is legal at the input and above the
        // 22.05 kHz output Nyquist; unfiltered it would alias to 20.6 kHz.
        let out = resample(48_000, 44_100, &sine(23_500.0, 0.5, 48_000), 1024);
        assert_eq!(out.len(), 44_100);
        let level = rms_db(settled(&out, 44_100), 0.5);
        assert!(level < -70.0, "alias at {level:.1} dB");
    }

    #[test]
    fn upsampling_22050_content_leaves_no_image() {
        // 9 kHz at 22.05 kHz has an image at 13.05 kHz once the rate is
        // raised; a band-limited resampler must leave only the tone.
        let out = resample(22_050, 48_000, &sine(9_000.0, 0.5, 22_050), 1024);
        assert_eq!(out.len(), 48_000);
        let y = settled(&out, 48_000);
        let (a, _) = fit(y, 12_000, 9_000.0, 48_000);
        assert!((20.0 * (a / 0.5).log10()).abs() < 0.05, "level {a}");
        let residual = residual_db(y, 12_000, 9_000.0, 48_000, 0.5);
        assert!(residual < -70.0, "image/residual {residual:.1} dB");
    }

    #[test]
    fn output_is_time_aligned_with_the_exact_length() {
        // 44.1 -> 48 kHz is 147:160, not an integer ratio. 32 <-> 44.1 kHz
        // (320:441) has a minimum block above CHUNK_FRAMES; with one block
        // the FFT sizes were odd and the output lagged by half a frame
        // (32 -> 44.1) or led by 0.36 frame (44.1 -> 32), which this bound
        // rejects (A6 finding 6).
        for (src, dst) in [(44_100, 48_000), (32_000, 44_100), (44_100, 32_000)] {
            let out = resample(src, dst, &sine(1_000.0, 0.5, src), 1024);
            assert_eq!(out.len(), dst as usize);
            let offset = dst as usize / 4;
            let (_, phase) = fit(settled(&out, dst), offset, 1_000.0, dst);
            let quarter_frame = 2.0 * PI * 1_000.0 / f64::from(dst) / 4.0;
            // A fifth of a frame: the trim is exact, so only the fit's own
            // noise remains, and the old errors (0.36 and 0.5) are rejected.
            assert!(
                phase.abs() < quarter_frame * 0.8,
                "{src}->{dst}: phase {phase}"
            );
        }

        // Lengths that do not divide evenly round up: 1001 * 160 / 147.
        let short = resample(44_100, 48_000, &ramp(1_001), 1024);
        assert_eq!(short.len(), (1_001_usize * 160).div_ceil(147));
    }

    #[test]
    fn fft_blocks_are_even_on_both_sides() {
        for (src, dst) in [
            (44_100, 48_000),
            (48_000, 44_100),
            (32_000, 44_100),
            (44_100, 32_000),
            (8_000, 11_025),
            (22_050, 48_000),
            (4_000, 8_000),
            (8_000, 4_000),
        ] {
            let c = FormatConverter::new(AudioSpec::new(src, 1), AudioSpec::new(dst, 1));
            let Some(Engine::Fft(fft)) = c.engine.as_ref() else {
                panic!("{src}->{dst}: expected the FFT engine");
            };
            let (fin, fout) = (fft.fft_size_in(), fft.fft_size_out());
            assert!(fin % 2 == 0 && fout % 2 == 0, "{src}->{dst}: {fin}/{fout}");
            // The trimmed delay is then exactly the filter's centre.
            assert_eq!(c.delay * fin, (fin / 2) * fout, "{src}->{dst}");
        }
    }

    /// A6 finding 1 at converter level. Two halves of one DC signal,
    /// converted as one continued stream, are the joined signal converted at
    /// once, sample for sample. The control half shows what the measurement
    /// catches: flushing after the first half and starting the second from a
    /// reset converter loses the filter ringing each half owes the other
    /// across the seam.
    ///
    /// The size of that error is estimated, not measured: the last frame of
    /// the first half lands 0.08 input frames after its last sample, where
    /// the truncated step response is about 6 % low (0.47 instead of 0.5),
    /// and the first frame of the second half about 2.5 % low. The review
    /// estimated a dip to half the level; at the sample instants of this rate
    /// pair the band-limited step is still near full level, so the error is
    /// smaller. The control only asserts that it is far above the
    /// resampler's own error (5e-3 against 1e-4).
    #[test]
    fn a_continued_stream_has_no_seam() {
        let half = vec![0.5_f32; 4_410];
        let joined = resample(44_100, 48_000, &[half.clone(), half.clone()].concat(), 1024);
        assert_eq!(joined.len(), 9_600);

        let spec_in = AudioSpec::new(44_100, 1);
        let spec_out = AudioSpec::new(48_000, 1);
        let mut c = FormatConverter::new(spec_in, spec_out);
        let mut buf = vec![0.0; c.max_out_frames(4_410)];
        let mut continued = Vec::new();
        let n = c.convert(&half, &mut buf);
        continued.extend_from_slice(&buf[..n]);
        assert!(c.continues(spec_in, spec_out));
        assert_eq!(c.stream_frames(), 4_800, "the seam's place on the output");
        let n = c.convert(&half, &mut buf);
        continued.extend_from_slice(&buf[..n]);
        let mut tail = vec![0.0; c.max_out_frames(2 * c.latency_frames())];
        let n = c.flush(&mut tail);
        continued.extend_from_slice(&tail[..n]);
        assert_eq!(bits(&continued), bits(&joined));
        // Away from the stream's own edges the level is the DC level.
        assert!(
            joined[1_000..8_600].iter().all(|v| (v - 0.5).abs() < 1e-4),
            "no dip at the seam"
        );

        // Control: flush and restart at the seam.
        let a = resample(44_100, 48_000, &half, 1024);
        let b = resample(44_100, 48_000, &half, 1024);
        assert_eq!(a.len() + b.len(), joined.len());
        let restarted = [a, b].concat();
        let worst = restarted[4_600..5_000]
            .iter()
            .zip(&joined[4_600..5_000])
            .map(|(r, j)| (r - j).abs())
            .fold(0.0_f32, f32::max);
        assert!(worst > 5e-3, "a restarted seam deviates: {worst}");
    }

    #[test]
    fn phase_carries_across_blocks() {
        // However the input is split, the output is the same, sample for
        // sample: no frame is dropped or duplicated at a seam.
        for (src, dst) in [(3_000, 2_000), (44_100, 48_000), (48_000, 44_100)] {
            let input = sine(440.0, 0.5, src);
            let whole = resample(src, dst, &input, src as usize);
            assert_eq!(whole.len(), dst as usize);
            for block in [1, 7, 255, 256, 257, 1000] {
                let split = resample(src, dst, &input, block);
                assert_eq!(bits(&split), bits(&whole), "{src}->{dst}, block {block}");
            }
            // And no click: a 440 Hz sine of amplitude 0.5 moves by at most
            // 0.5 * 2 pi * 440 / dst per output frame. The edges are skipped,
            // where the tone itself starts and stops.
            let bound = 0.5 * 2.0 * PI * 440.0 / f64::from(dst) * 1.01;
            let edge = dst as usize / 10;
            let max_step = whole[edge..whole.len() - edge]
                .windows(2)
                .map(|w| f64::from((w[1] - w[0]).abs()))
                .fold(0.0, f64::max);
            assert!(max_step < bound, "{src}->{dst}: step {max_step} > {bound}");
        }
    }

    #[test]
    fn silence_stays_exact_and_dc_passes_at_unity() {
        let silence = resample(44_100, 48_000, &vec![0.0; 44_100], 1024);
        assert_eq!(silence.len(), 48_000);
        assert!(
            silence.iter().all(|v| *v == 0.0),
            "silence must stay silent"
        );

        let dc = resample(44_100, 48_000, &vec![0.5; 44_100], 1024);
        assert!(
            settled(&dc, 48_000).iter().all(|v| (v - 0.5).abs() < 1e-4),
            "DC must pass at unity gain"
        );
    }

    #[test]
    fn stereo_resampling_keeps_channels_apart_and_averages_to_mono() {
        let left = sine(1_000.0, 0.5, 44_100);
        let input: Vec<f32> = left.iter().flat_map(|l| [*l, 0.0]).collect();
        let mut stereo = FormatConverter::new(AudioSpec::new(44_100, 2), AudioSpec::new(48_000, 2));
        let out = run(&mut stereo, &input, 1024);
        assert_eq!(out.len(), 2 * 48_000);
        assert!(out.iter().skip(1).step_by(2).all(|r| *r == 0.0));

        let mut mono = FormatConverter::new(AudioSpec::new(44_100, 2), AudioSpec::new(48_000, 1));
        let down = run(&mut mono, &input, 1024);
        assert_eq!(down.len(), 48_000);
        for (m, l) in down.iter().zip(out.iter().step_by(2)) {
            assert_eq!(m.to_bits(), ((l + 0.0) / 2.0).to_bits());
        }
    }

    #[test]
    fn reset_clears_the_history() {
        let src = AudioSpec::new(44_100, 2);
        let dst = AudioSpec::new(48_000, 2);
        let mut used = FormatConverter::new(src, dst);
        let mut scratch = vec![0.0; used.max_out_frames(4096) * 2];
        used.convert(&noise(4096 * 2), &mut scratch);
        used.reset();

        let probe: Vec<f32> = sine(440.0, 0.5, 44_100)
            .iter()
            .flat_map(|s| [*s, -*s])
            .collect();
        let mut fresh = FormatConverter::new(src, dst);
        let a = run(&mut used, &probe, 1024);
        let b = run(&mut fresh, &probe, 1024);
        assert_eq!(bits(&a), bits(&b), "no leak from before the reset");
    }

    #[test]
    fn a_ratio_without_a_small_fft_unit_uses_the_sinc_engine() {
        // 44100 -> 44101 reduces to 44100:44101, far above MAX_FFT_UNIT.
        let c = FormatConverter::new(AudioSpec::new(44_100, 1), AudioSpec::new(44_101, 1));
        assert_eq!(c.engine.as_ref().map(Engine::name), Some("sinc"));
        let common = FormatConverter::new(AudioSpec::new(44_100, 1), AudioSpec::new(48_000, 1));
        assert_eq!(common.engine.as_ref().map(Engine::name), Some("fft"));

        let out = resample(44_100, 44_101, &sine(1_000.0, 0.5, 44_100), 1000);
        assert_eq!(out.len(), 44_101);
        let y = &out[11_000..11_000 + 22_050];
        let (a, _) = fit(y, 11_000, 1_000.0, 44_101);
        assert!((20.0 * (a / 0.5).log10()).abs() < 0.05, "level {a}");
        let residual = residual_db(y, 11_000, 1_000.0, 44_101, 0.5);
        assert!(residual < -60.0, "residual {residual:.1} dB");
    }

    /// A6 finding 3 at converter level, with numbers. The tone
    /// `x[n] = S·sin(πn/2 + π/4)` at 44.1 kHz has samples of exactly
    /// `±S·√2/2`. At 48 kHz the fs/4 tone (11.025 kHz, inside the passband) is
    /// sampled near its true peak `S`, so the resampler rebuilds about
    /// `√2 ≈ +3 dB` over the sample peak.
    ///
    /// With `S = 1.0` the samples (`±0.7071`) pass any sample-peak limiter
    /// untouched, yet the output reaches `≈1.0`: a rebuild of over 40 % that
    /// no sample-peak criterion before the resampler can see. The bound is
    /// loose on purpose: the passband is flat to ±0.05 dB, so anything below
    /// `0.99` would be the filter, not the test.
    #[test]
    fn resampling_rebuilds_inter_sample_peaks_above_the_sample_peak() {
        let tone: Vec<f32> = (0..44_100)
            .map(|n| {
                (std::f64::consts::FRAC_PI_4 + std::f64::consts::PI * n as f64 / 2.0).sin() as f32
            })
            .collect();
        let pre = tone.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(
            (pre - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6,
            "sample peak {pre}"
        );
        let out = resample(44_100, 48_000, &tone, 1024);
        assert_eq!(out.len(), 48_000);
        let post = out.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(post > 0.99, "rebuilt peak {post} (sample peak {pre})");
        assert!(post / pre > 1.3, "rebuild ratio {}/{pre}", post);
        // ... but the final ceiling holds: nothing above full scale.
        assert!(post <= 1.0, "above 0 dBFS: {post}");
    }

    /// The A6 torture case end to end: samples at exactly the limiter ceiling
    /// (`±0.966`, so a sample-peak limiter passes them bit for bit) rebuild to
    /// about `+2.7 dBFS`, and the converter holds them at exactly full scale.
    #[test]
    fn torture_peaks_are_held_at_full_scale_after_resampling() {
        let ceiling = 10.0_f64.powf(-0.3 / 20.0) as f32;
        let tone: Vec<f32> = (0..44_100)
            .map(|n| {
                (1.3662
                    * (std::f64::consts::FRAC_PI_4 + std::f64::consts::PI * n as f64 / 2.0).sin())
                    as f32
            })
            .collect();
        let pre = tone.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(
            (pre - ceiling).abs() < 1e-3,
            "sample peak {pre}, ceiling {ceiling}"
        );
        let out = resample(44_100, 48_000, &tone, 1024);
        let post = out.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        // The rebuild happened (far above the ceiling) and was caught (at 1.0).
        assert!(post > ceiling, "no rebuild observed: {post}");
        assert!((post - 1.0).abs() < 1e-6, "post {post}, want exactly 1.0");
        let clamped = out.iter().filter(|s| s.abs() == 1.0).count();
        assert!(clamped > 0, "the safety net never engaged");
    }

    /// Subnormal inputs come out as silence on the resampled path (denormal
    /// hygiene for the render thread). The equal-rate path is untouched by
    /// `finish_sample` and keeps its bit-exact promise.
    #[test]
    fn resampled_subnormals_come_out_silent() {
        let input: Vec<f32> = noise(4096).iter().map(|s| s * 1e-40).collect();
        assert!(input.iter().all(|s| s.is_subnormal() || *s == 0.0));
        let out = resample(44_100, 48_000, &input, 1024);
        assert!(out.iter().all(|s| *s == 0.0), "subnormals must not pass");
    }

    /// Non-finite inputs come out as silence, not as spread NaNs: one NaN
    /// through a band-limited filter would otherwise contaminate a whole FFT
    /// block and its overlap.
    #[test]
    fn resampled_non_finite_comes_out_silent() {
        let mut input = noise(4096);
        input[0] = f32::NAN;
        input[1000] = f32::INFINITY;
        input[2000] = f32::NEG_INFINITY;
        let out = resample(44_100, 48_000, &input, 1024);
        assert!(
            out.iter().all(|s| s.is_finite()),
            "nothing non-finite leaves"
        );
        assert!(
            out.iter()
                .all(|s| *s == 0.0 || s.is_subnormal() || s.abs() <= 1.0),
            "bounded output"
        );
    }
}
