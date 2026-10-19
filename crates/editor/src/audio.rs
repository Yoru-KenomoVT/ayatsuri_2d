//! Audio for compositions: a layer can be a sound file. The file is decoded
//! once into samples (for the waveform, playback and the export mix), plays
//! through the system's output while the composition plays, and is mixed
//! down to a WAV that the Render window hands ffmpeg beside the frames.
//!
//! Playback is a pull: the output asks the mixer for frames and the mixer
//! sums the voices playing. On Linux the output speaks the PulseAudio
//! protocol straight to PipeWire (see `pulse.rs`); elsewhere, and when no
//! sound server answers, cpal drives the device.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
#[cfg(target_os = "linux")]
use std::time::Duration;

use ayatsuri_format::Layer;
use rodio::Source;

/// Peaks per second of the waveform strip.
pub const PEAKS_PER_S: f32 = 200.0;
/// The export mix's rate.
pub const MIX_RATE: u32 = 48_000;

/// A decoded sound: interleaved samples, and the waveform's peaks.
pub struct Clip {
    pub rate: u32,
    pub channels: u16,
    pub samples: Vec<f32>,
    /// The largest absolute sample over each stretch of 1/200 s, mono.
    pub peaks: Vec<f32>,
}

impl Clip {
    /// Seconds.
    pub fn len(&self) -> f32 {
        self.samples.len() as f32 / (self.rate as f32 * self.channels.max(1) as f32)
    }

    /// The sample of `channel` at `t` seconds, nearest frame, zero past the
    /// ends.
    fn at(&self, t: f32, channel: usize) -> f32 {
        if t < 0.0 {
            return 0.0;
        }
        let ch = self.channels.max(1) as usize;
        let frame = (t * self.rate as f32) as usize;
        self.samples.get(frame * ch + channel.min(ch - 1)).copied().unwrap_or(0.0)
    }
}

/// Decode a sound file (WAV, MP3, FLAC or Ogg Vorbis) into a clip.
pub fn decode(path: &Path) -> Result<Clip, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let dec = rodio::Decoder::new(std::io::BufReader::new(file))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let rate = dec.sample_rate().get();
    let channels = dec.channels().get();
    let samples: Vec<f32> = dec.collect();
    if samples.is_empty() {
        return Err(format!("{}: no samples", path.display()));
    }
    let per = ((rate as f32 / PEAKS_PER_S).round() as usize).max(1);
    let ch = channels.max(1) as usize;
    let peaks =
        samples.chunks(per * ch).map(|c| c.iter().fold(0.0f32, |m, s| m.max(s.abs()))).collect();
    Ok(Clip { rate, channels, samples, peaks })
}

/// One sound to start: the clip, how far into it, and how to play it.
pub struct Cue {
    pub clip: Arc<Clip>,
    /// Seconds into the clip.
    pub offset: f32,
    /// Seconds of the clip to play from there.
    pub len: f32,
    pub speed: f32,
    /// The layer, for its weight and fades, and the composition time the
    /// cue starts at, so the fades move while it plays.
    pub layer: Layer,
    pub at: f32,
    /// The layer's index, so the mixer's meters know whose sound it is.
    pub li: usize,
}

/// Peak levels since they were last read: per layer and for the main
/// out, left and right, as linear gain.
#[derive(Clone, Default)]
pub struct Levels {
    pub layers: HashMap<usize, [f32; 2]>,
    pub main: [f32; 2],
}

/// A sound playing: where it is in its clip, in the clip's own frames, and
/// where the composition's clock is, for the layer's fades.
struct Voice {
    clip: Arc<Clip>,
    pos: f64,
    end: f64,
    speed: f64,
    layer: Layer,
    t: f64,
    li: usize,
}

/// What both outputs pull from: the voices playing, and the master volume
/// as f32 bits.
#[derive(Clone, Default)]
struct Mixer {
    voices: Arc<Mutex<Vec<Voice>>>,
    volume: Arc<AtomicU32>,
    /// Peaks since the palette last looked.
    levels: Arc<Mutex<Levels>>,
}

