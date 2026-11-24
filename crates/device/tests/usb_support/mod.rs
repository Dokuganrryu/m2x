#![allow(dead_code)]

use device::rate_change;
use device::usb::answer::{Answer, ControlContext, Descriptors, Effect, answer};
use device::usb::function::{Event, Output, State, transition};
use device::usb::request::{Address, DeviceAddress, LineCoding};
use device::usb::{BENCH_DEVICE_DESCRIPTOR, BENCH_STRINGS, BusSpeed, FACTORY_SERIAL_TEMPLATE};
use dsp::SampleRate;

pub struct XorShift(pub u64);

impl XorShift {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    pub fn byte(&mut self) -> u8 {
        self.next() as u8
    }
}

pub fn step(state: State, event: Event) -> (State, Vec<Output>) {
    let (next, outputs) = transition(state, event);
    (next, outputs.as_slice().to_vec())
}

pub fn run(events: &[Event]) -> State {
    events
        .iter()
        .fold(State::INITIAL, |state, event| step(state, *event).0)
}

pub fn assigned(address: u8) -> Address {
    let packet = setup(0x00, 0x05, u16::from(address), 0, 0);
    match device::usb::request::ControlRequest::decode(packet, &[], BusSpeed::High) {
        Ok(device::usb::request::ControlRequest::Write(
            device::usb::request::Writable::Address(address),
        )) => address,
        other => panic!("SET_ADDRESS {address} decodes to {other:?}"),
    }
}

pub fn address_value(address: Address) -> u8 {
    match address {
        Address::Default => 0,
        Address::Assigned(assigned) => DeviceAddress::value(assigned),
    }
}

pub const fn setup(request_type: u8, request: u8, value: u16, index: u16, length: u16) -> [u8; 8] {
    let [value_low, value_high] = value.to_le_bytes();
    let [index_low, index_high] = index.to_le_bytes();
    let [length_low, length_high] = length.to_le_bytes();
    [
        request_type,
        request,
        value_low,
        value_high,
        index_low,
        index_high,
        length_low,
        length_high,
    ]
}

pub const STEADY_48K: rate_change::State = rate_change::State::Steady {
    rate: SampleRate::Hz48000,
    adc: device::adc::AdcKind::Unanswered,
};

pub fn context(function: State) -> ControlContext<'static> {
    context_with_rate(function, STEADY_48K)
}

pub fn context_with_rate(function: State, rate: rate_change::State) -> ControlContext<'static> {
    ControlContext {
        function,
        rate: rate.report(),
        line_coding: LineCoding::CONSOLE_DEFAULT,
        descriptors: Descriptors {
            device: &BENCH_DEVICE_DESCRIPTOR,
            strings: &BENCH_STRINGS,
            serial: FACTORY_SERIAL_TEMPLATE,
        },
    }
}

pub fn reply(packet: [u8; 8], context: &ControlContext<'static>) -> Vec<u8> {
    match answer(packet, &[], context) {
        Answer::Reply(reply) => reply.bytes().to_vec(),
        other => panic!("{packet:02X?} answered {other:?}"),
    }
}

pub fn effect(packet: [u8; 8], data: &[u8], context: &ControlContext<'static>) -> Effect {
    match answer(packet, data, context) {
        Answer::Accept(effect) => effect,
        other => panic!("{packet:02X?} answered {other:?}"),
    }
}

pub fn stalls(packet: [u8; 8], data: &[u8], context: &ControlContext<'static>) -> bool {
    matches!(answer(packet, data, context), Answer::Stall)
}

pub fn enumerated_at(speed: BusSpeed) -> State {
    run(&[
        Event::Attached,
        Event::BusReset(speed),
        Event::AddressSet(assigned(7)),
        Event::ConfigurationSet,
    ])
}
