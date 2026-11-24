use dsp::{Gain, MonitorMix};

use crate::expander::ButtonLevel;
use crate::knob_law::MonitorBlend;
use crate::phantom::{Button, Milliseconds, Release};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Monitoring {
    Off,
    On,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MonitorSwitch {
    monitoring: Monitoring,
    button: Button,
}

impl MonitorSwitch {
    pub(crate) const AT_BOOT: Self = Self {
        monitoring: Monitoring::Off,
        button: Button::Released,
    };

    // WHY: Phantom.rs, a press held past LONGEST_COUNTED_PRESS becomes HeldTooLong and its release is ignored, so the moment it crosses one second is a long press that the short-press monitor toggle never sees
    pub(crate) const fn became_a_long_press(self, next: Self) -> bool {
        matches!(
            (self.button, next.button),
            (Button::Held { .. }, Button::HeldTooLong)
        )
    }

    pub(crate) const fn monitoring(self) -> Monitoring {
        self.monitoring
    }

    pub(crate) const fn after(self, level: ButtonLevel, elapsed: Milliseconds) -> Self {
        let button = self.button.elapse(elapsed);
        match level {
            ButtonLevel::Pressed => Self {
                monitoring: self.monitoring,
                button: button.press(),
            },
            ButtonLevel::Released => Self {
                monitoring: match button.release() {
                    Release::Counted => self.monitoring.toggled(),
                    Release::Ignored => self.monitoring,
                },
                button: Button::Released,
            },
        }
    }
}

impl Monitoring {
    const fn toggled(self) -> Self {
        match self {
            Monitoring::Off => Monitoring::On,
            Monitoring::On => Monitoring::Off,
        }
    }

    const fn input_gain(self, blend: MonitorBlend) -> Gain {
        match self {
            Monitoring::Off => Gain::ZERO,
            Monitoring::On => blend.inputs,
        }
    }
}

pub(crate) const fn monitor_mix(switches: [MonitorSwitch; 2], blend: MonitorBlend) -> MonitorMix {
    let [one, two] = switches;
    match (one.monitoring, two.monitoring) {
        (Monitoring::Off, Monitoring::Off) => MonitorMix::OFF,
        (Monitoring::On, Monitoring::Off)
        | (Monitoring::Off, Monitoring::On)
        | (Monitoring::On, Monitoring::On) => MonitorMix {
            playback: blend.playback,
            inputs: [
                one.monitoring.input_gain(blend),
                two.monitoring.input_gain(blend),
            ],
        },
    }
}
