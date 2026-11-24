#![allow(dead_code)]

use device::usb::answer::ControlContext;
use device::usb::control::{Event, Output, State, transition};

pub const GET_DEVICE_DESCRIPTOR_64: [u8; 8] = [0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x40, 0x00];
pub const GET_DEVICE_DESCRIPTOR_18: [u8; 8] = [0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x12, 0x00];
pub const GET_DEVICE_DESCRIPTOR_NO_DATA: [u8; 8] = [0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00];
pub const GET_CONFIGURATION_DESCRIPTOR_255: [u8; 8] =
    [0x80, 0x06, 0x00, 0x02, 0x00, 0x00, 0xFF, 0x00];
pub const GET_CONFIGURATION_DESCRIPTOR_ALL: [u8; 8] =
    [0x80, 0x06, 0x00, 0x02, 0x00, 0x00, 0xFF, 0xFF];
pub const GET_CONFIGURATION_DESCRIPTOR_9: [u8; 8] =
    [0x80, 0x06, 0x00, 0x02, 0x00, 0x00, 0x09, 0x00];
pub const GET_PRODUCT_STRING_255: [u8; 8] = [0x80, 0x06, 0x02, 0x03, 0x09, 0x04, 0xFF, 0x00];
pub const GET_PRODUCT_STRING_64: [u8; 8] = [0x80, 0x06, 0x02, 0x03, 0x09, 0x04, 0x40, 0x00];
pub const VENDOR_OUT_Q: [u8; 8] = [0x40, b'q', 0x00, 0x00, 0x07, 0x00, 0x00, 0x00];
pub const SET_ADDRESS_9: [u8; 8] = [0x00, 0x05, 0x09, 0x00, 0x00, 0x00, 0x00, 0x00];
pub const SET_CONFIGURATION_1: [u8; 8] = [0x00, 0x09, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00];
pub const SET_PLAYBACK_ALTERNATE_1: [u8; 8] = [0x01, 0x0B, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00];
pub const SET_CUR_SAMPLING_FREQUENCY: [u8; 8] = [0x21, 0x01, 0x00, 0x01, 0x00, 0x0A, 0x04, 0x00];
pub const GET_CUR_SAMPLING_FREQUENCY: [u8; 8] = [0xA1, 0x01, 0x00, 0x01, 0x00, 0x0A, 0x04, 0x00];
pub const SET_LINE_CODING: [u8; 8] = [0x21, 0x20, 0x00, 0x00, 0x03, 0x00, 0x07, 0x00];
pub const SET_LINE_CODING_TOO_LONG: [u8; 8] = [0x21, 0x20, 0x00, 0x00, 0x03, 0x00, 0x08, 0x00];
pub const LINE_CODING_115200_8N1: [u8; 7] = [0x00, 0xC2, 0x01, 0x00, 0x00, 0x00, 0x08];
pub const NINETY_SIX_KILOHERTZ: [u8; 4] = 96_000u32.to_le_bytes();
pub const FULL_JUNK_PACKET: [u8; 64] = [0xA5; 64];

pub fn describe(output: &Output) -> String {
    match output {
        Output::ReleaseStall => "ReleaseStall".into(),
        Output::Send(packet) => format!("Send({})", packet.bytes().len()),
        Output::Receive => "Receive".into(),
        Output::Stall => "Stall".into(),
        Output::Effect(effect) => format!("Effect({effect:?})"),
    }
}

pub fn state_name(state: &State<'_>) -> &'static str {
    match state {
        State::Idle => "Idle",
        State::SendingReply(_) => "SendingReply",
        State::AwaitingHostStatus => "AwaitingHostStatus",
        State::ReceivingData(_) => "ReceivingData",
        State::ConfirmingWrite(_) => "ConfirmingWrite",
        State::ConfirmingEmptyRead => "ConfirmingEmptyRead",
        State::Stalled => "Stalled",
    }
}

pub fn step<'a>(
    state: State<'a>,
    event: Event<'_>,
    context: &ControlContext<'a>,
) -> (State<'a>, Vec<Output>) {
    let (next, outputs) = transition(state, event, context);
    (next, outputs.as_slice().to_vec())
}

pub fn drive<'a>(events: &[Event<'_>], context: &ControlContext<'a>) -> (State<'a>, Vec<Output>) {
    let mut state = State::INITIAL;
    let mut all = Vec::new();
    for event in events {
        let (next, outputs) = step(state, *event, context);
        state = next;
        all.extend(outputs);
    }
    (state, all)
}

pub fn sent_packets(outputs: &[Output]) -> Vec<Vec<u8>> {
    outputs
        .iter()
        .filter_map(|output| match output {
            Output::Send(packet) => Some(packet.bytes().to_vec()),
            Output::ReleaseStall | Output::Receive | Output::Stall | Output::Effect(_) => None,
        })
        .collect()
}

pub struct InTransfer {
    pub packets: Vec<Vec<u8>>,
    pub status_armed: bool,
    pub final_state: &'static str,
}

pub fn read_until_short_packet(setup: [u8; 8], context: &ControlContext<'_>) -> InTransfer {
    let (mut state, outputs) = step(State::INITIAL, Event::Setup(setup), context);
    let status_armed = outputs.contains(&Output::Receive);
    let mut packets = sent_packets(&outputs);
    while let State::SendingReply(_) = state {
        let (next, outputs) = step(state, Event::InSent, context);
        packets.extend(sent_packets(&outputs));
        state = next;
    }
    InTransfer {
        packets,
        status_armed,
        final_state: state_name(&state),
    }
}