impl Mixer {
    fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Relaxed))
    }

    /// Fill `buf` (interleaved, `channels` wide) at `rate` from the voices,
    /// dropping the ones that have played out.
    fn fill(&self, buf: &mut [f32], channels: usize, rate: u32) {
        buf.fill(0.0);
        let master = self.volume();
        let channels = channels.max(1);
        let frames = buf.len() / channels;
        let mut voices = self.voices.lock().unwrap_or_else(PoisonError::into_inner);
        let dt = 1.0 / rate.max(1) as f64;
        let mut levels = self.levels.lock().unwrap_or_else(PoisonError::into_inner);
        for v in voices.iter_mut() {
            let step = v.speed * v.clip.rate as f64 / rate.max(1) as f64;
            let ch = v.clip.channels.max(1) as usize;
            let mut peak = [0.0f32; 2];
            for f in 0..frames {
                if v.pos >= v.end {
                    break;
                }
                // the layer's weight and fades at this moment of the composition
                let gain = ayatsuri_core::comp::layer_weight(&v.layer, v.t as f32) * master;
                v.t += dt;
                let i0 = v.pos as usize;
                let t = (v.pos - i0 as f64) as f32;
                for c in 0..channels.min(2) {
                    let cc = c.min(ch - 1);
                    let a = v.clip.samples.get(i0 * ch + cc).copied().unwrap_or(0.0);
                    let b = v.clip.samples.get((i0 + 1) * ch + cc).copied().unwrap_or(a);
                    let s = (a + (b - a) * t) * gain;
                    if let Some(pk) = peak.get_mut(c) {
                        *pk = pk.max(s.abs());
                    }
                    if let Some(slot) = buf.get_mut(f * channels + c) {
                        *slot += s;
                    }
                }
                v.pos += step;
            }
            // a mono source meters the same on both sides
            if ch == 1 {
                peak[1] = peak[0];
            }
            let slot = levels.layers.entry(v.li).or_default();
            slot[0] = slot[0].max(peak[0]);
            slot[1] = slot[1].max(peak[1]);
        }
        voices.retain(|v| v.pos < v.end);
        let mut main = [0.0f32; 2];
        for (i, s) in buf.iter_mut().enumerate() {
            *s = s.clamp(-1.0, 1.0);
            if let Some(m) = main.get_mut(i % channels.max(1)) {
                *m = m.max(s.abs());
            }
        }
        levels.main[0] = levels.main[0].max(main[0]);
        levels.main[1] = levels.main[1].max(main[1]);
    }
}

/// How the sound reaches the system; held so the stream stays open, and
/// dropped to close it.
enum Backend {
    #[cfg(target_os = "linux")]
    Pulse(#[allow(dead_code)] crate::pulse::Playback),
    Cpal(#[allow(dead_code)] cpal::Stream),
}

/// The system output, fed from the mixer.
pub struct Out {
    backend: Backend,
    mixer: Mixer,
    /// What opened, for Settings.
    pub describe: String,
}

/// How the user wants sound out: a device by name (None for the default),
/// the master volume and the buffer size in frames.
#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    pub device: Option<String>,
    pub volume: f32,
    pub buffer_frames: u32,
}

/// An output device the user can pick.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub name: String,
    pub description: String,
}

/// The output devices: the sound server's sinks on Linux, else cpal's.
pub fn devices() -> Vec<Device> {
    #[cfg(target_os = "linux")]
    if crate::pulse::available()
        && let Ok(sinks) = crate::pulse::list_sinks(Duration::from_secs(1))
    {
        return sinks
            .into_iter()
            .map(|s| Device { name: s.name, description: s.description })
            .collect();
    }
    use cpal::traits::{DeviceTrait, HostTrait};
    cpal::default_host()
        .output_devices()
        .map(|it| {
            it.filter_map(|d| d.description().ok().map(|n| n.to_string()))
                .map(|n| Device { name: n.clone(), description: n })
                .collect()
        })
        .unwrap_or_default()
}

