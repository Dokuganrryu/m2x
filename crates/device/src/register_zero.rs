use core::hint::black_box;

use crate::boot::BootRecord;
use crate::breadcrumb::At;
use crate::command::Command;
use crate::dac::VolumeRegister;
use crate::fault::{FaultCode, I2cPeer};
use crate::input::Input;
use crate::outputs::{Filler, OutputList};
use crate::phantom::{Milliseconds, PhantomGrant, PhantomPower};
use crate::safe_init::SAFE_DAC_INIT;
use crate::steps::{
    Attempt, Countdown, PLACEHOLDER_I2C_ANSWER_TIMEOUT_UNTIL_T23, ScriptStep, count_down,
    fully_attenuate, later, reached,
};
use crate::supervisor;

pub const ARMING_STEP_TIMEOUT: Milliseconds = Milliseconds(60_000);
pub const NONCE_DIGITS: usize = 6;
pub const MOST_OUTPUTS_PER_EVENT: usize = 1 + VolumeRegister::ALL.len();

const NONCE_SPAN: u32 = 1_000_000;
const DECIMAL_BASE: u32 = 10;
const REGISTER_ZERO: u8 = 0;
const REGISTER_ZERO_FACTORY_VALUE: u8 = 0x00;
const RESTORE_STEPS: usize = VolumeRegister::ALL.len() + SAFE_DAC_INIT.len();
const RESTORE_SCRIPT: [ScriptStep<Command>; RESTORE_STEPS] = restore_script();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfirmationNonce(u32);

impl ConfirmationNonce {
    pub const fn from_boot_entropy(entropy: u32) -> Self {
        Self(entropy % NONCE_SPAN)
    }

    pub const fn new(value: u32) -> Option<Self> {
        if value < NONCE_SPAN {
            Some(Self(value))
        } else {
            None
        }
    }

