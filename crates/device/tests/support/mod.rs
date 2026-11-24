#![allow(dead_code)]

use device::analog_word::{AnalogWord, PreampCode};
use device::input::Input;
use device::phantom::{Event, Output, PinLevel, State, transition};
use dsp::FramesPerTick;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seen {
    Mute(Input),
    Unmute(Input),
    Word(u16),
    Pin(PinLevel),
}

pub fn seen(actions: &[Output]) -> Vec<Seen> {
    actions
        .iter()
        .map(|action| match *action {
            Output::MuteInput(input) => Seen::Mute(input),
            Output::UnmuteInput(input) => Seen::Unmute(input),
            Output::SendAnalogWord(grant) => Seen::Word(
                AnalogWord::new(
                    PreampCode::MINIMUM,
                    PreampCode::MINIMUM,
                    FramesPerTick::Six,
                    grant,
                )
                .bits(),
            ),
            Output::DrivePhantomActivePin(level) => Seen::Pin(level),
        })
        .collect()
}

pub fn run(events: &[Event]) -> (State, Vec<Seen>) {
    let mut state = State::Disarmed;
    let mut all = Vec::new();
    for event in events {
        let (next, actions) = transition(state, *event);
        all.extend(seen(actions.as_slice()));
        state = next;
    }
    (state, all)
}

pub fn step(state: State, event: Event) -> (State, Vec<Seen>) {
    let (next, actions) = transition(state, event);
    (next, seen(actions.as_slice()))
}
