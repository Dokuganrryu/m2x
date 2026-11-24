#![deny(clippy::wildcard_enum_match_arm)]

use crate::knob::{Knob, KnobReading, KnobReadings};
use crate::outputs::{Filler, OutputList};

const KNOB_COUNT: usize = Knob::ALL.len();
const RESULT_VALID: u32 = 1 << 31;
const RESULT_COMMAND_SOURCE_SHIFT: u32 = 24;
const RESULT_COMMAND_SOURCE_MASK: u32 = 0xF;
const RESULT_DATA_MASK: u32 = 0xFFFF;
const GAIN_CALIBRATION_FULL_SCALE: u32 = 0x1_FFFF;
const GAIN_CORRECTION_SHIFT: u32 = 16;
const GAIN_CORRECTION_READY: u32 = 1 << 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResultWord(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GainCalibrationCode(pub u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GainCorrection(pub u32);

impl GainCorrection {
    pub const fn of(code: GainCalibrationCode) -> Self {
        let code = code.0 as u32;
        Self(
            ((code << GAIN_CORRECTION_SHIFT) / (GAIN_CALIBRATION_FULL_SCALE - code))
                | GAIN_CORRECTION_READY,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanResults([Option<KnobReading>; KNOB_COUNT]);

impl ScanResults {
    pub const EMPTY: Self = Self([None; KNOB_COUNT]);

    pub fn with(self, word: ResultWord) -> Self {
        if word.0 & RESULT_VALID == 0 {
            return self;
        }
        let tag = (word.0 >> RESULT_COMMAND_SOURCE_SHIFT) & RESULT_COMMAND_SOURCE_MASK;
        let Some(knob) = Knob::from_result_tag(tag as u8) else {
            return self;
        };
        let Some(index) = Knob::ALL.iter().position(|each| *each == knob) else {
            return self;
        };
        let mut readings = self.0;
        readings[index] = Some(KnobReading((word.0 & RESULT_DATA_MASK) as u16));
        Self(readings)
    }

    pub fn complete(self) -> Option<KnobReadings> {
        let mut readings = [KnobReading(0); KNOB_COUNT];
        for (slot, reading) in readings.iter_mut().zip(self.0) {
            *slot = reading?;
        }
        Some(KnobReadings(readings))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    CalibratingOffset,
    CalibratingGain,
    SettlingGain,
    Converting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Awaiting {
    CalibrationReady,
    GainCalibration,
    Results,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    CalibrationReady,
    GainCalibrated([GainCalibrationCode; 2]),
    ResultsReady(ScanResults),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    RequestGainCalibration,
    WriteGainCorrections([GainCorrection; 2]),
    ConfigureCommands,
    Trigger,
    Readings(KnobReadings),
}

impl Filler for Output {
    const FILLER: Self = Output::Trigger;
}

pub type Outputs = OutputList<Output, 2>;

impl State {
    pub const START: Self = State::CalibratingOffset;

    pub const fn awaiting(self) -> Awaiting {
        match self {
            State::CalibratingOffset | State::SettlingGain => Awaiting::CalibrationReady,
            State::CalibratingGain => Awaiting::GainCalibration,
            State::Converting => Awaiting::Results,
        }
    }
}

pub fn transition(state: State, event: Event) -> (State, Outputs) {
    match (state, event) {
        (State::CalibratingOffset, Event::CalibrationReady) => (
            State::CalibratingGain,
            Outputs::of([Output::RequestGainCalibration]),
        ),
        (State::CalibratingGain, Event::GainCalibrated([first, second])) => (
            State::SettlingGain,
            Outputs::of([Output::WriteGainCorrections([
                GainCorrection::of(first),
                GainCorrection::of(second),
            ])]),
        ),
        (State::SettlingGain, Event::CalibrationReady) => (
            State::Converting,
            Outputs::of([Output::ConfigureCommands, Output::Trigger]),
        ),
        (State::Converting, Event::ResultsReady(results)) => match results.complete() {
            Some(readings) => (
                State::Converting,
                Outputs::of([Output::Readings(readings), Output::Trigger]),
            ),
            None => (State::Converting, Outputs::of([Output::Trigger])),
        },
        (State::CalibratingOffset, Event::GainCalibrated(_) | Event::ResultsReady(_))
        | (State::CalibratingGain, Event::CalibrationReady | Event::ResultsReady(_))
        | (State::SettlingGain, Event::GainCalibrated(_) | Event::ResultsReady(_))
        | (State::Converting, Event::CalibrationReady | Event::GainCalibrated(_)) => {
            (state, Outputs::NONE)
        }
    }
}
