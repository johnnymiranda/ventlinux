//! Capture and playback via cpal (Pulse/PipeWire on this machine).
//!
//! Incoming voice is mixed in software. libventrilo3 already resampled encode
//! on the way out; we linearly resample decode to the output device rate.

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

type SamplePush = Arc<dyn Fn(&[i16]) + Send + Sync>;

#[derive(Clone, Debug)]
pub struct AudioDevice {
    pub name: String,
    pub is_input: bool,
    pub is_output: bool,
}

pub fn list_devices() -> (Vec<AudioDevice>, Vec<AudioDevice>) {
    let host = cpal::default_host();
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    if let Ok(devs) = host.input_devices() {
        for d in devs {
            if let Ok(name) = d.name() {
                inputs.push(AudioDevice {
                    name,
                    is_input: true,
                    is_output: false,
                });
            }
        }
    }
    if let Ok(devs) = host.output_devices() {
        for d in devs {
            if let Ok(name) = d.name() {
                outputs.push(AudioDevice {
                    name,
                    is_input: false,
                    is_output: true,
                });
            }
        }
    }
    (inputs, outputs)
}

/// Resolved devices, keyed by `(want_input, configured name)`.
///
/// Resolving walks ALSA's PCM list and probes each entry, which costs ~500ms
/// once a specific card is configured — far too slow to repeat on every
/// push-to-talk press, where it would clip the start of every transmission.
static DEVICE_CACHE: OnceLock<Mutex<HashMap<(bool, String), Device>>> = OnceLock::new();

fn device_cache() -> &'static Mutex<HashMap<(bool, String), Device>> {
    DEVICE_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cache_key(want_input: bool, name: Option<&str>) -> (bool, String) {
    (want_input, name.unwrap_or_default().to_string())
}

/// `find_device`, memoised. The empty name resolves to the Pulse/PipeWire
/// virtual PCM, which stays correct when the session's default device changes,
/// so the entry does not go stale.
fn resolve_device(want_input: bool, name: Option<&str>) -> Result<Device> {
    let key = cache_key(want_input, name);
    if let Some(device) = device_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(&key).cloned())
    {
        return Ok(device);
    }
    let device = find_device(want_input, name)?;
    if let Ok(mut cache) = device_cache().lock() {
        cache.insert(key, device.clone());
    }
    Ok(device)
}

/// Resolve the capture device ahead of time so the first push-to-talk press
/// does not pay for it. Blocks, so call it off the UI thread; failures are
/// ignored because the real attempt will report them.
pub fn prewarm_input(name: Option<&str>) {
    let _ = resolve_device(true, name);
}

/// Forget a cached device. Returns whether there was one, so callers can skip a
/// pointless retry when the failure did not come from a stale entry.
fn forget_device(want_input: bool, name: Option<&str>) -> bool {
    let key = cache_key(want_input, name);
    device_cache()
        .lock()
        .map(|mut cache| cache.remove(&key).is_some())
        .unwrap_or(false)
}

fn find_device(want_input: bool, name: Option<&str>) -> Result<Device> {
    let host = cpal::default_host();
    if let Some(name) = name.filter(|s| !s.is_empty()) {
        let iter = if want_input {
            host.input_devices()
                .map_err(|e| anyhow!("input devices: {e}"))?
        } else {
            host.output_devices()
                .map_err(|e| anyhow!("output devices: {e}"))?
        };
        for d in iter {
            if d.name().ok().as_deref() == Some(name) {
                return Ok(d);
            }
        }
    }
    // Prefer the Pulse/PipeWire virtual PCMs so we don't bypass the session
    // and land on a raw HDMI ALSA device.
    let iter = if want_input {
        host.input_devices()
            .map_err(|e| anyhow!("input devices: {e}"))?
    } else {
        host.output_devices()
            .map_err(|e| anyhow!("output devices: {e}"))?
    };
    let mut fallback = None;
    for d in iter {
        if let Ok(n) = d.name() {
            if n == "pulse" || n == "pipewire" {
                return Ok(d);
            }
            if n == "default" && fallback.is_none() {
                fallback = Some(d);
            }
        }
    }
    if let Some(d) = fallback {
        return Ok(d);
    }
    if want_input {
        host.default_input_device()
            .ok_or_else(|| anyhow!("no default input device"))
    } else {
        host.default_output_device()
            .ok_or_else(|| anyhow!("no default output device"))
    }
}

