//! voiceflow-core — platform-independent heart of VoiceFlow.
//!
//! Everything here must compile for any target: no tauri, no cpal, no OS
//! frameworks. Shells (macOS Tauri app today; iOS/Windows later) own capture,
//! injection, persistence, and hotkeys, and call into this crate for the
//! provider layer, audio processing, and the settings model.

pub mod audio;
pub mod providers;
pub mod settings;

pub use audio::{to_wav_16k_mono, RecordedAudio};
pub use providers::{make_cleaner, make_transcriber, Cleaner, ProviderError, Transcriber};
pub use settings::{effective_key, Settings};
