//! When the microphone transmits, and in which mode.
//!
//! - **Push-to-talk**, one key per mode (PC: V proximity, B channel, G global, ...). Held keys win over everything;
//!   the most recently pressed one decides the mode.
//! - **Toggle** and **voice activation** (VAD) transmit in the player's default mode.
//! - The admin decides which activations and modes exist; a choice the admin does not allow falls back to the first
//!   allowed one in the order PTT, VAD, toggle. Self-mute stops everything.

use crate::VoiceMode;

/// How the default mode is activated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Push-to-talk only.
    Ptt,
    /// Voice activation.
    Vad,
    /// A key toggles transmitting.
    Toggle,
}

/// Activations the admin allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Allowed {
    /// PTT keys work.
    pub ptt: bool,
    /// VAD may be chosen.
    pub vad: bool,
    /// Toggle may be chosen.
    pub toggle: bool,
}

impl Default for Allowed {
    fn default() -> Allowed {
        Allowed { ptt: true, vad: true, toggle: true }
    }
}

impl Allowed {
    fn has(self, k: Kind) -> bool {
        match k {
            Kind::Ptt => self.ptt,
            Kind::Vad => self.vad,
            Kind::Toggle => self.toggle,
        }
    }
}

/// An input to the state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    /// A mode's PTT key went down.
    PttDown(VoiceMode),
    /// A mode's PTT key went up.
    PttUp(VoiceMode),
    /// The toggle key was pressed.
    Toggle,
    /// The VAD result of the latest frame.
    Vad(bool),
    /// Self-mute switched.
    SelfMute(bool),
}

/// The activation state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activation {
    /// Activations the admin allows.
    pub allowed: Allowed,
    /// Modes the admin enabled ([`VoiceMode::bit`] mask).
    pub modes: u8,
    /// The player's choice for the default mode.
    pub chosen: Kind,
    /// The player's default mode.
    pub default_mode: VoiceMode,
    held: Vec<VoiceMode>,
    toggled: bool,
    vad: bool,
    self_mute: bool,
}

impl Default for Activation {
    fn default() -> Activation {
        Activation::new(Allowed::default(), 0x0f, Kind::Ptt, VoiceMode::Proximity)
    }
}

impl Activation {
    /// A state machine; the microphone starts off.
    pub fn new(allowed: Allowed, modes: u8, chosen: Kind, default_mode: VoiceMode) -> Activation {
        Activation {
            allowed,
            modes,
            chosen,
            default_mode,
            held: Vec::new(),
            toggled: false,
            vad: false,
            self_mute: false,
        }
    }

    /// The activation actually in effect for the default mode (`None`: the admin allows nothing).
    pub fn effective(&self) -> Option<Kind> {
        if self.allowed.has(self.chosen) {
            return Some(self.chosen);
        }
        [Kind::Ptt, Kind::Vad, Kind::Toggle].into_iter().find(|k| self.allowed.has(*k))
    }

    fn enabled(&self, m: VoiceMode) -> bool {
        self.modes & m.bit() != 0
    }

    /// Feeds an input; returns [`Activation::transmitting`] afterwards.
    pub fn handle(&mut self, input: Input) -> Option<VoiceMode> {
        match input {
            Input::PttDown(m) => {
                self.held.retain(|h| *h != m);
                self.held.push(m);
            }
            Input::PttUp(m) => self.held.retain(|h| *h != m),
            Input::Toggle => self.toggled = !self.toggled,
            Input::Vad(v) => self.vad = v,
            Input::SelfMute(m) => self.self_mute = m,
        }
        self.transmitting()
    }

    /// The mode the microphone transmits in now, or `None`.
    pub fn transmitting(&self) -> Option<VoiceMode> {
        if self.self_mute {
            return None;
        }
        if self.allowed.ptt {
            if let Some(m) = self.held.iter().rev().find(|m| self.enabled(**m)) {
                return Some(*m);
            }
        }
        let on = match self.effective()? {
            Kind::Ptt => false,
            Kind::Vad => self.vad,
            Kind::Toggle => self.toggled,
        };
        (on && self.enabled(self.default_mode)).then_some(self.default_mode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use VoiceMode::*;

    #[test]
    fn ptt_per_mode() {
        let mut a = Activation::default();
        assert_eq!(a.handle(Input::PttDown(Proximity)), Some(Proximity));
        assert_eq!(a.handle(Input::PttDown(Global)), Some(Global), "latest key wins");
        assert_eq!(a.handle(Input::PttDown(Proximity)), Some(Proximity), "pressed again: latest");
        assert_eq!(a.handle(Input::PttUp(Proximity)), Some(Global));
        assert_eq!(a.handle(Input::PttUp(Global)), None);
        a.modes = Proximity.bit();
        assert_eq!(a.handle(Input::PttDown(Global)), None, "mode disabled by the admin");
        a.allowed.ptt = false;
        assert_eq!(a.handle(Input::PttDown(Proximity)), None);
    }

    #[test]
    fn vad_and_toggle() {
        let mut a = Activation::new(Allowed::default(), 0x0f, Kind::Vad, Channel);
        assert_eq!(a.handle(Input::Vad(true)), Some(Channel));
        assert_eq!(a.handle(Input::PttDown(Global)), Some(Global), "PTT beats VAD");
        assert_eq!(a.handle(Input::PttUp(Global)), Some(Channel));
        assert_eq!(a.handle(Input::SelfMute(true)), None);
        assert_eq!(a.handle(Input::SelfMute(false)), Some(Channel));
        assert_eq!(a.handle(Input::Vad(false)), None);

        a.chosen = Kind::Toggle;
        assert_eq!(a.handle(Input::Toggle), Some(Channel));
        a.modes = Global.bit();
        assert_eq!(a.transmitting(), None, "default mode disabled");
        a.modes = 0x0f;
        assert_eq!(a.handle(Input::Toggle), None);
    }

    #[test]
    fn admin_restrictions() {
        let ptt_only = Allowed { ptt: true, vad: false, toggle: false };
        let mut a = Activation::new(ptt_only, 0x0f, Kind::Vad, Proximity);
        assert_eq!(a.effective(), Some(Kind::Ptt));
        assert_eq!(a.handle(Input::Vad(true)), None, "VAD not allowed");
        a.allowed = Allowed { ptt: false, vad: false, toggle: true };
        assert_eq!(a.effective(), Some(Kind::Toggle));
        a.allowed = Allowed { ptt: false, vad: false, toggle: false };
        assert_eq!(a.effective(), None);
        assert_eq!(a.transmitting(), None);
        a.chosen = Kind::Ptt;
        a.allowed.ptt = true;
        assert_eq!(a.handle(Input::Vad(true)), None, "PTT chosen: VAD ignored");
    }
}
