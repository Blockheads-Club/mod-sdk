//! Voice chat on Android (plan: `notes/voice-mod.md`, milestones V6/V7): microphone capture and playback
//! ([`audio`]), the unsequenced 0xBC send path ([`net`]), the UDP fallback ([`udp`]), the voice client of a
//! connection ([`link`]: `bhmod_voice`'s `VoiceClient` does activation, Opus, sealing, jitter buffer and mixing) and
//! the per-frame [`pipeline`] that moves audio and packets between them. The loader owns one pipeline per world on
//! a voice-enabled server; nothing opens before a mod asks for voice.

pub mod audio;
pub mod link;
pub mod net;
pub mod pipeline;
pub mod udp;
