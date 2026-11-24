#![deny(
    clippy::wildcard_enum_match_arm,
    clippy::struct_excessive_bools,
    clippy::fn_params_excessive_bools
)]

use dsp::{FramesPerTick, SampleRate};

use crate::adc::AdcKind;
use crate::command::Command;
use crate::dac::VolumeRegister;
use crate::fault::{FaultCode, I2cPeer};
use crate::output_pin::OutputPin;
use crate::outputs::{Filler, OutputList};
use crate::phantom::{Milliseconds, PinLevel};
use crate::pll::audio_pll_stabilisation;
use crate::steps::{
    Attempt, PLACEHOLDER_I2C_ANSWER_TIMEOUT_UNTIL_T23,
    PLACEHOLDER_STEP_CONFIRMATION_TIMEOUT_UNTIL_T23, STOP_TRANSPORT, fully_attenuate, later,
    reached, start_transport,
};

pub const COALESCING_WINDOW: Milliseconds = Milliseconds(20);
pub const PLAYBACK_FADE: Ticks = Ticks(32);
pub const FACTORY_ADC_SPEED_BITS_HOLD: Milliseconds = Milliseconds(5);
pub const DAC_CLOCK_SETTLE: Milliseconds = Milliseconds(250);
pub const MOST_OUTPUTS_PER_EVENT: usize = 4;

const FIRST_ATTENUATED_REGISTER: VolumeRegister = VolumeRegister::MainA;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ticks(u8);

