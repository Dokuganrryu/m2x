use dsp::FramesPerTick;

use crate::command::Command;
use crate::dac::{Attenuation, DacWrite, VolumeRegister};
use crate::output_pin::OutputPin;
use crate::phantom::{Milliseconds, PinLevel};

pub const MOST_ATTEMPTS_PER_TRANSFER: Attempt = Attempt(4);
pub const PLACEHOLDER_I2C_ANSWER_TIMEOUT_UNTIL_T23: Milliseconds = Milliseconds(10);
pub const PLACEHOLDER_STEP_CONFIRMATION_TIMEOUT_UNTIL_T23: Milliseconds = Milliseconds(50);

pub(crate) const STOP_TRANSPORT: [Command; 4] = [
    Command::StopTickTimer,
    Command::StopDma,
    Command::DrivePin(OutputPin::TransportActive, PinLevel::Low),
    Command::StopI2s,
];

pub(crate) const fn start_transport(frames: FramesPerTick) -> [Command; 4] {
    [
        Command::StartDma,
        Command::StartI2s(frames),
        Command::StartTickTimer(frames),
        Command::DrivePin(OutputPin::TransportActive, PinLevel::High),
    ]
}

pub(crate) const fn fully_attenuate(register: VolumeRegister) -> Command {
    Command::WriteDac(DacWrite::Volume(register, Attenuation::FULL))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptStep<T> {
    Emit(T),
    Hold(Milliseconds),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attempt(u8);

impl Attempt {
    pub const FIRST: Self = Self(1);

    pub const fn count(self) -> u8 {
        self.0
    }

    pub(crate) const fn next(self) -> Option<Self> {
        if self.0 < MOST_ATTEMPTS_PER_TRANSFER.0 {
            Some(Self(self.0 + 1))
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Countdown {
    Left(Milliseconds),
    Over,
}

pub const fn later(waited: Milliseconds, elapsed: Milliseconds) -> Milliseconds {
    Milliseconds(waited.0.saturating_add(elapsed.0))
}

pub const fn reached(waited: Milliseconds, limit: Milliseconds) -> bool {
    waited.0 > limit.0
}

pub const fn count_down(remaining: Milliseconds, elapsed: Milliseconds) -> Countdown {
    if reached(elapsed, remaining) {
        Countdown::Over
    } else {
        Countdown::Left(Milliseconds(remaining.0 - elapsed.0))
    }
}
