//! Voice chat on Android (plan: `notes/voice-mod.md`, milestones V6/V7): microphone capture and playback
//! ([`audio`]), the voice UDP port link ([`udp`]; voice frames travel only there), the voice client of a
//! connection ([`link`]: `bhmod_voice`'s `VoiceClient` does activation, Opus, sealing, jitter buffer and mixing) and
//! the per-frame [`pipeline`] that moves audio and packets between them. The loader owns one pipeline per world on
//! a voice-enabled server; nothing opens before a mod asks for voice.

pub mod audio;
pub mod client;
pub mod link;
pub mod pipeline;
pub mod udp;
