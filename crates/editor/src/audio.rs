//! Audio for compositions: a layer can be a sound file. The file is decoded
//! once into samples (for the waveform, playback and the export mix), plays
//! through the system's output while the composition plays, and is mixed
//! down to a WAV that the Render window hands ffmpeg beside the frames.

use std::num::NonZero;
use std::path::Path;
use std::sync::Arc;
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
    pub volume: f32,
}

/// The system output, and the players started for the cues now playing.
pub struct Out {
    sink: rodio::MixerDeviceSink,
    players: Vec<rodio::Player>,
}

impl Out {
    /// The default output device, or none when there is no sound to be had.
    pub fn open() -> Option<Self> {
        let mut sink = rodio::DeviceSinkBuilder::from_default_device()
            .and_then(|b| b.open_stream())
            .map_err(|e| log::warn!("audio: no output: {e}"))
            .ok()?;
        sink.log_on_drop(false);
        Some(Self { sink, players: Vec::new() })
    }

    pub fn stop(&mut self) {
        for p in self.players.drain(..) {
            p.stop();
        }
    }

    /// Stop whatever plays and start these.
    pub fn start(&mut self, cues: Vec<Cue>) {
        self.stop();
        for cue in cues {
            if cue.offset >= cue.clip.len() || cue.len <= 0.0 || cue.volume <= 0.0 {
                continue;
            }
            let (Some(ch), Some(rate)) =
                (NonZero::new(cue.clip.channels.max(1)), NonZero::new(cue.clip.rate.max(1)))
            else {
                continue;
            };
            let skip = (cue.offset * cue.clip.rate as f32) as usize * cue.clip.channels as usize;
            let data: Vec<f32> = cue.clip.samples.get(skip..).unwrap_or(&[]).to_vec();
            let source = rodio::buffer::SamplesBuffer::new(ch, rate, data)
                .take_duration(Duration::from_secs_f32(cue.len.max(0.0)))
                .speed(cue.speed.max(0.05))
                .amplify(cue.volume.clamp(0.0, 1.0));
            let player = rodio::Player::connect_new(self.sink.mixer());
            player.append(source);
            self.players.push(player);
        }
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