impl Ticks {
    pub const fn count(self) -> u8 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockValidity {
    Valid,
    Invalid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockReport {
    pub rate: SampleRate,
    pub validity: ClockValidity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Board(Command),
    SendAnalogWordAtSpeed(FramesPerTick),
    HoldVolumeAtFull,
    RampVolumeFromFull,
    ResetStreams(SampleRate),
    SwapCoefficients(SampleRate),
    ClearDspState,
    ClockValidity(ClockValidity),
    RaiseFault(FaultCode),
}

pub type Outputs = OutputList<Output, MOST_OUTPUTS_PER_EVENT>;

impl Filler for Output {
    const FILLER: Self = Output::ClearDspState;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputLevel {
    Open,
    FadingOut { elapsed: Ticks },
    Silent,
    FadingIn { elapsed: Ticks },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    SetCur(SampleRate),
    Ticked,
    Elapsed(Milliseconds),
    I2cAcked,
    I2cNacked,
    StepDone,
    PllLockTimedOut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Queued {
    Nothing,
    Rate(SampleRate),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    from: SampleRate,
    to: SampleRate,
    adc: AdcKind,
    queued: Queued,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DacWriteAttempt {
    register: VolumeRegister,
    attempt: Attempt,
    waited: Milliseconds,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Steady {
        rate: SampleRate,
        adc: AdcKind,
    },
    Coalescing {
        from: SampleRate,
        requested: SampleRate,
        adc: AdcKind,
        quiet: Milliseconds,
    },
    FadingOut {
        change: Change,
        elapsed: Ticks,
    },
    Attenuating {
        change: Change,
        attempt: DacWriteAttempt,
    },
    Stopping {
        change: Change,
        waited: Milliseconds,
    },
    SettingSpeedBits {
        change: Change,
        waited: Milliseconds,
    },
    HoldingSpeedBits {
        change: Change,
        held: Milliseconds,
    },
    EnablingAdc {
        change: Change,
        waited: Milliseconds,
    },
    SwitchingFamily {
        change: Change,
        waited: Milliseconds,
    },
    StabilisingPll {
        change: Change,
        waited: Milliseconds,
    },
    Reconfiguring {
        change: Change,
        waited: Milliseconds,
    },
    Restarting {
        change: Change,
        waited: Milliseconds,
    },
    Settling {
        change: Change,
        elapsed: Milliseconds,
    },
    FadingIn {
        change: Change,
        elapsed: Ticks,
    },
    Failed {
        fault: FaultCode,
        reported: SampleRate,
    },
}

impl State {
    pub const fn reported_rate(self) -> SampleRate {
        match self {
            State::Steady { rate, .. } => rate,
            State::Coalescing { requested, .. } => requested,
            State::FadingOut { change, .. }
            | State::Attenuating { change, .. }
            | State::Stopping { change, .. }
            | State::SettingSpeedBits { change, .. }
            | State::HoldingSpeedBits { change, .. }
            | State::EnablingAdc { change, .. }
            | State::SwitchingFamily { change, .. }
            | State::StabilisingPll { change, .. }
            | State::Reconfiguring { change, .. }
            | State::Restarting { change, .. }
            | State::Settling { change, .. }
            | State::FadingIn { change, .. } => change.reported_rate(),
            State::Failed { reported, .. } => reported,
        }
    }

    pub const fn report(self) -> ClockReport {
        ClockReport {
            rate: self.reported_rate(),
            validity: self.clock_validity(),
        }
    }

    pub const fn clock_validity(self) -> ClockValidity {
        match self {
            State::Steady { .. } => ClockValidity::Valid,
            State::Coalescing { .. }
            | State::FadingOut { .. }
            | State::Attenuating { .. }
            | State::Stopping { .. }
            | State::SettingSpeedBits { .. }
            | State::HoldingSpeedBits { .. }
            | State::EnablingAdc { .. }
            | State::SwitchingFamily { .. }
            | State::StabilisingPll { .. }
            | State::Reconfiguring { .. }
            | State::Restarting { .. }
            | State::Settling { .. }
            | State::FadingIn { .. }
            | State::Failed { .. } => ClockValidity::Invalid,
        }
    }

    pub const fn output_level(self) -> OutputLevel {
        match self {
            State::Steady { .. } | State::Coalescing { .. } => OutputLevel::Open,
            State::FadingOut { elapsed, .. } => OutputLevel::FadingOut { elapsed },
            State::Attenuating { .. }
            | State::Stopping { .. }
            | State::SettingSpeedBits { .. }
            | State::HoldingSpeedBits { .. }
            | State::EnablingAdc { .. }
            | State::SwitchingFamily { .. }
            | State::StabilisingPll { .. }
            | State::Reconfiguring { .. }
            | State::Restarting { .. }
            | State::Settling { .. }
            | State::Failed { .. } => OutputLevel::Silent,
            State::FadingIn { elapsed, .. } => OutputLevel::FadingIn { elapsed },
        }
    }
}

impl Change {
    const fn reported_rate(self) -> SampleRate {
        match self.queued {
            Queued::Nothing => self.to,
            Queued::Rate(rate) => rate,
        }
    }

    const fn queue(self, rate: SampleRate) -> Self {
        Self {
            queued: Queued::Rate(rate),
            ..self
        }
    }

    fn family_changes(self) -> bool {
        self.from.family() != self.to.family()
    }

    const fn speed(self) -> FramesPerTick {
        self.to.frames_per_tick()
    }
}

const fn first_attempt(register: VolumeRegister) -> DacWriteAttempt {
    DacWriteAttempt {
        register,
        attempt: Attempt::FIRST,
        waited: Milliseconds(0),
    }
}

const fn attenuate(register: VolumeRegister) -> Output {
    Output::Board(fully_attenuate(register))
}

const fn attenuated_register_after(register: VolumeRegister) -> Option<VolumeRegister> {
    match register {
        VolumeRegister::MainA => Some(VolumeRegister::MainB),
        VolumeRegister::MainB => Some(VolumeRegister::HeadphoneA),
        VolumeRegister::HeadphoneA => Some(VolumeRegister::HeadphoneB),
        VolumeRegister::HeadphoneB
        | VolumeRegister::FactoryZeroA
        | VolumeRegister::FactoryZeroB
        | VolumeRegister::FactoryHeldA
        | VolumeRegister::FactoryHeldB => None,
    }
}

const fn next_tick(elapsed: Ticks) -> Ticks {
    Ticks(elapsed.0.saturating_add(1))
}

pub fn transition(state: State, event: Event) -> (State, Outputs) {
    match state {
        State::Steady { rate, adc } => match event {
            Event::SetCur(requested) if requested != rate => (
                State::Coalescing {
                    from: rate,
                    requested,
                    adc,
                    quiet: Milliseconds(0),
                },
                Outputs::of([Output::ClockValidity(ClockValidity::Invalid)]),
            ),
            Event::SetCur(_)
            | Event::Ticked
            | Event::Elapsed(_)
            | Event::I2cAcked
            | Event::I2cNacked
            | Event::PllLockTimedOut
            | Event::StepDone => (state, Outputs::NONE),
        },
        State::Coalescing {
            from,
            requested,
            adc,
            quiet,
        } => match event {
            Event::SetCur(requested) => (
                State::Coalescing {
                    from,
                    requested,
                    adc,
                    quiet: Milliseconds(0),
                },
                Outputs::NONE,
            ),
            Event::Elapsed(more) => {
                let quiet = later(quiet, more);
                if reached(quiet, COALESCING_WINDOW) {
                    begin(from, requested, adc)
                } else {
                    (
                        State::Coalescing {
                            from,
                            requested,
                            adc,
                            quiet,
                        },
                        Outputs::NONE,
                    )
                }
            }
            Event::Ticked
            | Event::I2cAcked
            | Event::I2cNacked
            | Event::PllLockTimedOut
            | Event::StepDone => (state, Outputs::NONE),
        },
        State::FadingOut { change, elapsed } => match event {
            Event::SetCur(rate) => (
                State::FadingOut {
                    change: change.queue(rate),
                    elapsed,
                },
                Outputs::NONE,
            ),
            Event::Ticked => {
                let elapsed = next_tick(elapsed);
                if elapsed >= PLAYBACK_FADE {
                    (
                        State::Attenuating {
                            change,
                            attempt: first_attempt(FIRST_ATTENUATED_REGISTER),
                        },
                        Outputs::of([
                            Output::HoldVolumeAtFull,
                            attenuate(FIRST_ATTENUATED_REGISTER),
                        ]),
                    )
                } else {
                    (State::FadingOut { change, elapsed }, Outputs::NONE)
                }
            }
            Event::Elapsed(_)
            | Event::I2cAcked
            | Event::I2cNacked
            | Event::PllLockTimedOut
            | Event::StepDone => (state, Outputs::NONE),
        },
        State::Attenuating { change, attempt } => match event {
            Event::SetCur(rate) => (
                State::Attenuating {
                    change: change.queue(rate),
                    attempt,
                },
                Outputs::NONE,
            ),
            Event::I2cAcked => match attenuated_register_after(attempt.register) {
                Some(register) => (
                    State::Attenuating {
                        change,
                        attempt: first_attempt(register),
                    },
                    Outputs::of([attenuate(register)]),
                ),
                None => (
                    State::Stopping {
                        change,
                        waited: Milliseconds(0),
                    },
                    Outputs::of([
                        Output::Board(STOP_TRANSPORT[0]),
                        Output::Board(STOP_TRANSPORT[1]),
                        Output::Board(STOP_TRANSPORT[2]),
                        Output::Board(STOP_TRANSPORT[3]),
                    ]),
                ),
            },
            Event::I2cNacked => retry(change, attempt),
            Event::Elapsed(more) => {
                let waited = later(attempt.waited, more);
                if reached(waited, PLACEHOLDER_I2C_ANSWER_TIMEOUT_UNTIL_T23) {
                    retry(change, attempt)
                } else {
                    (
                        State::Attenuating {
                            change,
                            attempt: DacWriteAttempt { waited, ..attempt },
                        },
                        Outputs::NONE,
                    )
                }
            }
            Event::Ticked | Event::PllLockTimedOut | Event::StepDone => (state, Outputs::NONE),
        },
        State::Stopping { change, waited } => match event {
            Event::SetCur(rate) => (
                State::Stopping {
                    change: change.queue(rate),
                    waited,
                },
                Outputs::NONE,
            ),
            Event::Elapsed(more) => await_board(change, later(waited, more), |change, waited| {
                State::Stopping { change, waited }
            }),
            Event::StepDone => after_stop(change),
            Event::Ticked | Event::I2cAcked | Event::I2cNacked | Event::PllLockTimedOut => {
                (state, Outputs::NONE)
            }
        },
        State::SettingSpeedBits { change, waited } => match event {
            Event::SetCur(rate) => (
                State::SettingSpeedBits {
                    change: change.queue(rate),
                    waited,
                },
                Outputs::NONE,
            ),
            Event::Elapsed(more) => await_board(change, later(waited, more), |change, waited| {
                State::SettingSpeedBits { change, waited }
            }),
            Event::StepDone => (
                State::HoldingSpeedBits {
                    change,
                    held: Milliseconds(0),
                },
                Outputs::NONE,
            ),
            Event::Ticked | Event::I2cAcked | Event::I2cNacked | Event::PllLockTimedOut => {
                (state, Outputs::NONE)
            }
        },
        State::HoldingSpeedBits { change, held } => match event {
            Event::SetCur(rate) => (
                State::HoldingSpeedBits {
                    change: change.queue(rate),
                    held,
                },
                Outputs::NONE,
            ),
            Event::Elapsed(more) => {
                let held = later(held, more);
                if reached(held, FACTORY_ADC_SPEED_BITS_HOLD) {
                    (
                        State::EnablingAdc {
                            change,
                            waited: Milliseconds(0),
                        },
                        Outputs::of([Output::Board(Command::DrivePin(
                            OutputPin::AdcEnable,
                            PinLevel::High,
                        ))]),
                    )
                } else {
                    (State::HoldingSpeedBits { change, held }, Outputs::NONE)
                }
            }
            Event::Ticked
            | Event::I2cAcked
            | Event::I2cNacked
            | Event::PllLockTimedOut
            | Event::StepDone => (state, Outputs::NONE),
        },
        State::EnablingAdc { change, waited } => match event {
            Event::SetCur(rate) => (
                State::EnablingAdc {
                    change: change.queue(rate),
                    waited,
                },
                Outputs::NONE,
            ),
            Event::Elapsed(more) => await_board(change, later(waited, more), |change, waited| {
                State::EnablingAdc { change, waited }
            }),
            Event::StepDone => after_speed_bits(change),
            Event::Ticked | Event::I2cAcked | Event::I2cNacked | Event::PllLockTimedOut => {
                (state, Outputs::NONE)
            }
        },
        State::SwitchingFamily { change, waited } => match event {
            Event::SetCur(rate) => (
                State::SwitchingFamily {
                    change: change.queue(rate),
                    waited,
                },
                Outputs::NONE,
            ),
            Event::Elapsed(more) => await_board(change, later(waited, more), |change, waited| {
                State::SwitchingFamily { change, waited }
            }),
            Event::StepDone => (
                State::StabilisingPll {
                    change,
                    waited: Milliseconds(0),
                },
                Outputs::NONE,
            ),
            Event::PllLockTimedOut => fail(change, FaultCode::AudioPllUnlocked),
            Event::Ticked | Event::I2cAcked | Event::I2cNacked => (state, Outputs::NONE),
        },
        State::StabilisingPll { change, waited } => match event {
            Event::SetCur(rate) => (
                State::StabilisingPll {
                    change: change.queue(rate),
                    waited,
                },
                Outputs::NONE,
            ),
            Event::Elapsed(more) => {
                let waited = later(waited, more);
                if reached(waited, audio_pll_stabilisation(change.to.family())) {
                    reconfigure(change)
                } else {
                    (State::StabilisingPll { change, waited }, Outputs::NONE)
                }
            }
            Event::PllLockTimedOut => fail(change, FaultCode::AudioPllUnlocked),
            Event::Ticked | Event::I2cAcked | Event::I2cNacked | Event::StepDone => {
                (state, Outputs::NONE)
            }
        },
        State::Reconfiguring { change, waited } => match event {
            Event::SetCur(rate) => (
                State::Reconfiguring {
                    change: change.queue(rate),
                    waited,
                },
                Outputs::NONE,
            ),
            Event::Elapsed(more) => await_board(change, later(waited, more), |change, waited| {
                State::Reconfiguring { change, waited }
            }),
            Event::StepDone => {
                let [dma, i2s, tick_timer, transport_pin] = start_transport(change.speed());
                (
                    State::Restarting {
                        change,
                        waited: Milliseconds(0),
                    },
                    Outputs::of([
                        Output::Board(dma),
                        Output::Board(i2s),
                        Output::Board(tick_timer),
                        Output::Board(transport_pin),
                    ]),
                )
            }
            Event::PllLockTimedOut => fail(change, FaultCode::AudioPllUnlocked),
            Event::Ticked | Event::I2cAcked | Event::I2cNacked => (state, Outputs::NONE),
        },
        State::Restarting { change, waited } => match event {
            Event::SetCur(rate) => (
                State::Restarting {
                    change: change.queue(rate),
                    waited,
                },
                Outputs::NONE,
            ),
            Event::Elapsed(more) => await_board(change, later(waited, more), |change, waited| {
                State::Restarting { change, waited }
            }),
            Event::StepDone => (
                State::Settling {
                    change,
                    elapsed: Milliseconds(0),
                },
                Outputs::NONE,
            ),
            Event::PllLockTimedOut => fail(change, FaultCode::AudioPllUnlocked),
            Event::Ticked | Event::I2cAcked | Event::I2cNacked => (state, Outputs::NONE),
        },
        State::Settling { change, elapsed } => match event {
            Event::SetCur(rate) => (
                State::Settling {
                    change: change.queue(rate),
                    elapsed,
                },
                Outputs::NONE,
            ),
            Event::Elapsed(more) => {
                let elapsed = later(elapsed, more);
                if reached(elapsed, DAC_CLOCK_SETTLE) {
                    (
                        State::FadingIn {
                            change,
                            elapsed: Ticks(0),
                        },
                        Outputs::of([Output::RampVolumeFromFull]),
                    )
                } else {
                    (State::Settling { change, elapsed }, Outputs::NONE)
                }
            }
            Event::PllLockTimedOut => fail(change, FaultCode::AudioPllUnlocked),
            Event::Ticked | Event::I2cAcked | Event::I2cNacked | Event::StepDone => {
                (state, Outputs::NONE)
            }
        },
        State::FadingIn { change, elapsed } => match event {
            Event::SetCur(rate) => (
                State::FadingIn {
                    change: change.queue(rate),
                    elapsed,
                },
                Outputs::NONE,
            ),
            Event::Ticked => {
                let elapsed = next_tick(elapsed);
                if elapsed >= PLAYBACK_FADE {
                    finish(change)
                } else {
                    (State::FadingIn { change, elapsed }, Outputs::NONE)
                }
            }
            Event::Elapsed(_)
            | Event::I2cAcked
            | Event::I2cNacked
            | Event::PllLockTimedOut
            | Event::StepDone => (state, Outputs::NONE),
        },
        State::Failed { fault, .. } => match event {
            Event::SetCur(reported) => (State::Failed { fault, reported }, Outputs::NONE),
            Event::Ticked
            | Event::Elapsed(_)
            | Event::I2cAcked
            | Event::I2cNacked
            | Event::PllLockTimedOut
            | Event::StepDone => (state, Outputs::NONE),
        },
    }
}

fn begin(from: SampleRate, to: SampleRate, adc: AdcKind) -> (State, Outputs) {
    if from == to {
        (
            State::Steady { rate: from, adc },
            Outputs::of([Output::ClockValidity(ClockValidity::Valid)]),
        )
    } else {
        (
            State::FadingOut {
                change: Change {
                    from,
                    to,
                    adc,
                    queued: Queued::Nothing,
                },
                elapsed: Ticks(0),
            },
            Outputs::NONE,
        )
    }
}

fn after_stop(change: Change) -> (State, Outputs) {
    match change.adc {
        AdcKind::Unanswered => (
            State::SettingSpeedBits {
                change,
                waited: Milliseconds(0),
            },
            Outputs::of([
                Output::Board(Command::DrivePin(OutputPin::AdcEnable, PinLevel::Low)),
                Output::SendAnalogWordAtSpeed(change.speed()),
            ]),
        ),
        AdcKind::Answered(_) => after_speed_bits(change),
    }
}

fn after_speed_bits(change: Change) -> (State, Outputs) {
    if change.family_changes() {
        (
            State::SwitchingFamily {
                change,
                waited: Milliseconds(0),
            },
            Outputs::of([Output::Board(Command::SwitchPll0Family(change.to.family()))]),
        )
    } else {
        reconfigure(change)
    }
}

const fn reconfigure(change: Change) -> (State, Outputs) {
    (
        State::Reconfiguring {
            change,
            waited: Milliseconds(0),
        },
        Outputs::of([
            Output::ResetStreams(change.to),
            Output::SwapCoefficients(change.to),
            Output::ClearDspState,
        ]),
    )
}

fn finish(change: Change) -> (State, Outputs) {
    match change.queued {
        Queued::Rate(requested) if requested != change.to => (
            State::Coalescing {
                from: change.to,
                requested,
                adc: change.adc,
                quiet: Milliseconds(0),
            },
            Outputs::NONE,
        ),
        Queued::Rate(_) | Queued::Nothing => (
            State::Steady {
                rate: change.to,
                adc: change.adc,
            },
            Outputs::of([Output::ClockValidity(ClockValidity::Valid)]),
        ),
    }
}

fn retry(change: Change, attempt: DacWriteAttempt) -> (State, Outputs) {
    match attempt.attempt.next() {
        Some(next) => (
            State::Attenuating {
                change,
                attempt: DacWriteAttempt {
                    register: attempt.register,
                    attempt: next,
                    waited: Milliseconds(0),
                },
            },
            Outputs::of([attenuate(attempt.register)]),
        ),
        None => fail(change, FaultCode::Unanswered(I2cPeer::Dac)),
    }
}

fn await_board(
    change: Change,
    waited: Milliseconds,
    still: fn(Change, Milliseconds) -> State,
) -> (State, Outputs) {
    if reached(waited, PLACEHOLDER_STEP_CONFIRMATION_TIMEOUT_UNTIL_T23) {
        fail(change, FaultCode::BoardStalled)
    } else {
        (still(change, waited), Outputs::NONE)
    }
}

const fn fail(change: Change, fault: FaultCode) -> (State, Outputs) {
    (
        State::Failed {
            fault,
            reported: change.reported_rate(),
        },
        Outputs::of([Output::RaiseFault(fault)]),
    )
}
