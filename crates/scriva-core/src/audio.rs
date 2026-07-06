//! Platform-independent audio processing: downmix, resample, 16 kHz mono WAV
//! encoding, and the too-short/silence guards. Capture is a shell concern.

use std::io::Cursor;

/// Raw captured audio in the device's native rate/channel layout.
pub struct RecordedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Minimum recording length worth transcribing.
const MIN_DURATION_SECS: f32 = 0.4;
/// RMS below this is treated as silence (a tapped hotkey with no speech).
const SILENCE_RMS: f32 = 0.006;
const TARGET_RATE: u32 = 16_000;

/// Downmix to mono, resample to 16 kHz, and encode a 16-bit PCM WAV.
///
/// Returns `None` (skip dictation — no API call) when the clip is too short or
/// effectively silent, per the empty-audio guard.
pub fn to_wav_16k_mono(audio: &RecordedAudio) -> Option<Vec<u8>> {
    if audio.samples.is_empty() || audio.channels == 0 {
        return None;
    }

    // Downmix to mono by averaging channels.
    let channels = audio.channels as usize;
    let mono: Vec<f32> = if channels == 1 {
        audio.samples.clone()
    } else {
        audio
            .samples
            .chunks(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect()
    };

    let in_rate = audio.sample_rate as f32;
    let duration = mono.len() as f32 / in_rate;
    if duration < MIN_DURATION_SECS {
        return None;
    }
    let rms = (mono.iter().map(|s| s * s).sum::<f32>() / mono.len() as f32).sqrt();
    if rms < SILENCE_RMS {
        let peak = mono.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        eprintln!(
            "[scriva] audio RMS {rms:.6} peak {peak:.6} below silence threshold {SILENCE_RMS} — treating as silent"
        );
        return None;
    }

    let resampled = resample_linear(&mono, in_rate, TARGET_RATE as f32);

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: TARGET_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::<u8>::new());
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec).ok()?;
        for s in resampled {
            let clamped = s.clamp(-1.0, 1.0);
            let v = (clamped * i16::MAX as f32) as i16;
            writer.write_sample(v).ok()?;
        }
        writer.finalize().ok()?;
    }
    Some(cursor.into_inner())
}

/// Linear-interpolation resampler. Adequate for speech feeding a Whisper model.
fn resample_linear(input: &[f32], in_rate: f32, out_rate: f32) -> Vec<f32> {
    if input.len() < 2 || (in_rate - out_rate).abs() < 1.0 {
        return input.to_vec();
    }
    let ratio = out_rate / in_rate;
    let out_len = (input.len() as f32 * ratio).round() as usize;
    if out_len == 0 {
        return input.to_vec();
    }
    let last = input.len() - 1;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_pos = i as f32 / ratio;
        let idx = src_pos.floor() as usize;
        let frac = src_pos - idx as f32;
        let a = input[idx.min(last)];
        let b = input[(idx + 1).min(last)];
        out.push(a + (b - a) * frac);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_clip_is_skipped() {
        let audio = RecordedAudio {
            samples: vec![0.5; 1000], // ~0.02s at 48k → below MIN_DURATION
            sample_rate: 48_000,
            channels: 1,
        };
        assert!(to_wav_16k_mono(&audio).is_none());
    }

    #[test]
    fn silent_clip_is_skipped() {
        let audio = RecordedAudio {
            samples: vec![0.0; 48_000], // 1s of pure silence
            sample_rate: 48_000,
            channels: 1,
        };
        assert!(to_wav_16k_mono(&audio).is_none());
    }

    #[test]
    fn speech_like_clip_encodes_a_riff_wav() {
        // 1s of a loud tone → passes duration + RMS guards.
        let samples: Vec<f32> = (0..48_000)
            .map(|i| (i as f32 * 0.05).sin() * 0.6)
            .collect();
        let audio = RecordedAudio {
            samples,
            sample_rate: 48_000,
            channels: 1,
        };
        let wav = to_wav_16k_mono(&audio).expect("should encode");
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
    }

    #[test]
    fn stereo_is_downmixed_and_resampled_to_16k() {
        // 1s stereo tone at 48k → 16k mono, ~16000 samples.
        let mut samples = Vec::new();
        for i in 0..48_000 {
            let v = (i as f32 * 0.05).sin() * 0.6;
            samples.push(v); // L
            samples.push(v); // R
        }
        let audio = RecordedAudio {
            samples,
            sample_rate: 48_000,
            channels: 2,
        };
        let wav = to_wav_16k_mono(&audio).expect("should encode");
        // 44-byte canonical header + 2 bytes per mono i16 sample.
        let data_bytes = wav.len() - 44;
        let sample_count = data_bytes / 2;
        assert!((15_000..=17_000).contains(&sample_count), "got {sample_count}");
    }
}
