#![deny(
    clippy::wildcard_enum_match_arm,
    clippy::struct_excessive_bools,
    clippy::fn_params_excessive_bools
)]

use super::answer::{Answer, ControlContext, Effect, Reply, answer};
use super::device::HIGH_SPEED_CONTROL_PACKET_BYTES;
use super::request::{LINE_CODING_LENGTH, SAMPLING_FREQUENCY_LENGTH, SETUP_PACKET_LENGTH};
use crate::outputs::{Filler, OutputList};

pub const CONTROL_PACKET_BYTES: usize = HIGH_SPEED_CONTROL_PACKET_BYTES as usize;
pub const LONGEST_WRITE_DATA: usize = if LINE_CODING_LENGTH > SAMPLING_FREQUENCY_LENGTH {
    LINE_CODING_LENGTH
} else {
    SAMPLING_FREQUENCY_LENGTH
};
pub const MOST_OUTPUTS_PER_EVENT: usize = 3;

const PACKET_REMAINDER_MASK: usize = CONTROL_PACKET_BYTES - 1;
const DEVICE_TO_HOST: u8 = 0x80;
const _: () = assert!(CONTROL_PACKET_BYTES.is_power_of_two());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlPacket {
    bytes: [u8; CONTROL_PACKET_BYTES],
    length: usize,
}

impl ControlPacket {
    pub const ZERO_LENGTH: Self = Self {
        bytes: [0; CONTROL_PACKET_BYTES],
        length: 0,
    };

    fn of(chunk: &[u8]) -> Self {
        let mut packet = Self::ZERO_LENGTH;
        for (slot, byte) in packet.bytes.iter_mut().zip(chunk) {
            *slot = *byte;
            packet.length += 1;
        }
        packet
    }

    pub fn bytes(&self) -> &[u8] {
        self.bytes.get(..self.length).unwrap_or(&[])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InFlight {
    DataEndingAt(usize),
    ZeroLengthPacket,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sending<'a> {
    reply: Reply<'a>,
    limit: usize,
    in_flight: InFlight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Receiving {
    setup: [u8; SETUP_PACKET_LENGTH],
    data: [u8; LONGEST_WRITE_DATA],
    received: usize,
    expected: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State<'a> {
    Idle,
    SendingReply(Sending<'a>),
    AwaitingHostStatus,
    ReceivingData(Receiving),
    ConfirmingWrite(Effect),
    ConfirmingEmptyRead,
    Stalled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event<'p> {
    Setup([u8; SETUP_PACKET_LENGTH]),
    InSent,
    OutReceived(&'p [u8]),
    BusReset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    ReleaseStall,
    Send(ControlPacket),
    Receive,
    Stall,
    Effect(Effect),
}

impl Filler for Output {
    const FILLER: Self = Output::Receive;
}

pub type Outputs = OutputList<Output, MOST_OUTPUTS_PER_EVENT>;

impl State<'_> {
    pub const INITIAL: Self = Self::Idle;
}

const fn requested_length(setup: [u8; SETUP_PACKET_LENGTH]) -> usize {
    u16::from_le_bytes([setup[6], setup[7]]) as usize
}

const fn asks_the_device(setup: [u8; SETUP_PACKET_LENGTH]) -> bool {
    setup[0] & DEVICE_TO_HOST != 0
}

const fn fills_whole_packets(length: usize) -> bool {
    length & PACKET_REMAINDER_MASK == 0
}

fn stall<'a>(outputs: &mut Outputs) -> State<'a> {
    outputs.push(Output::Stall);
    State::Stalled
}

fn send_chunk<'a>(reply: Reply<'a>, limit: usize, from: usize, outputs: &mut Outputs) -> State<'a> {
    let bytes = reply.bytes();
    let end = bytes.len().min(from + CONTROL_PACKET_BYTES);
    outputs.push(Output::Send(ControlPacket::of(
        bytes.get(from..end).unwrap_or(&[]),
    )));
    State::SendingReply(Sending {
        reply,
        limit,
        in_flight: InFlight::DataEndingAt(end),
    })
}

fn confirm_write<'a>(effect: Effect, outputs: &mut Outputs) -> State<'a> {
    outputs.push(Output::Send(ControlPacket::ZERO_LENGTH));
    State::ConfirmingWrite(effect)
}

fn begin<'a>(
    setup: [u8; SETUP_PACKET_LENGTH],
    context: &ControlContext<'a>,
    outputs: &mut Outputs,
) -> State<'a> {
    outputs.push(Output::ReleaseStall);
    let length = requested_length(setup);
    let device_to_host = asks_the_device(setup);
    if !device_to_host && length > 0 {
        if length > LONGEST_WRITE_DATA {
            return stall(outputs);
        }
        outputs.push(Output::Receive);
        return State::ReceivingData(Receiving {
            setup,
            data: [0; LONGEST_WRITE_DATA],
            received: 0,
            expected: length,
        });
    }
    match (answer(setup, &[], context), device_to_host) {
        (Answer::Reply(_), true) if length == 0 => {
            outputs.push(Output::Send(ControlPacket::ZERO_LENGTH));
            State::ConfirmingEmptyRead
        }
        (Answer::Reply(reply), true) => {
            let state = send_chunk(reply, length, 0, outputs);
            outputs.push(Output::Receive);
            state
        }
        (Answer::Accept(effect), false) => confirm_write(effect, outputs),
        (Answer::Reply(_), false) | (Answer::Accept(_), true) | (Answer::Stall, true | false) => {
            stall(outputs)
        }
    }
}