impl Out {
    /// Open the output the preferences ask for, or none when there is no
    /// sound to be had.
    pub fn open(prefs: &Prefs) -> Option<Self> {
        let mixer = Mixer::default();
        mixer.volume.store(prefs.volume.clamp(0.0, 2.0).to_bits(), Ordering::Relaxed);
        let frames = prefs.buffer_frames.clamp(128, 8192);
        #[cfg(target_os = "linux")]
        if crate::pulse::available() {
            let m = mixer.clone();
            match crate::pulse::Playback::start(
                MIX_RATE,
                2,
                prefs.device.clone(),
                frames,
                Box::new(move |buf, ch| m.fill(buf, ch, MIX_RATE)),
            ) {
                Ok(pb) => {
                    return Some(Self {
                        backend: Backend::Pulse(pb),
                        mixer,
                        describe: "PipeWire / PulseAudio".into(),
                    });
                }
                Err(e) => log::warn!("audio: pulse: {e}; trying cpal"),
            }
        }
        Self::open_cpal(prefs, mixer, frames)
    }

    fn open_cpal(prefs: &Prefs, mixer: Mixer, frames: u32) -> Option<Self> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let host = cpal::default_host();
        let device = match &prefs.device {
            Some(want) => host
                .output_devices()
                .ok()
                .and_then(|mut it| {
                    it.find(|d| d.description().map(|n| n.to_string() == *want).unwrap_or(false))
                })
                .or_else(|| host.default_output_device()),
            None => host.default_output_device(),
        }?;
        let supported =
            device.default_output_config().map_err(|e| log::warn!("audio: cpal: {e}")).ok()?;
        let rate = supported.sample_rate();
        let channels = supported.channels() as usize;
        let mut config: cpal::StreamConfig = supported.into();
        config.buffer_size = cpal::BufferSize::Fixed(frames);
        let m = mixer.clone();
        let build = |cfg: &cpal::StreamConfig| {
            let m = m.clone();
            device.build_output_stream(
                *cfg,
                move |buf: &mut [f32], _| m.fill(buf, channels, rate),
                |e| log::warn!("audio: cpal: {e}"),
                None,
            )
        };
        let stream = match build(&config) {
            Ok(s) => s,
            Err(_) => {
                // not every device takes a fixed buffer; let it choose
                config.buffer_size = cpal::BufferSize::Default;
                build(&config).map_err(|e| log::warn!("audio: cpal: {e}")).ok()?
            }
        };
        stream.play().map_err(|e| log::warn!("audio: cpal: {e}")).ok()?;
        let name = device.description().map(|n| n.to_string()).unwrap_or_default();
        Some(Self {
            backend: Backend::Cpal(stream),
            mixer,
            describe: format!("{} ({name})", host.id().name()),
        })
    }

    /// True while the stream is alive; a dead sound server shows here.
    pub fn alive(&self) -> bool {
        match &self.backend {
            #[cfg(target_os = "linux")]
            Backend::Pulse(pb) => !pb.failed(),
            Backend::Cpal(_) => true,
        }
    }

    pub fn set_volume(&self, v: f32) {
        self.mixer.volume.store(v.clamp(0.0, 2.0).to_bits(), Ordering::Relaxed);
    }

    /// The peaks since the last call, which clears them.
    pub fn take_levels(&self) -> Levels {
        std::mem::take(&mut *self.mixer.levels.lock().unwrap_or_else(PoisonError::into_inner))
    }

    pub fn stop(&mut self) {
        self.mixer.voices.lock().unwrap_or_else(PoisonError::into_inner).clear();
    }

    /// Stop whatever plays and start these.
    pub fn start(&mut self, cues: Vec<Cue>) {
        let mut voices = self.mixer.voices.lock().unwrap_or_else(PoisonError::into_inner);
        voices.clear();
        for cue in cues {
            if cue.offset >= cue.clip.len() || cue.len <= 0.0 || cue.layer.weight <= 0.0 {
                continue;
            }
            let rate = cue.clip.rate as f64;
            let pos = cue.offset as f64 * rate;
            let frames = cue.clip.samples.len() / cue.clip.channels.max(1) as usize;
            voices.push(Voice {
                clip: cue.clip,
                pos,
                end: (pos + cue.len as f64 * rate).min(frames as f64),
                speed: cue.speed.max(0.05) as f64,
                layer: cue.layer,
                t: cue.at as f64,
                li: cue.li,
            });
        }
    }

    /// A short tone, so Settings can prove the output works.
    pub fn test_tone(&mut self) {
        let rate = 48_000u32;
        let n = rate as usize / 3;
        let samples: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / rate as f32;
                let env = (1.0 - i as f32 / n as f32).min(t * 40.0).clamp(0.0, 1.0);
                (t * 440.0 * std::f32::consts::TAU).sin() * 0.3 * env
            })
            .collect();
        let clip = Arc::new(Clip { rate, channels: 1, samples, peaks: Vec::new() });
        let layer = Layer { out_point: 1.0, ..Layer::default() };
        self.start(vec![Cue {
            clip,
            offset: 0.0,
            len: 1.0,
            speed: 1.0,
            layer,
            at: 0.0,
            li: usize::MAX,
        }]);
    }
}

