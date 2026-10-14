//! Test doubles for the audio traits (feature `mock`).
//!
//! Lanes use these before real implementations exist:
//! [`MockDecoder`] (sine / silence of a given length), [`MockDecoderFactory`],
//! [`NullOutput`] (discards audio, advances a clock when pumped) and
//! [`MockDsp`] (passthrough that counts frames).

use std::f32::consts::TAU;
use std::sync::Arc;
use std::time::Duration;

use crate::audio::{
    AudioSpec, Confidence, Decoder, DecoderFactory, DspStage, MediaSource, OutputDevice,
    OutputFormat, ProbeHint, RenderFn,
};
use crate::error::{Error, Result};
use crate::meta::TrackMetadata;

/// Generated signal kind.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MockSignal {
    /// Sine wave at the given frequency (Hz), amplitude 0.5.
    Sine(f32),
    /// Digital silence.
    Silence,
}

/// Decoder generating a deterministic signal of fixed length.
#[derive(Debug, Clone)]
pub struct MockDecoder {
    spec: AudioSpec,
    signal: MockSignal,
    total_frames: u64,
    pos: u64,
}

impl MockDecoder {
    /// Sine decoder of `total_frames` frames.
    pub fn sine(spec: AudioSpec, freq_hz: f32, total_frames: u64) -> Self {
        MockDecoder {
            spec,
            signal: MockSignal::Sine(freq_hz),
            total_frames,
            pos: 0,
        }
    }

    /// Silent decoder of `total_frames` frames.
    pub fn silence(spec: AudioSpec, total_frames: u64) -> Self {
        MockDecoder {
            spec,
            signal: MockSignal::Silence,
            total_frames,
            pos: 0,
        }
    }

    /// Frames already produced.
    pub fn position_frames(&self) -> u64 {
        self.pos
    }
}

impl Decoder for MockDecoder {
    fn spec(&self) -> AudioSpec {
        self.spec
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        let ch = usize::from(self.spec.channels);
        let want = (out.len() / ch) as u64;
        let n = want.min(self.total_frames - self.pos) as usize;
        for i in 0..n {
            let v = match self.signal {
                MockSignal::Silence => 0.0,
                MockSignal::Sine(f) => {
                    let t = (self.pos + i as u64) as f64 / f64::from(self.spec.sample_rate);
                    0.5 * ((f64::from(f) * f64::from(TAU) * t).sin() as f32)
                }
            };
            out[i * ch..(i + 1) * ch].fill(v);
        }
        self.pos += n as u64;
        Ok(n)
    }

    fn seek(&mut self, to: Duration) -> Result<Duration> {
        let frames = (to.as_secs_f64() * f64::from(self.spec.sample_rate)) as u64;
        self.pos = frames.min(self.total_frames);
        Ok(self.spec.duration_of(self.pos))
    }

    fn duration(&self) -> Option<Duration> {
        Some(self.spec.duration_of(self.total_frames))
    }

    fn metadata(&self) -> TrackMetadata {
        TrackMetadata {
            title: Some("Mock Track".into()),
            artist: Some("Mock".into()),
            duration_ms: Some(self.spec.duration_of(self.total_frames).as_millis() as u64),
            ..TrackMetadata::default()
        }
    }
}

/// Factory claiming the `mock` extension or a `MOCK` header; produces one second of 440 Hz stereo sine at 44.1 kHz.
#[derive(Debug, Default, Clone, Copy)]
pub struct MockDecoderFactory;

impl MockDecoderFactory {
    /// Convenience: an `Arc` ready for `DecoderRegistry::register`.
    pub fn arc() -> Arc<dyn DecoderFactory> {
        Arc::new(MockDecoderFactory)
    }
}

impl DecoderFactory for MockDecoderFactory {
    fn name(&self) -> &str {
        "mock"
    }
    fn extensions(&self) -> &[&str] {
        &["mock"]
    }
    fn probe(&self, hint: &ProbeHint<'_>) -> Confidence {
        if hint.header.starts_with(b"MOCK") {
            Confidence::CONTENT
        } else if hint.extension == Some("mock") {
            Confidence::EXTENSION
        } else {
            Confidence::NONE
        }
    }
    fn open(&self, _source: MediaSource) -> Result<Box<dyn Decoder>> {
        let spec = AudioSpec::new(44100, 2);
        Ok(Box::new(MockDecoder::sine(spec, 440.0, 44100)))
    }
}

/// Output that discards audio. Nothing runs on its own: call [`NullOutput::pump`]
/// to simulate the device pulling frames and advancing the audio clock.
pub struct NullOutput {
    spec: Option<AudioSpec>,
    render: Option<RenderFn>,
    paused: bool,
    played: u64,
}

impl Default for NullOutput {
    fn default() -> Self {
        Self::new()
    }
}

impl NullOutput {
    /// New closed output.
    pub fn new() -> Self {
        NullOutput {
            spec: None,
            render: None,
            paused: false,
            played: 0,
        }
    }

