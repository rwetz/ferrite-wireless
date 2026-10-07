//! The radio's insides: one output stream that mixes the station with
//! static, and a decoder thread per tuned station feeding it.
//!
//! The output callback (cpal) pulls stereo frames from a short ring, mixes
//! in generated static by how far off-station the dial is, applies the
//! volume, and records peak levels for the VU meters. A station is an HTTP
//! stream (Icecast/SHOUTcast) decoded by symphonia; `Icy-MetaData` gives the
//! track title, stripped out of the audio by [`Icy`].

use std::collections::VecDeque;
use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How much decoded audio to keep ahead, in seconds.
const AHEAD: f32 = 1.5;
/// Static is quieter than music at the same volume.
const STATIC_GAIN: f32 = 0.22;

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Not tuned to anything: static only.
    Idle,
    Connecting,
    Playing,
    Failed(String),
}

/// An f32 in an atomic.
#[derive(Default)]
struct Level(AtomicU32);

impl Level {
    fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
    fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed);
    }
}

/// What the UI, the output callback and the decoder share.
pub struct Radio {
    ring: Mutex<VecDeque<[f32; 2]>>,
    /// The output device's frame rate (0 until it opens).
    rate: AtomicU32,
    on: AtomicBool,
    volume: Level,
    hiss: Level,
    /// Peak levels since the UI last read them.
    peak: [Level; 2],
    /// Bumped to cancel the running decoder.
    generation: AtomicU64,
    status: Mutex<Status>,
    title: Mutex<Option<String>>,
    /// Why there's no sound at all, when the device didn't open.
    output_error: Mutex<Option<String>>,
}

impl Radio {
    /// Open the default output device on its own thread and return the
    /// shared state. Never fails: without a device the radio is silent and
    /// says why in `output_error`.
    pub fn start(volume: f32) -> Arc<Radio> {
        let radio = Arc::new(Radio {
            ring: Mutex::new(VecDeque::new()),
            rate: AtomicU32::new(0),
            on: AtomicBool::new(true),
            volume: Level::default(),
            hiss: Level::default(),
            peak: Default::default(),
            generation: AtomicU64::new(0),
            status: Mutex::new(Status::Idle),
            title: Mutex::new(None),
            output_error: Mutex::new(None),
        });
        radio.volume.set(volume);
        radio.hiss.set(1.);
        let r = radio.clone();
        std::thread::Builder::new()
            .name("wireless-output".into())
            .spawn(move || match output::open(r.clone()) {
                // The stream plays for as long as it's alive: keep it here.
                Ok(_stream) => loop {
                    std::thread::park();
                },
                Err(why) => *r.output_error.lock().unwrap() = Some(why),
            })
            .expect("spawning the audio thread");
        radio
    }

    pub fn set_on(&self, on: bool) {
        self.on.store(on, Ordering::Relaxed);
    }

    pub fn set_volume(&self, v: f32) {
        self.volume.set(v.clamp(0., 1.));
    }

    /// How much static to mix in, 0–1.
    pub fn set_hiss(&self, h: f32) {
        self.hiss.set(h.clamp(0., 1.));
    }

    /// Peak levels (left, right) since the last call, 0–1.
    pub fn take_peaks(&self) -> [f32; 2] {
        [self.peak[0].0.swap(0, Ordering::Relaxed), self.peak[1].0.swap(0, Ordering::Relaxed)].map(f32::from_bits)
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    pub fn title(&self) -> Option<String> {
        self.title.lock().unwrap().clone()
    }

    pub fn output_error(&self) -> Option<String> {
        self.output_error.lock().unwrap().clone()
    }

    /// Stop the station (static carries on).
    pub fn stop(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.ring.lock().unwrap().clear();
        *self.title.lock().unwrap() = None;
        *self.status.lock().unwrap() = Status::Idle;
    }

    /// Tune to a stream, replacing whatever was playing.
    pub fn play(self: &Arc<Self>, url: String) {
        self.stop();
        let generation = self.generation.load(Ordering::SeqCst);
        *self.status.lock().unwrap() = Status::Connecting;
        let radio = self.clone();
        std::thread::Builder::new()
            .name("wireless-decoder".into())
            .spawn(move || {
                let result = decode::run(&radio, &url, generation);
                if radio.generation.load(Ordering::SeqCst) == generation {
                    *radio.status.lock().unwrap() = match result {
                        Ok(()) => Status::Failed("the station ended the stream".into()),
                        Err(why) => Status::Failed(why),
                    };
                }
            })
            .expect("spawning the decoder thread");
    }

    fn current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == generation
    }
}

// ── Static ──────────────────────────────────────────────────────────────────

