#![allow(dead_code)]

use device::command::Command;
use device::input::Input;
use device::phantom::{self, Milliseconds, PhantomGrant, PhantomPower};
use device::register_zero::{
    BoardRecord, ConfirmationNonce, Event, Output, State, TransmitBuffers, VolumeEvidence,
    transition,
};

pub const NONCE: ConfirmationNonce = ConfirmationNonce::from_boot_entropy(424_242);
pub const BOARD_HOLD_STEP: Milliseconds = Milliseconds(100);

pub fn locked() -> State {
    State::locked(NONCE)
}

pub fn clean_record() -> BoardRecord {
    BoardRecord {
        volume: VolumeEvidence::FullyAttenuatedAndAckedThisBoot,
        transmit: TransmitBuffers::Zeroed,
        phantom: PhantomGrant::NONE,
    }
}

pub fn walk_to_nonce(state: State) -> State {
    [
        Event::ProtocolSigned,
        Event::SpareConfirmed,
        Event::OutputsUnplugged,
        Event::ReplayRequested(clean_record()),
    ]
    .into_iter()
    .fold(state, |state, event| transition(state, event).0)
}

pub fn phantom_requested(input: Input) -> PhantomGrant {
    let state = [
        phantom::Event::Boot,
        phantom::Event::BootReachedRunning,
        phantom::Event::JackRemoved(input),
        phantom::Event::ButtonPressed(input),
        phantom::Event::Elapsed(Milliseconds(100)),
        phantom::Event::ButtonReleased(input),
    ]
    .into_iter()
    .fold(phantom::State::Disarmed, |state, event| {
        phantom::transition(state, event).0
    });
    let grant = state.grant();
    assert_eq!(
        grant.power(input),
        PhantomPower::On,
        "the helper raised phantom"
    );
    grant
}

pub fn board_answer(outputs: &[Output]) -> Event {
    match outputs.last() {
        Some(Output::WriteRegisterZero(_) | Output::Command(Command::WriteDac(_))) => {
            Event::I2cAcked
        }
        Some(Output::Command(Command::ReadDac(_))) => Event::StatusRead(0x00),
        _ => Event::Elapsed(BOARD_HOLD_STEP),
    }
}

pub fn replay_with_a_willing_board(state: State, first: Event) -> (State, Vec<Output>) {
    let mut all = Vec::new();
    let mut state = state;
    let mut event = first;
    for _ in 0..1_000 {
        let (next, outputs) = transition(state, event);
        all.extend_from_slice(outputs.as_slice());
        state = next;
        if !matches!(state, State::Replaying(_)) {
            break;
        }
        event = board_answer(outputs.as_slice());
    }
    (state, all)
}

pub fn register_zero_writes(outputs: &[Output]) -> usize {
    outputs
        .iter()
        .filter(|output| matches!(output, Output::WriteRegisterZero(_)))
        .count()
}

pub fn commands(outputs: &[Output]) -> Vec<Command> {
    outputs
        .iter()
        .filter_map(|output| match output {
            Output::Command(command) => Some(*command),
            _ => None,
        })
        .collect()
}
