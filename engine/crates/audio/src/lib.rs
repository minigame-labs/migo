//! # Audio Processing Module
//!
//! This crate provides audio playback and processing capabilities for the Migo engine,
//! implementing both WebAudio API semantics and InnerAudioContext API.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                        Audio Thread                              │
//! │  ┌─────────────┐    ┌─────────────┐    ┌─────────────────────┐ │
//! │  │AudioContext │    │InnerAudio   │    │     AudioOutput     │ │
//! │  │  (WebAudio) │    │  Player     │───►│  (cpal/oboe/ALSA)   │ │
//! │  │             │    │             │    │                     │ │
//! │  │ ┌─────────┐ │    │ ┌─────────┐ │    └─────────────────────┘ │
//! │  │ │Src Node │ │    │ │Decoder  │ │                            │
//! │  │ └────┬────┘ │    │ └─────────┘ │                            │
//! │  │      │      │    │             │                            │
//! │  │ ┌────▼────┐ │    │ ┌─────────┐ │                            │
//! │  │ │GainNode │ │    │ │Resampler│ │                            │
//! │  │ └────┬────┘ │    │ └─────────┘ │                            │
//! │  │      │      │    │             │                            │
//! │  │ ┌────▼────┐ │    │ ┌─────────┐ │                            │
//! │  │ │  Dest   │─┼────┼─│  Mixer  │─┼────────────────────────────┤
//! │  │ └─────────┘ │    │ └─────────┘ │                            │
//! │  └─────────────┘    └─────────────┘                            │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Features
//!
//! - **WebAudio API**: Supports `AudioContext`, `AudioBufferSourceNode`, `GainNode`
//! - **InnerAudioContext**: audio player with streaming support
//! - **Format Support**: MP3, OGG/Vorbis, WAV, FLAC (via symphonia)
//! - **Streaming**: Edge-download-edge-play for large audio files
//! - **Caching**: LRU cache for decoded audio to avoid repeated decoding
//! - **Resampling**: Automatic sample rate conversion for device compatibility
//! - **3-Level Power Management**: Active (5ms) / LowPower (50ms) / event-driven Sleep
//!
//! ## Usage
//!
//! The audio module is typically not used directly. Instead, commands are sent
//! via the [`AudioCmd`](shared::protocol::audio_cmd::AudioCmd) protocol:
//!
//! ```rust,ignore
//! use shared::protocol::audio_cmd::AudioCmd;
//! use tokio::sync::oneshot;
//!
//! // Create an InnerAudioContext
//! audio_tx.send(AudioCmd::CreateInnerAudio { id: 1 });
//!
//! // Load audio from URL (with streaming)
//! let (resp_tx, resp_rx) = oneshot::channel();
//! audio_tx.send(AudioCmd::InnerAudioLoadUrl {
//!     id: 1,
//!     url: "https://example.com/music.mp3".to_string(),
//!     resp: resp_tx,
//! });
//!
//! // Start playback
//! audio_tx.send(AudioCmd::InnerAudioPlay { id: 1 });
//! ```
//!
//! ## Platform Support
//!
//! - **Android**: Uses Oboe (AAudio/OpenSL ES) via cpal
//! - **Linux**: Uses ALSA via cpal
//! - **macOS/iOS**: Uses CoreAudio via cpal
//!
//! ## Module Structure
//!
//! - [`audio_thread`]: Main audio processing thread and command handler
//! - [`cache`]: LRU audio cache for decoded audio data
//! - [`streaming`]: HTTP streaming download and progressive decoding
//! - [`power_manager`]: 3-level power state management

mod audio_thread;
pub mod cache;
mod context;
mod decoder;
mod inner_audio;
mod limits;
mod nodes;
mod off_worker;
mod output;
pub mod param;
pub mod power_manager;
mod resampler;
pub mod streaming;

pub use audio_thread::*;
pub use cache::GlobalAudioCache;
pub use power_manager::{AudioPowerConfig, AudioPowerManager, AudioPowerState};