/// Radio static: white noise, softened by a one-pole low-pass, with the odd
/// crackle. Deterministic for a seed.
pub struct Static {
    state: u32,
    low: [f32; 2],
}

impl Static {
    pub fn new(seed: u32) -> Self {
        Static { state: seed.max(1), low: [0.; 2] }
    }

    fn rand(&mut self) -> f32 {
        // xorshift32
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        (x as f32 / u32::MAX as f32) * 2. - 1.
    }

    pub fn frame(&mut self) -> [f32; 2] {
        let crackle = if self.rand() > 0.9985 { self.rand() * 2.5 } else { 0. };
        let mut out = [0.; 2];
        for (ch, o) in out.iter_mut().enumerate() {
            let white = self.rand();
            self.low[ch] += 0.45 * (white - self.low[ch]);
            *o = (self.low[ch] * 0.8 + white * 0.2 + crackle).clamp(-1., 1.);
        }
        out
    }
}

// ── Resampling ──────────────────────────────────────────────────────────────

/// Linear resampling of stereo frames from one rate to another, carrying
/// its position across calls so packet boundaries don't click.
pub struct Resampler {
    from: u32,
    to: u32,
    /// Position in source frames, relative to `last`.
    pos: f64,
    last: [f32; 2],
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        Resampler { from: from.max(1), to: to.max(1), pos: 0., last: [0.; 2] }
    }

    /// Resample `input` and push the frames to `out`.
    pub fn process(&mut self, input: &[[f32; 2]], out: &mut impl Extend<[f32; 2]>) {
        if self.from == self.to {
            out.extend(input.iter().copied());
            return;
        }
        let step = self.from as f64 / self.to as f64;
        // Source frame i is `last` at i = 0, then input[0], input[1], ...
        let at = |i: usize| if i == 0 { self.last } else { input[i - 1] };
        let mut frames = Vec::with_capacity((input.len() as f64 / step) as usize + 1);
        while self.pos + 1. <= input.len() as f64 {
            let i = self.pos.floor() as usize;
            let t = (self.pos - i as f64) as f32;
            let (a, b) = (at(i), at(i + 1));
            frames.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
            self.pos += step;
        }
        self.pos -= input.len() as f64;
        if let Some(l) = input.last() {
            self.last = *l;
        }
        out.extend(frames);
    }
}

/// Interleaved samples of any channel count → stereo frames.
pub fn to_stereo(samples: &[f32], channels: usize) -> Vec<[f32; 2]> {
    match channels {
        0 => Vec::new(),
        1 => samples.iter().map(|&s| [s, s]).collect(),
        n => samples.chunks_exact(n).map(|c| [c[0], c[1]]).collect(),
    }
}

// ── ICY metadata ────────────────────────────────────────────────────────────

/// The title out of an ICY metadata block: `StreamTitle='Artist - Song';`.
pub fn stream_title(meta: &str) -> Option<String> {
    let start = meta.find("StreamTitle='")? + "StreamTitle='".len();
    let rest = &meta[start..];
    let end = rest.find("';").unwrap_or(rest.len());
    let title = rest[..end].trim();
    (!title.is_empty()).then(|| title.to_string())
}

/// Strips SHOUTcast/Icecast metadata blocks out of a stream, every
/// `metaint` bytes, and reports titles as they arrive.
pub struct Icy<R> {
    inner: R,
    metaint: usize,
    until_meta: usize,
    on_title: Box<dyn FnMut(String) + Send>,
}

impl<R: Read> Icy<R> {
    pub fn new(inner: R, metaint: usize, on_title: impl FnMut(String) + Send + 'static) -> Self {
        Icy { inner, metaint, until_meta: metaint, on_title: Box::new(on_title) }
    }
}

impl<R: Read> Read for Icy<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.metaint == 0 {
            return self.inner.read(buf);
        }
        if self.until_meta == 0 {
            let mut len = [0u8; 1];
            self.inner.read_exact(&mut len)?;
            let mut meta = vec![0u8; len[0] as usize * 16];
            self.inner.read_exact(&mut meta)?;
            if let Some(t) = stream_title(&String::from_utf8_lossy(&meta)) {
                (self.on_title)(t);
            }
            self.until_meta = self.metaint;
        }
        let n = buf.len().min(self.until_meta);
        let read = self.inner.read(&mut buf[..n])?;
        self.until_meta -= read;
        Ok(read)
    }
}

/// symphonia wants `Send + Sync`; an HTTP body is only `Send`.
struct SyncRead(Mutex<Box<dyn Read + Send>>);

impl Read for SyncRead {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.get_mut().unwrap().read(buf)
    }
}

// ── Decoding ────────────────────────────────────────────────────────────────