struct Voice {
    rate: u32,
    channels: u8,
    samples: VecDeque<i16>,
    phase: f64,
}

struct MixerInner {
    voices: HashMap<u16, Voice>,
    muted_users: HashSet<u16>,
    master_mute: bool,
    out_rate: u32,
    out_channels: u16,
}

/// Playback mixer handle — `Send + Sync`, safe to call from the protocol thread.
#[derive(Clone)]
pub struct PlayerHandle {
    inner: Arc<Mutex<MixerInner>>,
}

pub struct Player {
    inner: Arc<Mutex<MixerInner>>,
    _stream: Stream,
}

impl Player {
    pub fn start(output_name: Option<&str>) -> Result<Self> {
        let err = match Self::open(resolve_device(false, output_name)?) {
            Ok(player) => return Ok(player),
            Err(e) => e,
        };
        // A cached device can go stale if it was unplugged. Re-resolve once;
        // if nothing was cached the retry would just repeat the same failure.
        if !forget_device(false, output_name) {
            return Err(err);
        }
        Self::open(find_device(false, output_name)?)
    }

    fn open(device: Device) -> Result<Self> {
        let supported = device.default_output_config().context("output config")?;
        let sample_format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let out_rate = config.sample_rate.0;
        let out_channels = config.channels;
        let inner = Arc::new(Mutex::new(MixerInner {
            voices: HashMap::new(),
            muted_users: HashSet::new(),
            master_mute: false,
            out_rate,
            out_channels,
        }));
        let mix = inner.clone();
        let err_fn = |e| eprintln!("vent-audio output: {e}");
        let stream = match sample_format {
            SampleFormat::F32 => device.build_output_stream(
                &config,
                move |data: &mut [f32], _| mix.lock().unwrap().fill_f32(data),
                err_fn,
                None,
            )?,
            SampleFormat::I16 => device.build_output_stream(
                &config,
                move |data: &mut [i16], _| mix.lock().unwrap().fill_i16(data),
                err_fn,
                None,
            )?,
            SampleFormat::U16 => device.build_output_stream(
                &config,
                move |data: &mut [u16], _| mix.lock().unwrap().fill_u16(data),
                err_fn,
                None,
            )?,
            other => return Err(anyhow!("unsupported output format {other:?}")),
        };
        stream.play()?;
        Ok(Self {
            inner,
            _stream: stream,
        })
    }

    pub fn handle(&self) -> PlayerHandle {
        PlayerHandle {
            inner: self.inner.clone(),
        }
    }

    pub fn play(&self, user_id: u16, rate: u32, channels: u8, pcm: &[u8]) {
        self.handle().play(user_id, rate, channels, pcm);
    }

    pub fn set_muted(&self, muted: bool) {
        self.handle().set_muted(muted);
    }
    pub fn set_user_muted(&self, user_id: u16, muted: bool) {
        self.handle().set_user_muted(user_id, muted);
    }
    pub fn clear_user_mutes(&self) {
        self.handle().clear_user_mutes();
    }
    pub fn shutdown(&self) {
        self.inner.lock().unwrap().voices.clear();
    }
}

impl PlayerHandle {
    pub fn play(&self, user_id: u16, rate: u32, channels: u8, pcm: &[u8]) {
        if pcm.is_empty() || rate == 0 {
            return;
        }
        let mut g = self.inner.lock().unwrap();
        if g.master_mute || g.muted_users.contains(&user_id) {
            return;
        }
        let ch = channels.max(1);
        let (sample_bytes, _) = pcm.as_chunks::<2>();
        let samples: Vec<i16> = sample_bytes
            .iter()
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        let voice = g.voices.entry(user_id).or_insert_with(|| Voice {
            rate,
            channels: ch,
            samples: VecDeque::new(),
            phase: 0.0,
        });
        if voice.rate != rate || voice.channels != ch {
            voice.rate = rate;
            voice.channels = ch;
            voice.samples.clear();
            voice.phase = 0.0;
        }
        // Cap ~1s of PCM so a stalled consumer can't grow forever.
        let cap = rate as usize * ch as usize;
        for s in samples {
            if voice.samples.len() >= cap {
                voice.samples.pop_front();
            }
            voice.samples.push_back(s);
        }
    }

