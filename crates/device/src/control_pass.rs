use dsp::{Gain, MonitorMix};

use crate::analog_settings::AnalogSettings;
use crate::boot::VolumeTargets;
use crate::command::Command;
use crate::dac::{Attenuation, DacWrite, VolumeRegister};
use crate::expander::ExpanderInputs;
use crate::input::Input;
use crate::knob::{Knob, KnobReadings, Knobs};
use crate::knob_law::{monitor_blend, preamp_gain, volume_targets};
pub use crate::monitor::Monitoring;
use crate::monitor::{MonitorSwitch, monitor_mix};
use crate::outputs::{Filler, OutputList};
use crate::phantom::{Jack, Milliseconds, PhantomGrant};
use crate::volume_slew::VolumeSlew;

pub const CONTROL_PERIOD: Milliseconds = Milliseconds(5);
pub const MOST_COMMANDS_PER_PASS: usize = 5;

pub type PassCommands = OutputList<Command, MOST_COMMANDS_PER_PASS>;

impl Filler for Command {
    const FILLER: Self = Command::StopI2s;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanelReadings {
    pub knobs: KnobReadings,
    pub jacks: [Jack; 2],
    pub buttons: ExpanderInputs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickGains {
    pub capture: [Gain; 2],
    pub monitor: MonitorMix,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PassOutputs {
    pub commands: PassCommands,
    pub analog: AnalogSettings,
    pub tick: TickGains,
    pub long_presses: [bool; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlPass {
    knobs: Knobs,
    volume: VolumeSlew,
    monitors: [MonitorSwitch; 2],
}

impl PassOutputs {
    pub fn holding_volume_at_full(self) -> Self {
        let mut commands = PassCommands::NONE;
        for command in self.commands.as_slice() {
            match command {
                Command::WriteDac(DacWrite::Volume(..)) => {}
                other => commands.push(*other),
            }
        }
        Self { commands, ..self }
    }
}

impl ControlPass {
    pub const fn after_boot(knobs: Knobs, ramp_end: VolumeTargets) -> Self {
        Self {
            knobs,
            volume: VolumeSlew::from_ramp_end(ramp_end),
            monitors: [MonitorSwitch::AT_BOOT; 2],
        }
    }

    pub const fn with_volume_from_full_attenuation(self) -> Self {
        Self {
            volume: VolumeSlew::from_ramp_end(VolumeTargets {
                main: Attenuation::FULL,
                headphone: Attenuation::FULL,
            }),
            ..self
        }
    }

    pub const fn monitoring(&self) -> [Monitoring; 2] {
        let [one, two] = self.monitors;
        [one.monitoring(), two.monitoring()]
    }

    pub const fn knobs(&self) -> Knobs {
        self.knobs
    }

    pub fn run(
        self,
        readings: PanelReadings,
        analog: AnalogSettings,
        grant: PhantomGrant,
    ) -> (Self, PassOutputs) {
        let knobs = self.knobs.after_scan(readings.knobs);
        let volume = self.volume.toward(volume_targets(knobs));
        let monitors = Input::BOTH.map(|input| {
            self.monitors[slot(input)].after(readings.buttons.monitor_button(input), CONTROL_PERIOD)
        });
        let preamps = Input::BOTH.map(|input| {
            preamp_gain(
                knobs.position(Knob::preamp(input)),
                readings.jacks[slot(input)],
            )
        });
        let [one, two] = preamps;
        let next_analog = AnalogSettings {
            preamp_one: one.code,
            preamp_two: two.code,
            ..analog
        };

        let mut commands = PassCommands::NONE;
        push_changed(
            &mut commands,
            self.volume.levels().main,
            volume.levels().main,
            MAIN_REGISTERS,
        );
        push_changed(
            &mut commands,
            self.volume.levels().headphone,
            volume.levels().headphone,
            HEADPHONE_REGISTERS,
        );
        if next_analog != analog {
            commands.push(Command::SendAnalogWord(next_analog.word(grant)));
        }

        let tick = TickGains {
            capture: preamps.map(|gain| gain.digital),
            monitor: monitor_mix(monitors, monitor_blend(knobs.position(Knob::MonitorMix))),
        };
        let pass = Self {
            knobs,
            volume,
            monitors,
        };
        (
            pass,
            PassOutputs {
                commands,
                analog: next_analog,
                tick,
                long_presses: Input::BOTH.map(|input| {
                    self.monitors[slot(input)].became_a_long_press(monitors[slot(input)])
                }),
            },
        )
    }
}

const MAIN_REGISTERS: [VolumeRegister; 2] = [VolumeRegister::MainA, VolumeRegister::MainB];
const HEADPHONE_REGISTERS: [VolumeRegister; 2] =
    [VolumeRegister::HeadphoneA, VolumeRegister::HeadphoneB];

fn push_changed(
    commands: &mut PassCommands,
    before: Attenuation,
    after: Attenuation,
    registers: [VolumeRegister; 2],
) {
    if before != after {
        for register in registers {
            commands.push(Command::WriteDac(DacWrite::Volume(register, after)));
        }
    }
}

const fn slot(input: Input) -> usize {
    match input {
        Input::One => 0,
        Input::Two => 1,
    }
}