fn after_packet_sent<'a>(sending: Sending<'a>, outputs: &mut Outputs) -> State<'a> {
    let total = sending.reply.bytes().len();
    match sending.in_flight {
        InFlight::DataEndingAt(end) if end < total => {
            send_chunk(sending.reply, sending.limit, end, outputs)
        }
        InFlight::DataEndingAt(_)
            if total > 0 && fills_whole_packets(total) && total < sending.limit =>
        {
            outputs.push(Output::Send(ControlPacket::ZERO_LENGTH));
            State::SendingReply(Sending {
                in_flight: InFlight::ZeroLengthPacket,
                ..sending
            })
        }
        InFlight::DataEndingAt(_) | InFlight::ZeroLengthPacket => State::AwaitingHostStatus,
    }
}

fn data_arrived<'a>(
    receiving: Receiving,
    packet: &[u8],
    context: &ControlContext<'a>,
    outputs: &mut Outputs,
) -> State<'a> {
    let received = receiving.received + packet.len();
    if received > receiving.expected {
        return stall(outputs);
    }
    let mut data = receiving.data;
    for (slot, byte) in data.iter_mut().skip(receiving.received).zip(packet) {
        *slot = *byte;
    }
    if received < receiving.expected && packet.len() == CONTROL_PACKET_BYTES {
        outputs.push(Output::Receive);
        return State::ReceivingData(Receiving {
            data,
            received,
            ..receiving
        });
    }
    match answer(
        receiving.setup,
        data.get(..received).unwrap_or(&[]),
        context,
    ) {
        Answer::Accept(effect) => confirm_write(effect, outputs),
        Answer::Reply(_) | Answer::Stall => stall(outputs),
    }
}

pub fn transition<'a>(
    state: State<'a>,
    event: Event<'_>,
    context: &ControlContext<'a>,
) -> (State<'a>, Outputs) {
    let mut outputs = Outputs::NONE;
    let next = match (state, event) {
        (
            State::Idle
            | State::SendingReply(_)
            | State::AwaitingHostStatus
            | State::ReceivingData(_)
            | State::ConfirmingWrite(_)
            | State::ConfirmingEmptyRead
            | State::Stalled,
            Event::Setup(setup),
        ) => begin(setup, context, &mut outputs),
        (
            State::Idle
            | State::SendingReply(_)
            | State::AwaitingHostStatus
            | State::ReceivingData(_)
            | State::ConfirmingWrite(_)
            | State::ConfirmingEmptyRead
            | State::Stalled,
            Event::BusReset,
        ) => State::Idle,
        (State::SendingReply(sending), Event::InSent) => after_packet_sent(sending, &mut outputs),
        (State::SendingReply(_) | State::AwaitingHostStatus, Event::OutReceived([])) => State::Idle,
        (State::SendingReply(_) | State::AwaitingHostStatus, Event::OutReceived(_)) => {
            stall(&mut outputs)
        }
        (State::ReceivingData(receiving), Event::OutReceived(packet)) => {
            data_arrived(receiving, packet, context, &mut outputs)
        }
        (State::ConfirmingWrite(effect), Event::InSent) => {
            outputs.push(Output::Effect(effect));
            State::Idle
        }
        (State::ConfirmingEmptyRead, Event::InSent) => State::Idle,
        (
            State::Idle | State::AwaitingHostStatus | State::ReceivingData(_) | State::Stalled,
            Event::InSent,
        )
        | (
            State::Idle | State::ConfirmingWrite(_) | State::ConfirmingEmptyRead | State::Stalled,
            Event::OutReceived(_),
        ) => state,
    };
    (next, outputs)
}