/// The composition's sound between `start` and `end` seconds, mixed to
/// stereo 16-bit at the mix rate. None when no layer makes a sound.
pub fn mix(layers: &[(Arc<Clip>, Layer)], start: f32, end: f32) -> Option<Vec<i16>> {
    let frames = ((end - start).max(0.0) * MIX_RATE as f32).round() as usize;
    if frames == 0 || layers.is_empty() {
        return None;
    }
    let mut out = vec![0.0f32; frames * 2];
    let mut any = false;
    for (clip, l) in layers {
        let speed = l.speed.max(0.05);
        for k in 0..frames {
            let t = start + k as f32 / MIX_RATE as f32;
            let w = ayatsuri_core::comp::layer_weight(l, t);
            if w <= 0.0 {
                continue;
            }
            let st = l.in_point + (t - l.start) * speed;
            if st > l.out_point {
                continue;
            }
            any = true;
            if let Some([left, right]) = out.get_mut(k * 2..k * 2 + 2) {
                *left += clip.at(st, 0) * w;
                *right += clip.at(st, 1) * w;
            }
        }
    }
    any.then(|| out.iter().map(|s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).collect())
}

/// Write stereo 16-bit samples at the mix rate as a WAV file.
pub fn write_wav(path: &Path, samples: &[i16]) -> Result<(), String> {
    let data_len = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&2u16.to_le_bytes()); // stereo
    bytes.extend_from_slice(&MIX_RATE.to_le_bytes());
    bytes.extend_from_slice(&(MIX_RATE * 4).to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayatsuri_format::LayerSource;

    fn tone(rate: u32, secs: f32) -> Arc<Clip> {
        let n = (rate as f32 * secs) as usize;
        let samples: Vec<f32> = (0..n).map(|i| if i % 2 == 0 { 0.5 } else { -0.5 }).collect();
        let peaks = vec![0.5; (secs * PEAKS_PER_S) as usize];
        Arc::new(Clip { rate, channels: 1, samples, peaks })
    }

    #[test]
    fn a_mix_follows_the_layer_and_its_weight() {
        let clip = tone(1000, 2.0);
        let layer = Layer {
            source: LayerSource::Audio("t.wav".into()),
            start: 1.0,
            in_point: 0.0,
            out_point: 2.0,
            weight: 0.5,
            ..Layer::default()
        };
        let out = mix(&[(clip, layer)], 0.0, 2.0).unwrap();
        assert_eq!(out.len(), 2 * 2 * MIX_RATE as usize);
        // silence before the layer starts, a quarter-scale tone after
        assert_eq!(out[0], 0);
        let late = out[(MIX_RATE as usize + 10) * 2].unsigned_abs();
        assert!((late as f32 - 0.25 * i16::MAX as f32).abs() < 2.0);
    }

    #[test]
    fn a_wav_has_the_header_it_claims() {
        let dir = std::env::temp_dir().join(format!("ayatsuri-wav-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("t.wav");
        write_wav(&p, &[0, 1, -1, 2]).unwrap();
        let b = std::fs::read(&p).unwrap();
        assert_eq!(&b[..4], b"RIFF");
        assert_eq!(b.len(), 44 + 8);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
