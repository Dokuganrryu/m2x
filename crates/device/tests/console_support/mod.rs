#![allow(dead_code)]

use device::console::{Event, Output, State, transition};

pub fn feed(state: State, bytes: &[u8]) -> (State, Vec<Output>) {
    let mut state = state;
    let mut outputs = Vec::new();
    for &byte in bytes {
        let (next, emitted) = transition(state, Event::Byte(byte));
        outputs.extend_from_slice(emitted.as_slice());
        state = next;
    }
    (state, outputs)
}

pub fn opened() -> State {
    transition(State::Closed, Event::PortOpened).0
}

pub fn typed(bytes: &[u8]) -> Vec<Output> {
    feed(opened(), bytes).1
}