    /// Simulate the device pulling `frames` frames (in chunks of up to 1024).
    /// Returns frames consumed; 0 if not started or paused.
    pub fn pump(&mut self, frames: u64) -> u64 {
        let (Some(spec), Some(render)) = (self.spec, self.render.as_mut()) else {
            return 0;
        };
        if self.paused {
            return 0;
        }
        let mut buf = vec![0.0f32; spec.samples_for(1024)];
        let mut left = frames;
        while left > 0 {
            let n = left.min(1024) as usize;
            render(&mut buf[..spec.samples_for(n)]);
            left -= n as u64;
        }
        self.played += frames;
        frames
    }
}

impl OutputDevice for NullOutput {
    fn name(&self) -> &str {
        "null"
    }
    fn open(&mut self, spec: AudioSpec) -> Result<OutputFormat> {
        self.spec = Some(spec);
        Ok(OutputFormat {
            spec,
            buffer_frames: 4096,
        })
    }
    fn start(&mut self, render: RenderFn) -> Result<()> {
        if self.spec.is_none() {
            return Err(Error::Output("not open".into()));
        }
        self.render = Some(render);
        self.paused = false;
        Ok(())
    }
    fn pause(&mut self) -> Result<()> {
        self.paused = true;
        Ok(())
    }
    fn resume(&mut self) -> Result<()> {
        self.paused = false;
        Ok(())
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
    fn latency(&self) -> Duration {
        Duration::ZERO
    }
    fn played_frames(&self) -> u64 {
        self.played
    }
    fn close(&mut self) -> Result<()> {
        self.render = None;
        self.spec = None;
        Ok(())
    }
}

/// Passthrough DSP stage that counts processed frames.
#[derive(Debug, Default)]
pub struct MockDsp {
    channels: u16,
    /// Total frames seen by `process`.
    pub frames_processed: u64,
    /// Number of `reset` calls.
    pub resets: u32,
}

impl DspStage for MockDsp {
    fn name(&self) -> &str {
        "mock-passthrough"
    }
    fn prepare(&mut self, spec: AudioSpec) -> Result<()> {
        self.channels = spec.channels;
        Ok(())
    }
    fn process(&mut self, buf: &mut [f32]) {
        self.frames_processed += (buf.len() / usize::from(self.channels.max(1))) as u64;
    }
    fn reset(&mut self) {
        self.resets += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::DecoderRegistry;

    #[test]
    fn decoder_produces_expected_frames() {
        let spec = AudioSpec::new(8000, 2);
        let mut d = MockDecoder::sine(spec, 1000.0, 2500);
        assert_eq!(d.duration(), Some(Duration::from_secs_f64(2500.0 / 8000.0)));
        let mut buf = vec![0.0; spec.samples_for(1000)];
        let mut total = 0;
        loop {
            let n = d.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            total += n;
        }
        assert_eq!(total, 2500);
        // seek back and re-read deterministically
        d.seek(Duration::from_secs_f64(0.125)).unwrap();
        assert_eq!(d.position_frames(), 1000);
        assert_eq!(d.read(&mut buf).unwrap(), 1000);
        // sine at 1 kHz / 8 kHz: frame 2 is at phase pi/2*... just check channels are equal
        assert_eq!(buf[0], buf[1]);
    }

    #[test]
    fn silence_is_zero() {
        let spec = AudioSpec::new(44100, 1);
        let mut d = MockDecoder::silence(spec, 10);
        let mut buf = [1.0f32; 16];
        assert_eq!(d.read(&mut buf).unwrap(), 10);
        assert!(buf[..10].iter().all(|v| *v == 0.0));
    }

    #[test]
    fn registry_and_null_output() {
        let mut reg = DecoderRegistry::new();
        reg.register(MockDecoderFactory::arc());
        assert!(reg.open(MediaSource::Path("x.wav".into())).is_err());
        let mut dec = reg
            .open(MediaSource::Reader {
                reader: Box::new(std::io::Cursor::new(b"hello".to_vec())),
                extension: Some("MOCK".into()),
            })
            .unwrap();

        let mut dsp = MockDsp::default();
        dsp.prepare(dec.spec()).unwrap();

        let mut out = NullOutput::new();
        let fmt = out.open(dec.spec()).unwrap();
        let spec = fmt.spec;
        out.start(Box::new(move |buf: &mut [f32]| {
            // would pull from ring; here we just fill silence
            buf.fill(0.0);
            let _ = spec;
        }))
        .unwrap();
        assert_eq!(out.pump(2048), 2048);
        out.pause().unwrap();
        assert_eq!(out.pump(100), 0);
        assert_eq!(out.played_frames(), 2048);

        let mut buf = vec![0.0; spec.samples_for(512)];
        let n = dec.read(&mut buf).unwrap();
        dsp.process(&mut buf[..spec.samples_for(n)]);
        assert_eq!(dsp.frames_processed, 512);
    }
}