    pub fn set_muted(&self, muted: bool) {
        let mut g = self.inner.lock().unwrap();
        g.master_mute = muted;
        if muted {
            g.voices.clear();
        }
    }
    pub fn set_user_muted(&self, user_id: u16, muted: bool) {
        let mut g = self.inner.lock().unwrap();
        if muted {
            g.muted_users.insert(user_id);
            g.voices.remove(&user_id);
        } else {
            g.muted_users.remove(&user_id);
        }
    }
    pub fn clear_user_mutes(&self) {
        self.inner.lock().unwrap().muted_users.clear();
    }
}

impl MixerInner {
    fn next_sample(&mut self) -> f32 {
        if self.master_mute {
            return 0.0;
        }
        let out_rate = self.out_rate as f64;
        let mut mix = 0.0f32;
        let mut dead = Vec::new();
        for (id, voice) in self.voices.iter_mut() {
            let ch = voice.channels as usize;
            if voice.samples.len() < ch {
                if voice.samples.is_empty() {
                    dead.push(*id);
                }
                continue;
            }
            let step = voice.rate as f64 / out_rate;
            let idx = voice.phase.floor() as usize * ch;
            if idx + ch > voice.samples.len() {
                dead.push(*id);
                continue;
            }
            let mut s = 0.0f32;
            for c in 0..ch {
                s += voice.samples[idx + c] as f32 / 32768.0;
            }
            s /= ch as f32;
            mix += s;
            voice.phase += step;
            let drop = voice.phase.floor() as usize;
            if drop > 0 {
                let n = drop * ch;
                for _ in 0..n.min(voice.samples.len()) {
                    voice.samples.pop_front();
                }
                voice.phase -= drop as f64;
            }
        }
        for id in dead {
            self.voices.remove(&id);
        }
        mix.clamp(-1.0, 1.0)
    }

    fn fill_f32(&mut self, data: &mut [f32]) {
        let ch = self.out_channels as usize;
        for frame in data.chunks_mut(ch) {
            let s = self.next_sample();
            for c in frame.iter_mut() {
                *c = s;
            }
        }
    }
    fn fill_i16(&mut self, data: &mut [i16]) {
        let ch = self.out_channels as usize;
        for frame in data.chunks_mut(ch) {
            let s = (self.next_sample() * 32767.0) as i16;
            for c in frame.iter_mut() {
                *c = s;
            }
        }
    }
    fn fill_u16(&mut self, data: &mut [u16]) {
        let ch = self.out_channels as usize;
        for frame in data.chunks_mut(ch) {
            let s = ((self.next_sample() * 0.5 + 0.5) * u16::MAX as f32) as u16;
            for c in frame.iter_mut() {
                *c = s;
            }
        }
    }
}

pub struct Capture {
    _stream: Stream,
    running: Arc<AtomicBool>,
}

impl Capture {
    /// Start the mic. `on_chunk` receives ~40ms of 16-bit LE mono PCM.
    pub fn start<F>(input_name: Option<&str>, on_chunk: F) -> Result<Self>
    where
        F: Fn(&[u8], u32) + Send + Sync + 'static,
    {
        let on_chunk = Arc::new(on_chunk);
        let err = match Self::open(resolve_device(true, input_name)?, on_chunk.clone()) {
            Ok(capture) => return Ok(capture),
            Err(e) => e,
        };
        // A cached device can go stale if it was unplugged. Re-resolve once;
        // if nothing was cached the retry would just repeat the same failure.
        if !forget_device(true, input_name) {
            return Err(err);
        }
        Self::open(find_device(true, input_name)?, on_chunk)
    }