    pub const fn digits(self) -> [u8; NONCE_DIGITS] {
        let mut digits = [b'0'; NONCE_DIGITS];
        let mut rest = self.0;
        let mut index = NONCE_DIGITS;
        while index > 0 {
            index -= 1;
            digits[index] = b'0' + (rest % DECIMAL_BASE) as u8;
            rest /= DECIMAL_BASE;
        }
        digits
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Sealed;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegisterZeroWrite(Sealed);

#[inline(never)]
pub fn register_zero_factory_bus_bytes(write: RegisterZeroWrite) -> [u8; 2] {
    let RegisterZeroWrite(Sealed) = write;
    black_box([REGISTER_ZERO, REGISTER_ZERO_FACTORY_VALUE])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolumeEvidence {
    FullyAttenuatedAndAckedThisBoot,
    Unconfirmed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransmitBuffers {
    Zeroed,
    Live,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoardFact {
    VolumeFullyAttenuated,
    TransmitSilent,
    PhantomRequestsClear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoardRecord {
    pub volume: VolumeEvidence,
    pub transmit: TransmitBuffers,
    pub phantom: PhantomGrant,
}

impl BoardRecord {
    const fn first_unmet(self) -> Option<BoardFact> {
        match (
            self.volume,
            self.transmit,
            self.phantom.power(Input::One),
            self.phantom.power(Input::Two),
        ) {
            (VolumeEvidence::Unconfirmed, _, _, _) => Some(BoardFact::VolumeFullyAttenuated),
            (VolumeEvidence::FullyAttenuatedAndAckedThisBoot, TransmitBuffers::Live, _, _) => {
                Some(BoardFact::TransmitSilent)
            }
            (
                VolumeEvidence::FullyAttenuatedAndAckedThisBoot,
                TransmitBuffers::Zeroed,
                PhantomPower::On,
                _,
            )
            | (
                VolumeEvidence::FullyAttenuatedAndAckedThisBoot,
                TransmitBuffers::Zeroed,
                PhantomPower::Off,
                PhantomPower::On,
            ) => Some(BoardFact::PhantomRequestsClear),
            (
                VolumeEvidence::FullyAttenuatedAndAckedThisBoot,
                TransmitBuffers::Zeroed,
                PhantomPower::Off,
                PhantomPower::Off,
            ) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperatorStep {
    ProtocolSigned,
    SpareUnit,
    OutputsUnplugged,
    RequestReplay,
    Confirm(ConfirmationNonce),
    Abort,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    ProtocolSigned,
    SpareConfirmed,
    OutputsUnplugged,
    ReplayRequested(BoardRecord),
    NonceTyped(ConfirmationNonce, BoardRecord),
    Aborted,
    PortClosed,
    PreconditionLost(BoardFact),
    FaultRaised(FaultCode),
    I2cAcked,
    I2cNacked,
    StatusRead(u8),
    Elapsed(Milliseconds),
}

impl Event {
    pub const fn from_operator(step: OperatorStep, record: BoardRecord) -> Self {
        match step {
            OperatorStep::ProtocolSigned => Event::ProtocolSigned,
            OperatorStep::SpareUnit => Event::SpareConfirmed,
            OperatorStep::OutputsUnplugged => Event::OutputsUnplugged,
            OperatorStep::RequestReplay => Event::ReplayRequested(record),
            OperatorStep::Confirm(nonce) => Event::NonceTyped(nonce, record),
            OperatorStep::Abort => Event::Aborted,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisarmCause {
    OutOfOrder,
    TimedOut,
    Aborted,
    PortClosed,
    FaultRaised,
    WrongNonce,
    Unmet(BoardFact),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    ShowNonce(ConfirmationNonce),
    Disarmed(DisarmCause),
    WriteRegisterZero(RegisterZeroWrite),
    Command(Command),
    Log(BootRecord),
    Supervisor(supervisor::Event),
}

pub type Outputs = OutputList<Output, MOST_OUTPUTS_PER_EVENT>;

impl Filler for Output {
    const FILLER: Self = Output::Disarmed(DisarmCause::OutOfOrder);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Checklist {
    Signed,
    OnSpare,
    Unplugged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Arming {
    nonce: ConfirmationNonce,
    waited: Milliseconds,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReplayStep {
    RegisterZero,
    Restore(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answer {
    Ack,
    StatusByte,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReplayWait {
    Answer {
        expected: Answer,
        attempt: Attempt,
        waited: Milliseconds,
    },
    Hold {
        remaining: Milliseconds,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Replay {
    step: ReplayStep,
    wait: ReplayWait,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Locked(ConfirmationNonce),
    Arming(Checklist, Arming),
    NonceIssued(Arming),
    Replaying(Replay),
    HeldAttenuated,
    Failed(FaultCode),
}

impl State {
    pub const fn locked(nonce: ConfirmationNonce) -> Self {
        State::Locked(nonce)
    }

    pub const fn forbids_volume_changes(self) -> bool {
        match self {
            State::Locked(_) | State::Arming(..) | State::NonceIssued(_) => false,
            State::Replaying(_) | State::HeldAttenuated | State::Failed(_) => true,
        }
    }
}

enum RestoreStep {
    Emit,
    Transfer(Answer),
}

const fn restore_script() -> [ScriptStep<Command>; RESTORE_STEPS] {
    let mut script = [ScriptStep::Emit(Command::StopI2s); RESTORE_STEPS];
    let mut index = 0;
    while index < VolumeRegister::ALL.len() {
        script[index] = ScriptStep::Emit(fully_attenuate(VolumeRegister::ALL[index]));
        index += 1;
    }
    while index < RESTORE_STEPS {
        script[index] = SAFE_DAC_INIT[index - VolumeRegister::ALL.len()];
        index += 1;
    }
    script
}

const fn restore_step(command: Command) -> RestoreStep {
    match command {
        Command::WriteDac(_) => RestoreStep::Transfer(Answer::Ack),
        Command::ReadDac(_) => RestoreStep::Transfer(Answer::StatusByte),
        Command::DrivePin(..)
        | Command::SendAnalogWord(_)
        | Command::ProbeAdc(_)
        | Command::WriteAdc(_)
        | Command::SwitchPll0Family(_)
        | Command::StartI2s(_)
        | Command::StopI2s
        | Command::StartDma
        | Command::StopDma
        | Command::StartTickTimer(_)
        | Command::StopTickTimer
        | Command::MuteInput(_)
        | Command::UnmuteInput(_)
        | Command::AttachUsb
        | Command::DisableInterrupts
        | Command::DetachUsb
        | Command::FeedWatchdog
        | Command::ArmWatchdog
        | Command::FenceFlashController
        | Command::ConfigureAnalogWordBus
        | Command::StartClockInput
        | Command::ReadClockInput
        | Command::StartCorePll
        | Command::SelectCorePllAsMainClock
        | Command::EnableMasterClockOutput
        | Command::ConfigureConverterBus
        | Command::WriteExpanderDirection
        | Command::ReadExpanderInputs
        | Command::WriteExpanderOutputs(_)
        | Command::StartKnobScan
        | Command::ZeroTransmitBuffers
        | Command::StopTransmitDma
        | Command::LatchPlaybackSilence
        | Command::RecordFault(..) => RestoreStep::Emit,
    }
}

pub fn transition(state: State, event: Event) -> (State, Outputs) {
    let mut outputs = Outputs::NONE;
    let next = match state {
        State::Locked(nonce) => locked(nonce, event, &mut outputs),
        State::Arming(checklist, arming) => armed(checklist, arming, event, &mut outputs),
        State::NonceIssued(arming) => nonce_issued(arming, event, &mut outputs),
        State::Replaying(replay) => replaying(replay, event, &mut outputs),
        State::HeldAttenuated => State::HeldAttenuated,
        State::Failed(code) => State::Failed(code),
    };
    (next, outputs)
}

const fn fresh(nonce: ConfirmationNonce) -> Arming {
    Arming {
        nonce,
        waited: Milliseconds(0),
    }
}

fn locked(nonce: ConfirmationNonce, event: Event, outputs: &mut Outputs) -> State {
    match event {
        Event::ProtocolSigned => State::Arming(Checklist::Signed, fresh(nonce)),
        Event::SpareConfirmed
        | Event::OutputsUnplugged
        | Event::ReplayRequested(_)
        | Event::NonceTyped(..) => {
            outputs.push(Output::Disarmed(DisarmCause::OutOfOrder));
            State::Locked(nonce)
        }
        Event::Aborted
        | Event::PortClosed
        | Event::PreconditionLost(_)
        | Event::FaultRaised(_)
        | Event::I2cAcked
        | Event::I2cNacked
        | Event::StatusRead(_)
        | Event::Elapsed(_) => State::Locked(nonce),
    }
}

fn disarm(nonce: ConfirmationNonce, cause: DisarmCause, outputs: &mut Outputs) -> State {
    outputs.push(Output::Disarmed(cause));
    State::Locked(nonce)
}

enum Arrival {
    Advance(Arming),
    Disarm(DisarmCause),
    Wait(Arming),
}

fn arming_common(arming: Arming, event: Event) -> Arrival {
    match event {
        Event::Elapsed(elapsed) => {
            let waited = later(arming.waited, elapsed);
            if reached(waited, ARMING_STEP_TIMEOUT) {
                Arrival::Disarm(DisarmCause::TimedOut)
            } else {
                Arrival::Wait(Arming {
                    nonce: arming.nonce,
                    waited,
                })
            }
        }
        Event::Aborted => Arrival::Disarm(DisarmCause::Aborted),
        Event::PortClosed => Arrival::Disarm(DisarmCause::PortClosed),
        Event::FaultRaised(_) => Arrival::Disarm(DisarmCause::FaultRaised),
        Event::PreconditionLost(fact) => Arrival::Disarm(DisarmCause::Unmet(fact)),
        Event::ProtocolSigned
        | Event::SpareConfirmed
        | Event::OutputsUnplugged
        | Event::ReplayRequested(_)
        | Event::NonceTyped(..)
        | Event::I2cAcked
        | Event::I2cNacked
        | Event::StatusRead(_) => Arrival::Disarm(DisarmCause::OutOfOrder),
    }
}

fn armed(checklist: Checklist, arming: Arming, event: Event, outputs: &mut Outputs) -> State {
    let arrival = match (checklist, event) {
        (Checklist::Signed, Event::SpareConfirmed) => {
            return State::Arming(Checklist::OnSpare, fresh(arming.nonce));
        }
        (Checklist::OnSpare, Event::OutputsUnplugged) => {
            return State::Arming(Checklist::Unplugged, fresh(arming.nonce));
        }
        (Checklist::Unplugged, Event::ReplayRequested(record)) => match record.first_unmet() {
            Some(fact) => Arrival::Disarm(DisarmCause::Unmet(fact)),
            None => Arrival::Advance(fresh(arming.nonce)),
        },
        (
            Checklist::Signed | Checklist::OnSpare | Checklist::Unplugged,
            Event::ProtocolSigned
            | Event::SpareConfirmed
            | Event::OutputsUnplugged
            | Event::ReplayRequested(_)
            | Event::NonceTyped(..)
            | Event::Aborted
            | Event::PortClosed
            | Event::PreconditionLost(_)
            | Event::FaultRaised(_)
            | Event::I2cAcked
            | Event::I2cNacked
            | Event::StatusRead(_)
            | Event::Elapsed(_),
        ) => arming_common(arming, event),
    };
    match arrival {
        Arrival::Advance(next) => {
            outputs.push(Output::ShowNonce(next.nonce));
            State::NonceIssued(next)
        }
        Arrival::Wait(next) => State::Arming(checklist, next),
        Arrival::Disarm(cause) => disarm(arming.nonce, cause, outputs),
    }
}

fn nonce_issued(arming: Arming, event: Event, outputs: &mut Outputs) -> State {
    let arrival = match event {
        Event::NonceTyped(typed, record) => match (typed == arming.nonce, record.first_unmet()) {
            (false, _) => Arrival::Disarm(DisarmCause::WrongNonce),
            (true, Some(fact)) => Arrival::Disarm(DisarmCause::Unmet(fact)),
            (true, None) => Arrival::Advance(arming),
        },
        Event::ProtocolSigned
        | Event::SpareConfirmed
        | Event::OutputsUnplugged
        | Event::ReplayRequested(_)
        | Event::Aborted
        | Event::PortClosed
        | Event::PreconditionLost(_)
        | Event::FaultRaised(_)
        | Event::I2cAcked
        | Event::I2cNacked
        | Event::StatusRead(_)
        | Event::Elapsed(_) => arming_common(arming, event),
    };
    match arrival {
        Arrival::Advance(_) => {
            outputs.push(Output::WriteRegisterZero(RegisterZeroWrite(Sealed)));
            State::Replaying(Replay {
                step: ReplayStep::RegisterZero,
                wait: awaiting(Answer::Ack, Attempt::FIRST),
            })
        }
        Arrival::Wait(next) => State::NonceIssued(next),
        Arrival::Disarm(cause) => disarm(arming.nonce, cause, outputs),
    }
}

fn fail(code: FaultCode, outputs: &mut Outputs) -> State {
    outputs.push(Output::Supervisor(supervisor::Event::FaultRaised(
        code,
        At::Here,
    )));
    attenuate_everything(outputs);
    State::Failed(code)
}

fn attenuate_everything(outputs: &mut Outputs) {
    VolumeRegister::ALL
        .into_iter()
        .for_each(|register| outputs.push(Output::Command(fully_attenuate(register))));
}

const fn awaiting(expected: Answer, attempt: Attempt) -> ReplayWait {
    ReplayWait::Answer {
        expected,
        attempt,
        waited: Milliseconds(0),
    }
}

fn restore_from(mut index: usize, outputs: &mut Outputs) -> State {
    while let Some(&step) = RESTORE_SCRIPT.get(index) {
        let wait = match step {
            ScriptStep::Hold(remaining) => ReplayWait::Hold { remaining },
            ScriptStep::Emit(command) => {
                outputs.push(Output::Command(command));
                match restore_step(command) {
                    RestoreStep::Emit => {
                        index += 1;
                        continue;
                    }
                    RestoreStep::Transfer(expected) => awaiting(expected, Attempt::FIRST),
                }
            }
        };
        return State::Replaying(Replay {
            step: ReplayStep::Restore(index),
            wait,
        });
    }
    State::HeldAttenuated
}

const fn next_index(step: ReplayStep) -> usize {
    match step {
        ReplayStep::RegisterZero => 0,
        ReplayStep::Restore(index) => index + 1,
    }
}

fn fault_from_elsewhere(code: FaultCode, outputs: &mut Outputs) -> State {
    attenuate_everything(outputs);
    State::Failed(code)
}

fn replaying(replay: Replay, event: Event, outputs: &mut Outputs) -> State {
    let unchanged = State::Replaying(replay);
    match replay.wait {
        ReplayWait::Hold { remaining } => match event {
            Event::Elapsed(elapsed) => match count_down(remaining, elapsed) {
                Countdown::Over => restore_from(next_index(replay.step), outputs),
                Countdown::Left(remaining) => State::Replaying(Replay {
                    step: replay.step,
                    wait: ReplayWait::Hold { remaining },
                }),
            },
            Event::FaultRaised(code) => fault_from_elsewhere(code, outputs),
            Event::ProtocolSigned
            | Event::SpareConfirmed
            | Event::OutputsUnplugged
            | Event::ReplayRequested(_)
            | Event::NonceTyped(..)
            | Event::Aborted
            | Event::PortClosed
            | Event::PreconditionLost(_)
            | Event::I2cAcked
            | Event::I2cNacked
            | Event::StatusRead(_) => unchanged,
        },
        ReplayWait::Answer {
            expected,
            attempt,
            waited,
        } => {
            let answered = match (expected, event) {
                (Answer::Ack, Event::I2cAcked) => true,
                (Answer::StatusByte, Event::StatusRead(byte)) => {
                    outputs.push(Output::Log(BootRecord::DacStatus(byte)));
                    true
                }
                (Answer::Ack | Answer::StatusByte, Event::I2cNacked) => false,
                (Answer::Ack | Answer::StatusByte, Event::Elapsed(elapsed)) => {
                    let waited = later(waited, elapsed);
                    if !reached(waited, PLACEHOLDER_I2C_ANSWER_TIMEOUT_UNTIL_T23) {
                        return State::Replaying(Replay {
                            step: replay.step,
                            wait: ReplayWait::Answer {
                                expected,
                                attempt,
                                waited,
                            },
                        });
                    }
                    false
                }
                (Answer::Ack | Answer::StatusByte, Event::FaultRaised(code)) => {
                    return fault_from_elsewhere(code, outputs);
                }
                (Answer::Ack, Event::StatusRead(_))
                | (Answer::StatusByte, Event::I2cAcked)
                | (
                    Answer::Ack | Answer::StatusByte,
                    Event::ProtocolSigned
                    | Event::SpareConfirmed
                    | Event::OutputsUnplugged
                    | Event::ReplayRequested(_)
                    | Event::NonceTyped(..)
                    | Event::Aborted
                    | Event::PortClosed
                    | Event::PreconditionLost(_),
                ) => return unchanged,
            };
            if answered {
                restore_from(next_index(replay.step), outputs)
            } else {
                retry_or_fail(replay.step, expected, attempt, outputs)
            }
        }
    }
}

fn retry_or_fail(
    step: ReplayStep,
    expected: Answer,
    attempt: Attempt,
    outputs: &mut Outputs,
) -> State {
    let unanswered = FaultCode::Unanswered(I2cPeer::Dac);
    let index = match step {
        ReplayStep::RegisterZero => return fail(unanswered, outputs),
        ReplayStep::Restore(index) => index,
    };
    match (attempt.next(), RESTORE_SCRIPT.get(index)) {
        (Some(attempt), Some(&ScriptStep::Emit(command))) => {
            outputs.push(Output::Command(command));
            State::Replaying(Replay {
                step,
                wait: awaiting(expected, attempt),
            })
        }
        (None, _) | (_, None | Some(ScriptStep::Hold(_))) => fail(unanswered, outputs),
    }
}
