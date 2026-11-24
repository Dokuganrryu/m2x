#![deny(
    clippy::wildcard_enum_match_arm,
    clippy::struct_excessive_bools,
    clippy::fn_params_excessive_bools
)]

use crate::command::Command;
use crate::dac::{Attenuation, DacWrite, VolumeRegister};
use crate::expander::{ExpanderInputs, ExpanderOutputs};
use crate::fault::{FaultCode, I2cPeer};
use crate::i2c::Answer;
use crate::outputs::{Filler, OutputList};

pub const CONTROL_VOLUME_REGISTERS: [VolumeRegister; 4] = [
    VolumeRegister::MainA,
    VolumeRegister::MainB,
    VolumeRegister::HeadphoneA,
    VolumeRegister::HeadphoneB,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonRead {
    NotWanted,
    Wanted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expander {
    Unready,
    Ready,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelTransfer {
    Volume(VolumeRegister, Attenuation),
    Leds(ExpanderOutputs),
    Buttons,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mailbox {
    volume: [Option<Attenuation>; 4],
    leds: Option<ExpanderOutputs>,
    buttons: ButtonRead,
    written_leds: Option<ExpanderOutputs>,
    expander: Expander,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Waiting(Mailbox),
    InFlight {
        mailbox: Mailbox,
        transfer: PanelTransfer,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Wanted(PanelTransfer),
    BusFree,
    Answered(Answer),
    Preempted,
    ExpanderReady,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Begin(Command),
    ButtonsRead(ExpanderInputs),
    Fault(FaultCode),
}

impl Filler for Output {
    const FILLER: Self = Output::Begin(Command::ReadExpanderInputs);
}

pub type Outputs = OutputList<Output, 1>;

impl Mailbox {
    pub const EMPTY: Self = Self {
        volume: [None; 4],
        leds: None,
        buttons: ButtonRead::NotWanted,
        written_leds: None,
        expander: Expander::Ready,
    };

    pub const fn written_leds(self) -> Option<ExpanderOutputs> {
        self.written_leds
    }

    pub fn holds(self, transfer: PanelTransfer) -> bool {
        match transfer {
            PanelTransfer::Volume(register, level) => {
                slot(register).is_some_and(|index| self.volume[index] == Some(level))
            }
            PanelTransfer::Leds(outputs) => self.leds == Some(outputs),
            PanelTransfer::Buttons => self.buttons == ButtonRead::Wanted,
        }
    }

    pub fn is_empty(self) -> bool {
        self.volume.iter().all(Option::is_none)
            && self.leds.is_none()
            && self.buttons == ButtonRead::NotWanted
    }

    fn wanting(self, transfer: PanelTransfer) -> Self {
        match transfer {
            PanelTransfer::Volume(register, level) => match slot(register) {
                Some(index) => {
                    let mut volume = self.volume;
                    volume[index] = Some(level);
                    Self { volume, ..self }
                }
                None => self,
            },
            PanelTransfer::Leds(outputs) => Self {
                leds: (self.written_leds != Some(outputs)).then_some(outputs),
                ..self
            },
            PanelTransfer::Buttons => Self {
                buttons: ButtonRead::Wanted,
                ..self
            },
        }
    }

    fn requeued(self, transfer: PanelTransfer) -> Self {
        let superseded = match transfer {
            PanelTransfer::Volume(register, _) => {
                slot(register).is_some_and(|index| self.volume[index].is_some())
            }
            PanelTransfer::Leds(_) => self.leds.is_some(),
            PanelTransfer::Buttons => false,
        };
        if superseded {
            self
        } else {
            self.wanting(transfer)
        }
    }

    fn taken(self) -> Option<(PanelTransfer, Self)> {
        let expander_ready = self.expander == Expander::Ready;
        if expander_ready && let Some(outputs) = self.leds {
            return Some((PanelTransfer::Leds(outputs), Self { leds: None, ..self }));
        }
        if let Some(index) = self.volume.iter().position(Option::is_some) {
            let mut volume = self.volume;
            let level = volume[index].take()?;
            return Some((
                PanelTransfer::Volume(CONTROL_VOLUME_REGISTERS[index], level),
                Self { volume, ..self },
            ));
        }
        match self.buttons {
            ButtonRead::Wanted if expander_ready => Some((
                PanelTransfer::Buttons,
                Self {
                    buttons: ButtonRead::NotWanted,
                    ..self
                },
            )),
            ButtonRead::Wanted | ButtonRead::NotWanted => None,
        }
    }

    fn expander_ready(self) -> Self {
        Self {
            expander: Expander::Ready,
            ..self
        }
    }
}

fn slot(register: VolumeRegister) -> Option<usize> {
    CONTROL_VOLUME_REGISTERS
        .iter()
        .position(|control| *control == register)
}

impl PanelTransfer {
    pub const fn command(self) -> Command {
        match self {
            PanelTransfer::Volume(register, level) => {
                Command::WriteDac(DacWrite::Volume(register, level))
            }
            PanelTransfer::Leds(outputs) => Command::WriteExpanderOutputs(outputs),
            PanelTransfer::Buttons => Command::ReadExpanderInputs,
        }
    }

    pub const fn of_pass_command(command: Command) -> Option<Self> {
        match command {
            Command::WriteDac(DacWrite::Volume(register, level)) => {
                Some(PanelTransfer::Volume(register, level))
            }
            Command::WriteExpanderOutputs(outputs) => Some(PanelTransfer::Leds(outputs)),
            Command::ReadExpanderInputs => Some(PanelTransfer::Buttons),
            Command::WriteDac(_)
            | Command::DrivePin(..)
            | Command::SendAnalogWord(_)
            | Command::ReadDac(_)
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
            | Command::StartKnobScan
            | Command::ZeroTransmitBuffers
            | Command::StopTransmitDma
            | Command::LatchPlaybackSilence
            | Command::RecordFault(..) => None,
        }
    }
}

impl State {
    pub const START: Self = State::Waiting(Mailbox::EMPTY);
    pub const AT_BOOT: Self = State::Waiting(Mailbox {
        expander: Expander::Unready,
        ..Mailbox::EMPTY
    });

    pub const fn mailbox(self) -> Mailbox {
        match self {
            State::Waiting(mailbox) | State::InFlight { mailbox, .. } => mailbox,
        }
    }

    pub const fn in_flight(self) -> Option<PanelTransfer> {
        match self {
            State::Waiting(_) => None,
            State::InFlight { transfer, .. } => Some(transfer),
        }
    }
}

fn quiet(state: State) -> (State, Outputs) {
    (state, Outputs::NONE)
}

fn emitting(state: State, output: Output) -> (State, Outputs) {
    (state, Outputs::of([output]))
}

pub fn transition(state: State, event: Event) -> (State, Outputs) {
    match (state, event) {
        (State::Waiting(mailbox), Event::Wanted(transfer)) => {
            quiet(State::Waiting(mailbox.wanting(transfer)))
        }
        (State::Waiting(mailbox), Event::BusFree) => match mailbox.taken() {
            Some((transfer, mailbox)) => emitting(
                State::InFlight { mailbox, transfer },
                Output::Begin(transfer.command()),
            ),
            None => quiet(state),
        },
        (State::Waiting(mailbox), Event::ExpanderReady) => {
            quiet(State::Waiting(mailbox.expander_ready()))
        }
        (State::Waiting(_), Event::Answered(_) | Event::Preempted) => quiet(state),
        (State::InFlight { mailbox, transfer }, Event::Wanted(wanted)) => quiet(State::InFlight {
            mailbox: mailbox.wanting(wanted),
            transfer,
        }),
        (State::InFlight { mailbox, transfer }, Event::ExpanderReady) => quiet(State::InFlight {
            mailbox: mailbox.expander_ready(),
            transfer,
        }),
        (State::InFlight { .. }, Event::BusFree) => quiet(state),
        (State::InFlight { mailbox, transfer }, Event::Preempted) => {
            quiet(State::Waiting(mailbox.requeued(transfer)))
        }
        (State::InFlight { mailbox, transfer }, Event::Answered(answer)) => {
            answered(mailbox, transfer, answer)
        }
    }
}

fn answered(mailbox: Mailbox, transfer: PanelTransfer, answer: Answer) -> (State, Outputs) {
    match (transfer, answer) {
        (PanelTransfer::Buttons, Answer::Read(byte)) => emitting(
            State::Waiting(mailbox),
            Output::ButtonsRead(ExpanderInputs(byte)),
        ),
        (PanelTransfer::Buttons, Answer::Acked | Answer::Nacked)
        | (PanelTransfer::Leds(_), Answer::Nacked)
        | (PanelTransfer::Volume(..), Answer::Acked | Answer::Read(_)) => {
            quiet(State::Waiting(mailbox))
        }
        (PanelTransfer::Leds(outputs), Answer::Acked | Answer::Read(_)) => {
            quiet(State::Waiting(Mailbox {
                written_leds: Some(outputs),
                leds: mailbox.leds.filter(|pending| *pending != outputs),
                ..mailbox
            }))
        }
        (PanelTransfer::Volume(..), Answer::Nacked) => emitting(
            State::Waiting(mailbox),
            Output::Fault(FaultCode::Unanswered(I2cPeer::Dac)),
        ),
    }
}
