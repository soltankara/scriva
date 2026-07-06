//! Microphone capture (cpal). Capture only — the platform-independent
//! processing (downmix, resample, 16 kHz mono WAV encoding, and the
//! too-short/silence guards) lives in `voiceflow_core::audio`.
//!
//! A cpal `Stream` is `!Send`, so a dedicated OS thread owns it for the life of
//! a recording. The stream callback ships sample chunks over a channel; on stop
//! the thread drops the stream, drains the buffer, and returns the audio.

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};

pub use voiceflow_core::audio::{to_wav_16k_mono, RecordedAudio};

/// Current microphone authorization: `"granted"`, `"denied"`, or
/// `"undetermined"` (macOS; other platforms report `"granted"`).
pub fn mic_status() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        mic_permission::mic_status()
    }
    #[cfg(not(target_os = "macos"))]
    {
        "granted"
    }
}

/// Trigger the one-time macOS microphone prompt. No-op once a decision has been
/// made, and a no-op on non-macOS platforms.
pub fn request_mic_access() {
    #[cfg(target_os = "macos")]
    {
        mic_permission::request_mic_access();
    }
}

/// Query and drive macOS microphone authorization via AVCaptureDevice. This is
/// a status query / prompt trigger only — the actual capture path (cpal) is
/// untouched, so nothing here runs on the hot path.
#[cfg(target_os = "macos")]
mod mic_permission {
    use block2::RcBlock;
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject, Bool};

    #[link(name = "AVFoundation", kind = "framework")]
    extern "C" {
        // NSString constant identifying the audio media type.
        static AVMediaTypeAudio: *mut AnyObject;
    }

    fn av_capture_device() -> &'static AnyClass {
        AnyClass::get(c"AVCaptureDevice").expect("AVFoundation linked via #[link] above")
    }

    /// AVAuthorizationStatus: 0 NotDetermined, 1 Restricted, 2 Denied, 3 Authorized.
    /// The selector returns an `NSInteger`, bound here as `isize`.
    pub fn mic_status() -> &'static str {
        unsafe {
            let s: isize = msg_send![
                av_capture_device(),
                authorizationStatusForMediaType: AVMediaTypeAudio,
            ];
            match s {
                3 => "granted",
                0 => "undetermined",
                _ => "denied",
            }
        }
    }

    /// Triggers the one-time macOS mic prompt (only has an effect when the
    /// status is undetermined). The completion handler intentionally does
    /// nothing — the settings UI polls `check_permissions` and picks up the new
    /// status. The block escapes the call, so it must live on the heap (RcBlock).
    pub fn request_mic_access() {
        unsafe {
            let handler = RcBlock::new(|_granted: Bool| {});
            let _: () = msg_send![
                av_capture_device(),
                requestAccessForMediaType: AVMediaTypeAudio,
                completionHandler: &*handler,
            ];
        }
    }
}

/// Handle to an in-flight recording. Dropping it (or calling `stop_and_collect`)
/// signals the capture thread to finish.
pub struct RecorderHandle {
    stop_tx: Sender<()>,
    result_rx: Receiver<RecordedAudio>,
}

impl RecorderHandle {
    /// Stop capture and return the recorded audio. Blocking — call from a
    /// blocking context (e.g. `spawn_blocking`).
    pub fn stop_and_collect(self) -> Result<RecordedAudio, String> {
        let _ = self.stop_tx.send(());
        self.result_rx
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "Could not read the recorded audio.".to_string())
    }
}

/// Open the default input device and begin capturing. Blocks briefly until the
/// stream is confirmed open (so the caller can flip the mic-permission flag),
/// then returns a handle. Errors carry a human-readable, key-free message.
pub fn start_recording() -> Result<RecorderHandle, String> {
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let (result_tx, result_rx) = mpsc::channel::<RecordedAudio>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();

    std::thread::spawn(move || {
        let host = cpal::default_host();
        let device = match host.default_input_device() {
            Some(d) => d,
            None => {
                let _ = ready_tx.send(Err("No microphone input device was found.".to_string()));
                return;
            }
        };
        let name = device.name().unwrap_or_else(|_| "unknown".into());
        eprintln!("[voiceflow] recording from input device: {name}");
        let supported = match device.default_input_config() {
            Ok(c) => c,
            Err(e) => {
                let _ = ready_tx.send(Err(e.to_string()));
                return;
            }
        };

        let sample_format = supported.sample_format();
        let sample_rate = supported.sample_rate().0;
        let channels = supported.channels();
        let config: StreamConfig = supported.config();

        let (sample_tx, sample_rx) = mpsc::channel::<Vec<f32>>();
        let err_fn = |_e: cpal::StreamError| {};

        let stream_result = match sample_format {
            SampleFormat::F32 => {
                let tx = sample_tx.clone();
                device.build_input_stream(
                    &config,
                    move |data: &[f32], _: &_| {
                        let _ = tx.send(data.to_vec());
                    },
                    err_fn,
                    None,
                )
            }
            SampleFormat::I16 => {
                let tx = sample_tx.clone();
                device.build_input_stream(
                    &config,
                    move |data: &[i16], _: &_| {
                        let v = data.iter().map(|s| *s as f32 / i16::MAX as f32).collect();
                        let _ = tx.send(v);
                    },
                    err_fn,
                    None,
                )
            }
            SampleFormat::U16 => {
                let tx = sample_tx.clone();
                device.build_input_stream(
                    &config,
                    move |data: &[u16], _: &_| {
                        let v = data
                            .iter()
                            .map(|s| (*s as f32 / u16::MAX as f32) * 2.0 - 1.0)
                            .collect();
                        let _ = tx.send(v);
                    },
                    err_fn,
                    None,
                )
            }
            other => {
                let _ = ready_tx.send(Err(format!("Unsupported audio sample format: {other:?}")));
                return;
            }
        };

        let stream = match stream_result {
            Ok(s) => s,
            Err(e) => {
                let _ = ready_tx.send(Err(e.to_string()));
                return;
            }
        };
        if let Err(e) = stream.play() {
            let _ = ready_tx.send(Err(e.to_string()));
            return;
        }

        // Stream is live.
        let _ = ready_tx.send(Ok(()));

        // Block until asked to stop, then drop the stream to halt callbacks and
        // drain whatever the callback buffered.
        let _ = stop_rx.recv();
        drop(stream);

        let mut samples = Vec::new();
        while let Ok(chunk) = sample_rx.try_recv() {
            samples.extend(chunk);
        }
        let _ = result_tx.send(RecordedAudio {
            samples,
            sample_rate,
            channels,
        });
    });

    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(RecorderHandle { stop_tx, result_rx }),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("Timed out opening the microphone.".to_string()),
    }
}