mod decode {
    use super::*;
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::errors::Error;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::{MediaSourceStream, ReadOnlySource};
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    /// Connect, decode and feed the ring until the stream ends, fails, or
    /// another station is tuned (`generation` moves on).
    pub fn run(radio: &Arc<Radio>, url: &str, generation: u64) -> Result<(), String> {
        let resp = ureq::get(url).header("Icy-MetaData", "1").call().map_err(|e| match e {
            ureq::Error::StatusCode(code) => format!("the station returned HTTP {code}"),
            _ => "couldn't reach the station".to_string(),
        })?;
        let header = |name: &str| resp.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
        let metaint = header("icy-metaint").and_then(|v| v.parse().ok()).unwrap_or(0);
        let mime = header("content-type").unwrap_or_default();
        let titles = radio.clone();
        let body = Icy::new(resp.into_body().into_reader(), metaint, move |t| {
            if titles.current(generation) {
                *titles.title.lock().unwrap() = Some(t);
            }
        });

        let mut hint = Hint::new();
        match mime.split(';').next().unwrap_or("").trim() {
            "audio/mpeg" | "audio/mp3" => hint.with_extension("mp3"),
            "audio/aac" | "audio/aacp" => hint.with_extension("aac"),
            "application/ogg" | "audio/ogg" => hint.with_extension("ogg"),
            _ => &mut hint,
        };
        let source = ReadOnlySource::new(SyncRead(Mutex::new(Box::new(body))));
        let stream = MediaSourceStream::new(Box::new(source), Default::default());
        let probed = symphonia::default::get_probe()
            .format(&hint, stream, &FormatOptions::default(), &MetadataOptions::default())
            .map_err(|_| format!("can't play this stream ({})", if mime.is_empty() { "unknown format" } else { &mime }))?;
        let mut format = probed.format;
        let track = format.default_track().ok_or("the stream has no audio")?;
        let track_id = track.id;
        let mut decoder =
            symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).map_err(|_| "can't decode this stream's codec".to_string())?;

        let mut resampler: Option<Resampler> = None;
        let mut started = false;
        loop {
            if !radio.current(generation) {
                return Ok(());
            }
            let packet = match format.next_packet() {
                Ok(p) => p,
                Err(Error::IoError(_)) => return Err("the stream dropped".into()),
                Err(_) => return Err("the stream broke".into()),
            };
            if packet.track_id() != track_id {
                continue;
            }
            let decoded = match decoder.decode(&packet) {
                Ok(d) => d,
                // A bad frame in a live stream: skip it.
                Err(Error::DecodeError(_)) => continue,
                Err(_) => return Err("the decoder failed".into()),
            };
            let spec = *decoded.spec();
            let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
            buf.copy_interleaved_ref(decoded);
            let frames = to_stereo(buf.samples(), spec.channels.count());

            // Wait for the output device's rate (it opens on its own thread).
            let mut to = radio.rate.load(Ordering::Relaxed);
            while to == 0 {
                if radio.output_error().is_some() || !radio.current(generation) {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(50));
                to = radio.rate.load(Ordering::Relaxed);
            }
            let rs = resampler.get_or_insert_with(|| Resampler::new(spec.rate, to));
            // Don't run more than AHEAD seconds in front of the speakers.
            while radio.ring.lock().unwrap().len() > (to as f32 * AHEAD) as usize {
                if !radio.current(generation) {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            let mut ring = radio.ring.lock().unwrap();
            if !radio.current(generation) {
                return Ok(());
            }
            rs.process(&frames, &mut *ring);
            drop(ring);
            if !started {
                started = true;
                *radio.status.lock().unwrap() = Status::Playing;
            }
        }
    }
}

// ── Output ──────────────────────────────────────────────────────────────────

mod output {
    use super::*;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{FromSample, SampleFormat, SizedSample};

