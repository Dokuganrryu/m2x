use crate::command::Command;
use crate::dac::{DAC_ADDRESS, DacRegister, DacTransaction};
use crate::expander::{
    EXPANDER_ADDRESS, EXPANDER_BUTTONS_AS_INPUTS, EXPANDER_DIRECTION_REGISTER,
    EXPANDER_INPUT_REGISTER, EXPANDER_OUTPUT_REGISTER,
};
use crate::outputs::{Filler, OutputList};

pub const MOST_OUTPUTS_PER_EVENT: usize = 3;
pub const MOST_WRITES_PER_TRANSFER: usize = 4;

const WRITE_DIRECTION: u8 = 0;
const READ_DIRECTION: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Peer(u8);

impl Peer {
    const fn addressed_for(self, direction: u8) -> u8 {
        self.0 << 1 | direction
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteWrite {
    peer: Peer,
    register: u8,
    value: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Writes {
    items: [ByteWrite; MOST_WRITES_PER_TRANSFER],
    len: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteRead {
    peer: Peer,
    register: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transfer {
    Writes(Writes),
    Read(ByteRead),
    Probe(Peer),
}

impl Transfer {
    pub const fn peer_address(self) -> u8 {
        match self {
            Transfer::Writes(writes) => writes.items[0].peer.0,
            Transfer::Read(read) => read.peer.0,
            Transfer::Probe(peer) => peer.0,
        }
    }

    pub fn of(command: Command) -> Option<Self> {
        Some(match command {
            Command::WriteDac(write) => {
                let transactions = write.transactions();
                let mut items = [ByteWrite {
                    peer: Peer(DAC_ADDRESS),
                    register: 0,
                    value: 0,
                }; MOST_WRITES_PER_TRANSFER];
                let listed = transactions.as_slice();
                for (slot, transaction) in items.iter_mut().zip(listed) {
                    *slot = dac_write(*transaction);
                }
                Transfer::Writes(Writes {
                    items,
                    len: listed.len(),
                })
            }
            Command::ReadDac(register) => Transfer::Read(ByteRead {
                peer: Peer(DAC_ADDRESS),
                register: DacRegister::Status(register).number(),
            }),
            Command::ProbeAdc(address) => Transfer::Probe(Peer(address.address())),
            Command::WriteAdc(write) => Transfer::Writes(Writes::one(ByteWrite {
                peer: Peer(write.address().address()),
                register: write.register(),
                value: write.value(),
            })),
            Command::WriteExpanderDirection => Transfer::Writes(Writes::one(ByteWrite {
                peer: Peer(EXPANDER_ADDRESS),
                register: EXPANDER_DIRECTION_REGISTER,
                value: EXPANDER_BUTTONS_AS_INPUTS,
            })),
            Command::ReadExpanderInputs => Transfer::Read(ByteRead {
                peer: Peer(EXPANDER_ADDRESS),
                register: EXPANDER_INPUT_REGISTER,
            }),
            Command::WriteExpanderOutputs(outputs) => Transfer::Writes(Writes::one(ByteWrite {
                peer: Peer(EXPANDER_ADDRESS),
                register: EXPANDER_OUTPUT_REGISTER,
                value: outputs.bits(),
            })),
            Command::DrivePin(..)
            | Command::SendAnalogWord(_)
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
            | Command::StartKnobScan
            | Command::ZeroTransmitBuffers
            | Command::StopTransmitDma
            | Command::LatchPlaybackSilence
            | Command::RecordFault(..) => return None,
        })
    }
}

const fn dac_write(transaction: DacTransaction) -> ByteWrite {
    ByteWrite {
        peer: Peer(DAC_ADDRESS),
        register: transaction.register.number(),
        value: transaction.value,
    }
}

impl Writes {
    const fn one(write: ByteWrite) -> Self {
        Self {
            items: [write; MOST_WRITES_PER_TRANSFER],
            len: 1,
        }
    }

    const fn get(&self, index: usize) -> Option<ByteWrite> {
        if index < self.len {
            Some(self.items[index])
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MasterStatus {
    InProgress,
    Idle,
    ReceiveReady(u8),
    TransmitReady,
    NackAddress,
    NackData,
    Disrupted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusAction {
    Transmit(u8),
    Start,
    Continue,
    Stop,
    ResetMaster,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Acked,
    Nacked,
    Read(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    Acked,
    Nacked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WritePhase {
    Address,
    Register,
    Value,
    Stopping(Ending),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadEnding {
    Byte(u8),
    Refused,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadPhase {
    Address,
    Register,
    ReadAddress,
    Stopping(ReadEnding),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbePhase {
    Address,
    Stopping(Ending),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Writing {
    writes: Writes,
    index: usize,
    phase: WritePhase,
}

impl Writing {
    pub const fn phase(self) -> WritePhase {
        self.phase
    }

    pub const fn transaction(self) -> usize {
        self.index
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reading {
    read: ByteRead,
    phase: ReadPhase,
}

impl Reading {
    pub const fn phase(self) -> ReadPhase {
        self.phase
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Probing {
    peer: Peer,
    phase: ProbePhase,
}

impl Probing {
    pub const fn phase(self) -> ProbePhase {
        self.phase
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Writing(Writing),
    Reading(Reading),
    Probing(Probing),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Begin(Transfer),
    Polled(MasterStatus),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Act(BusAction),
    Answered(Answer),
}

impl Filler for Output {
    const FILLER: Self = Output::Act(BusAction::Stop);
}

pub type Outputs = OutputList<Output, MOST_OUTPUTS_PER_EVENT>;

pub fn transition(state: State, event: Event) -> (State, Outputs) {
    match (state, event) {
        (State::Idle, Event::Begin(transfer)) => begin(transfer, Outputs::NONE),
        (State::Writing(_) | State::Reading(_) | State::Probing(_), Event::Begin(transfer)) => {
            begin(transfer, Outputs::of([Output::Act(BusAction::ResetMaster)]))
        }
        (State::Idle, Event::Polled(_)) => (state, Outputs::NONE),
        (State::Writing(writing), Event::Polled(status)) => writing_polled(writing, status),
        (State::Reading(reading), Event::Polled(status)) => reading_polled(reading, status),
        (State::Probing(probing), Event::Polled(status)) => probing_polled(probing, status),
    }
}

fn begin(transfer: Transfer, mut outputs: Outputs) -> (State, Outputs) {
    let (state, peer) = match transfer {
        Transfer::Writes(writes) => match writes.get(0) {
            Some(first) => (
                State::Writing(Writing {
                    writes,
                    index: 0,
                    phase: WritePhase::Address,
                }),
                first.peer,
            ),
            None => {
                outputs.push(Output::Answered(Answer::Acked));
                return (State::Idle, outputs);
            }
        },
        Transfer::Read(read) => (
            State::Reading(Reading {
                read,
                phase: ReadPhase::Address,
            }),
            read.peer,
        ),
        Transfer::Probe(peer) => (
            State::Probing(Probing {
                peer,
                phase: ProbePhase::Address,
            }),
            peer,
        ),
    };
    push_start(&mut outputs, peer.addressed_for(WRITE_DIRECTION));
    (state, outputs)
}

fn push_start(outputs: &mut Outputs, address_byte: u8) {
    outputs.push(Output::Act(BusAction::Transmit(address_byte)));
    outputs.push(Output::Act(BusAction::Start));
}

fn send(byte: u8) -> Outputs {
    Outputs::of([
        Output::Act(BusAction::Transmit(byte)),
        Output::Act(BusAction::Continue),
    ])
}

const STOP: Outputs = Outputs::of([Output::Act(BusAction::Stop)]);

const ABANDON: Outputs = Outputs::of([
    Output::Act(BusAction::ResetMaster),
    Output::Answered(Answer::Nacked),
]);

fn writing_polled(writing: Writing, status: MasterStatus) -> (State, Outputs) {
    let at = |phase| State::Writing(Writing { phase, ..writing });
    let Some(current) = writing.writes.get(writing.index) else {
        return (State::Idle, ABANDON);
    };
    match (writing.phase, status) {
        (WritePhase::Address, MasterStatus::TransmitReady) => {
            (at(WritePhase::Register), send(current.register))
        }
        (WritePhase::Register, MasterStatus::TransmitReady) => {
            (at(WritePhase::Value), send(current.value))
        }
        (WritePhase::Value, MasterStatus::TransmitReady) => {
            (at(WritePhase::Stopping(Ending::Acked)), STOP)
        }
        (WritePhase::Address, MasterStatus::NackAddress)
        | (WritePhase::Register | WritePhase::Value, MasterStatus::NackData) => {
            (at(WritePhase::Stopping(Ending::Nacked)), STOP)
        }
        (WritePhase::Stopping(Ending::Acked), MasterStatus::Idle) => {
            let next = writing.index + 1;
            match writing.writes.get(next) {
                Some(following) => {
                    let mut outputs = Outputs::NONE;
                    push_start(&mut outputs, following.peer.addressed_for(WRITE_DIRECTION));
                    (
                        State::Writing(Writing {
                            writes: writing.writes,
                            index: next,
                            phase: WritePhase::Address,
                        }),
                        outputs,
                    )
                }
                None => (State::Idle, Outputs::of([Output::Answered(Answer::Acked)])),
            }
        }
        (WritePhase::Stopping(Ending::Nacked), MasterStatus::Idle) => {
            (State::Idle, Outputs::of([Output::Answered(Answer::Nacked)]))
        }
        (
            WritePhase::Address | WritePhase::Register | WritePhase::Value,
            MasterStatus::Idle | MasterStatus::ReceiveReady(_) | MasterStatus::Disrupted,
        )
        | (WritePhase::Address, MasterStatus::NackData)
        | (WritePhase::Register | WritePhase::Value, MasterStatus::NackAddress)
        | (
            WritePhase::Stopping(_),
            MasterStatus::ReceiveReady(_)
            | MasterStatus::TransmitReady
            | MasterStatus::NackAddress
            | MasterStatus::NackData
            | MasterStatus::Disrupted,
        ) => (State::Idle, ABANDON),
        (
            WritePhase::Address
            | WritePhase::Register
            | WritePhase::Value
            | WritePhase::Stopping(_),
            MasterStatus::InProgress,
        ) => (State::Writing(writing), Outputs::NONE),
    }
}

fn reading_polled(reading: Reading, status: MasterStatus) -> (State, Outputs) {
    let at = |phase| State::Reading(Reading { phase, ..reading });
    match (reading.phase, status) {
        (ReadPhase::Address, MasterStatus::TransmitReady) => {
            (at(ReadPhase::Register), send(reading.read.register))
        }
        (ReadPhase::Register, MasterStatus::TransmitReady) => {
            let mut outputs = Outputs::NONE;
            push_start(
                &mut outputs,
                reading.read.peer.addressed_for(READ_DIRECTION),
            );
            (at(ReadPhase::ReadAddress), outputs)
        }
        (ReadPhase::ReadAddress, MasterStatus::ReceiveReady(byte)) => {
            (at(ReadPhase::Stopping(ReadEnding::Byte(byte))), STOP)
        }
        (ReadPhase::Address | ReadPhase::ReadAddress, MasterStatus::NackAddress)
        | (ReadPhase::Register, MasterStatus::NackData) => {
            (at(ReadPhase::Stopping(ReadEnding::Refused)), STOP)
        }
        (ReadPhase::Stopping(ReadEnding::Byte(byte)), MasterStatus::Idle) => (
            State::Idle,
            Outputs::of([Output::Answered(Answer::Read(byte))]),
        ),
        (ReadPhase::Stopping(ReadEnding::Refused), MasterStatus::Idle) => {
            (State::Idle, Outputs::of([Output::Answered(Answer::Nacked)]))
        }
        (
            ReadPhase::Address | ReadPhase::Register | ReadPhase::ReadAddress,
            MasterStatus::Idle | MasterStatus::Disrupted,
        )
        | (ReadPhase::Address | ReadPhase::Register, MasterStatus::ReceiveReady(_))
        | (ReadPhase::ReadAddress, MasterStatus::TransmitReady | MasterStatus::NackData)
        | (ReadPhase::Address, MasterStatus::NackData)
        | (ReadPhase::Register, MasterStatus::NackAddress)
        | (
            ReadPhase::Stopping(_),
            MasterStatus::ReceiveReady(_)
            | MasterStatus::TransmitReady
            | MasterStatus::NackAddress
            | MasterStatus::NackData
            | MasterStatus::Disrupted,
        ) => (State::Idle, ABANDON),
        (
            ReadPhase::Address
            | ReadPhase::Register
            | ReadPhase::ReadAddress
            | ReadPhase::Stopping(_),
            MasterStatus::InProgress,
        ) => (State::Reading(reading), Outputs::NONE),
    }
}

fn probing_polled(probing: Probing, status: MasterStatus) -> (State, Outputs) {
    let at = |phase| State::Probing(Probing { phase, ..probing });
    match (probing.phase, status) {
        (ProbePhase::Address, MasterStatus::TransmitReady) => {
            (at(ProbePhase::Stopping(Ending::Acked)), STOP)
        }
        (ProbePhase::Address, MasterStatus::NackAddress) => {
            (at(ProbePhase::Stopping(Ending::Nacked)), STOP)
        }
        (ProbePhase::Stopping(Ending::Acked), MasterStatus::Idle) => {
            (State::Idle, Outputs::of([Output::Answered(Answer::Acked)]))
        }
        (ProbePhase::Stopping(Ending::Nacked), MasterStatus::Idle) => {
            (State::Idle, Outputs::of([Output::Answered(Answer::Nacked)]))
        }
        (
            ProbePhase::Address,
            MasterStatus::Idle
            | MasterStatus::ReceiveReady(_)
            | MasterStatus::NackData
            | MasterStatus::Disrupted,
        )
        | (
            ProbePhase::Stopping(_),
            MasterStatus::ReceiveReady(_)
            | MasterStatus::TransmitReady
            | MasterStatus::NackAddress
            | MasterStatus::NackData
            | MasterStatus::Disrupted,
        ) => (State::Idle, ABANDON),
        (ProbePhase::Address | ProbePhase::Stopping(_), MasterStatus::InProgress) => {
            (State::Probing(probing), Outputs::NONE)
        }
    }
}
