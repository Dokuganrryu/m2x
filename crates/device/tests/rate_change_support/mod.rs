#![allow(dead_code)]

use device::adc::{AdcAddress, AdcKind};
use device::phantom::Milliseconds;
use device::rate_change::{Event, Output, State, transition};

pub const ADC_KINDS: [AdcKind; 3] = [
    AdcKind::Unanswered,
    AdcKind::Answered(AdcAddress::At13),
    AdcKind::Answered(AdcAddress::At4F),
];

pub const ELAPSED_STEP: Milliseconds = Milliseconds(10);

pub fn step(state: State, event: Event) -> (State, Vec<Output>) {
    let (next, outputs) = transition(state, event);
    (next, outputs.as_slice().to_vec())
}

pub fn completion_for(state: State) -> Option<Event> {
    match state {
        State::Steady { .. } | State::Failed { .. } => None,
        State::Coalescing { .. }
        | State::HoldingSpeedBits { .. }
        | State::StabilisingPll { .. }
        | State::Settling { .. } => Some(Event::Elapsed(ELAPSED_STEP)),
        State::FadingOut { .. } | State::FadingIn { .. } => Some(Event::Ticked),
        State::Attenuating { .. } => Some(Event::I2cAcked),
        State::Stopping { .. }
        | State::SettingSpeedBits { .. }
        | State::EnablingAdc { .. }
        | State::SwitchingFamily { .. }
        | State::Reconfiguring { .. }
        | State::Restarting { .. } => Some(Event::StepDone),
    }
}

pub fn run_until_settled(mut state: State, trace: &mut Vec<(Event, State, Vec<Output>)>) -> State {
    for _ in 0..10_000 {
        let Some(event) = completion_for(state) else {
            return state;
        };
        let (next, outputs) = step(state, event);
        trace.push((event, next, outputs));
        state = next;
    }
    panic!("the machine did not settle: {state:?}");
}