    fn open<F>(device: Device, on_chunk: Arc<F>) -> Result<Self>
    where
        F: Fn(&[u8], u32) + Send + Sync + 'static,
    {
        let supported = device.default_input_config().context("input config")?;
        let sample_format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let rate = config.sample_rate.0;
        let channels = config.channels as usize;
        let running = Arc::new(AtomicBool::new(true));
        let run = running.clone();
        let buf = Arc::new(Mutex::new(Vec::<i16>::new()));
        let target = (rate as usize / 25).max(160); // ~40ms

        let push: SamplePush = {
            let buf = buf.clone();
            let on_chunk = on_chunk.clone();
            let run = run.clone();
            Arc::new(move |samples: &[i16]| {
                if !run.load(Ordering::Relaxed) {
                    return;
                }
                let mut b = buf.lock().unwrap();
                b.extend_from_slice(samples);
                while b.len() >= target {
                    let chunk: Vec<i16> = b.drain(..target).collect();
                    let mut bytes = Vec::with_capacity(chunk.len() * 2);
                    for s in chunk {
                        bytes.extend_from_slice(&s.to_le_bytes());
                    }
                    on_chunk(&bytes, rate);
                }
            })
        };

        let err_fn = |e| eprintln!("vent-audio input: {e}");
        let stream = match sample_format {
            SampleFormat::F32 => {
                let push = push.clone();
                device.build_input_stream(
                    &config,
                    move |data: &[f32], _| {
                        let mono: Vec<i16> = data
                            .chunks(channels)
                            .map(|f| (f[0].clamp(-1.0, 1.0) * 32767.0) as i16)
                            .collect();
                        push(&mono);
                    },
                    err_fn,
                    None,
                )?
            }
            SampleFormat::I16 => {
                let push = push.clone();
                device.build_input_stream(
                    &config,
                    move |data: &[i16], _| {
                        let mono: Vec<i16> = data.chunks(channels).map(|f| f[0]).collect();
                        push(&mono);
                    },
                    err_fn,
                    None,
                )?
            }
            SampleFormat::U16 => {
                let push = push.clone();
                device.build_input_stream(
                    &config,
                    move |data: &[u16], _| {
                        let mono: Vec<i16> = data
                            .chunks(channels)
                            .map(|f| (i32::from(f[0]) - 32768) as i16)
                            .collect();
                        push(&mono);
                    },
                    err_fn,
                    None,
                )?
            }
            other => return Err(anyhow!("unsupported input format {other:?}")),
        };
        stream.play()?;
        Ok(Self {
            _stream: stream,
            running,
        })
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle() -> PlayerHandle {
        PlayerHandle {
            inner: Arc::new(Mutex::new(MixerInner {
                voices: HashMap::new(),
                muted_users: HashSet::new(),
                master_mute: false,
                out_rate: 48_000,
                out_channels: 2,
            })),
        }
    }

    fn pcm(samples: &[i16]) -> Vec<u8> {
        samples.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    #[test]
    fn master_mute_discards_buffered_audio() {
        let handle = handle();
        handle.play(7, 48_000, 1, &pcm(&[1000, 2000]));
        assert!(!handle.inner.lock().unwrap().voices.is_empty());

        handle.set_muted(true);

        assert!(handle.inner.lock().unwrap().voices.is_empty());
    }

    #[test]
    fn user_mute_discards_only_that_users_buffer() {
        let handle = handle();
        handle.play(7, 48_000, 1, &pcm(&[1000]));
        handle.play(8, 48_000, 1, &pcm(&[2000]));

        handle.set_user_muted(7, true);

        let inner = handle.inner.lock().unwrap();
        assert!(!inner.voices.contains_key(&7));
        assert!(inner.voices.contains_key(&8));
    }

    #[test]
    fn zero_rate_audio_is_ignored() {
        let handle = handle();

        handle.play(7, 0, 1, &pcm(&[1000]));

        assert!(handle.inner.lock().unwrap().voices.is_empty());
    }
}