    pub fn open(radio: Arc<Radio>) -> Result<cpal::Stream, String> {
        let device = cpal::default_host().default_output_device().ok_or("no audio output device")?;
        let supported = device.default_output_config().map_err(|e| format!("the audio device won't say its format: {e}"))?;
        let format = supported.sample_format();
        let config = supported.config();
        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, &config, radio.clone()),
            SampleFormat::I16 => build::<i16>(&device, &config, radio.clone()),
            SampleFormat::U16 => build::<u16>(&device, &config, radio.clone()),
            SampleFormat::I32 => build::<i32>(&device, &config, radio.clone()),
            other => return Err(format!("unsupported audio format {other}")),
        }?;
        stream.play().map_err(|e| format!("the audio device won't play: {e}"))?;
        radio.rate.store(config.sample_rate, Ordering::Relaxed);
        Ok(stream)
    }

    fn build<T: SizedSample + FromSample<f32>>(device: &cpal::Device, config: &cpal::StreamConfig, radio: Arc<Radio>) -> Result<cpal::Stream, String> {
        let channels = config.channels as usize;
        let mut hiss = Static::new(0x5eed);
        device
            .build_output_stream(
                config,
                move |data: &mut [T], _| {
                    let on = radio.on.load(Ordering::Relaxed);
                    let (volume, h) = (radio.volume.get(), radio.hiss.get());
                    let mut peak = [0f32; 2];
                    let mut ring = radio.ring.lock().unwrap();
                    for frame in data.chunks_mut(channels) {
                        let music = if on { ring.pop_front().unwrap_or([0.; 2]) } else { [0.; 2] };
                        let noise = hiss.frame();
                        let mut out = [0f32; 2];
                        for ch in 0..2 {
                            let s = if on { music[ch] * (1. - h) + noise[ch] * h * STATIC_GAIN } else { 0. };
                            out[ch] = (s * volume).clamp(-1., 1.);
                            peak[ch] = peak[ch].max(out[ch].abs());
                        }
                        for (i, sample) in frame.iter_mut().enumerate() {
                            *sample = T::from_sample(out[i.min(1)]);
                        }
                    }
                    drop(ring);
                    for (p, held) in peak.iter().zip(&radio.peak) {
                        if *p > held.get() {
                            held.set(*p);
                        }
                    }
                },
                |err| eprintln!("wireless: audio stream error: {err}"),
                None,
            )
            .map_err(|e| format!("couldn't open the audio device: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_from_icy_metadata() {
        assert_eq!(stream_title("StreamTitle='Boards of Canada - Dayvan Cowboy';StreamUrl='';").as_deref(), Some("Boards of Canada - Dayvan Cowboy"));
        assert_eq!(stream_title("StreamTitle='';"), None);
        assert_eq!(stream_title("nothing here"), None);
    }

    #[test]
    fn icy_strips_metadata_blocks() {
        // 4 bytes of audio, a 16-byte metadata block, 4 more bytes, an empty block, 2 bytes.
        let meta = b"StreamTitle='A';";
        assert_eq!(meta.len(), 16);
        let mut stream = b"abcd".to_vec();
        stream.push(1);
        stream.extend_from_slice(meta);
        stream.extend_from_slice(b"efgh");
        stream.push(0);
        stream.extend_from_slice(b"ij");
        let titles = Arc::new(Mutex::new(Vec::new()));
        let t = titles.clone();
        let mut icy = Icy::new(&stream[..], 4, move |s| t.lock().unwrap().push(s));
        let mut audio = Vec::new();
        icy.read_to_end(&mut audio).unwrap();
        assert_eq!(audio, b"abcdefghij");
        assert_eq!(*titles.lock().unwrap(), vec!["A".to_string()]);
    }

    #[test]
    fn without_metaint_icy_passes_through() {
        let mut out = Vec::new();
        Icy::new(&b"plain"[..], 0, |_| {}).read_to_end(&mut out).unwrap();
        assert_eq!(out, b"plain");
    }

    #[test]
    fn static_is_bounded_and_not_silent() {
        let mut s = Static::new(7);
        let frames: Vec<[f32; 2]> = (0..48_000).map(|_| s.frame()).collect();
        assert!(frames.iter().flatten().all(|v| (-1.0..=1.0).contains(v)));
        let rms = (frames.iter().flatten().map(|v| v * v).sum::<f32>() / 96_000.).sqrt();
        assert!(rms > 0.1, "{rms}");
        // Left and right are different noise.
        assert!(frames.iter().any(|[l, r]| l != r));
    }

    #[test]
    fn resampling_keeps_duration() {
        let input: Vec<[f32; 2]> = (0..44_100).map(|i| [(i as f32 / 100.).sin(); 2]).collect();
        let mut out: Vec<[f32; 2]> = Vec::new();
        let mut rs = Resampler::new(44_100, 48_000);
        // Fed in uneven packets, like a decoder does.
        for chunk in input.chunks(1152) {
            rs.process(chunk, &mut out);
        }
        assert!((out.len() as i64 - 48_000).abs() <= 2, "{}", out.len());
        let mut same = Vec::new();
        Resampler::new(48_000, 48_000).process(&input[..10], &mut same);
        assert_eq!(same, input[..10]);
    }

    #[test]
    fn stereo_from_any_layout() {
        assert_eq!(to_stereo(&[0.5, -0.5], 1), vec![[0.5, 0.5], [-0.5, -0.5]]);
        assert_eq!(to_stereo(&[1., 2., 3., 4., 5., 6.], 3), vec![[1., 2.], [4., 5.]]);
        assert!(to_stereo(&[1.], 0).is_empty());
    }
}
