use dsp::FramesPerTick;

use crate::analog_word::{AnalogWord, PreampCode};
use crate::phantom::PhantomGrant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnalogSettings {
    pub preamp_one: PreampCode,
    pub preamp_two: PreampCode,
    pub speed: FramesPerTick,
}

impl AnalogSettings {
    pub const AT_BOOT: Self = Self {
        preamp_one: PreampCode::MINIMUM,
        preamp_two: PreampCode::MINIMUM,
        speed: FramesPerTick::Six,
    };

    pub const fn word(self, grant: PhantomGrant) -> AnalogWord {
        AnalogWord::new(self.preamp_one, self.preamp_two, self.speed, grant)
    }
}
