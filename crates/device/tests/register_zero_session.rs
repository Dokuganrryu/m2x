#![cfg(feature = "register-zero-protocol")]

mod register_zero_support;

use device::breadcrumb::At;
use device::command::Command;
use device::dac::{Attenuation, DacWrite, VolumeRegister};
use device::factory_init::{FACTORY_DAC_INIT, FactoryStep};
use device::fault::{FaultCode, I2cPeer};
use device::input::Input;
use device::phantom::Milliseconds;
use device::register_zero::{
    ARMING_STEP_TIMEOUT, BoardFact, BoardRecord, ConfirmationNonce, DisarmCause, Event, Output,
    State, TransmitBuffers, VolumeEvidence, register_zero_factory_bus_bytes, transition,
};
use device::safe_init::SAFE_DAC_INIT;
use device::steps::ScriptStep;
use device::supervisor;
use register_zero_support::{
    NONCE, clean_record, commands, locked, phantom_requested, register_zero_writes,
    replay_with_a_willing_board, walk_to_nonce,
};

fn run(events: &[Event]) -> (State, Vec<Output>) {
    let mut state = locked();
    let mut all = Vec::new();
    for &event in events {
        let (next, outputs) = transition(state, event);
        all.extend_from_slice(outputs.as_slice());
        state = next;
    }
    (state, all)
}

fn full_walk(
    at_request: BoardRecord,
    typed: ConfirmationNonce,
    at_confirm: BoardRecord,
) -> [Event; 5] {
    [
        Event::ProtocolSigned,
        Event::SpareConfirmed,
        Event::OutputsUnplugged,
        Event::ReplayRequested(at_request),
        Event::NonceTyped(typed, at_confirm),
    ]
}

fn clean_walk() -> [Event; 5] {
    full_walk(clean_record(), NONCE, clean_record())
}

fn wrong_nonce() -> ConfirmationNonce {
    ConfirmationNonce::new(123_456).expect("a six digit nonce")
}

fn replaying() -> State {
    let issued = walk_to_nonce(locked());
    transition(issued, Event::NonceTyped(NONCE, clean_record())).0
}

fn fully_attenuated_volume_writes() -> Vec<Command> {
    VolumeRegister::ALL
        .iter()
        .map(|&register| Command::WriteDac(DacWrite::Volume(register, Attenuation::FULL)))
        .collect()
}

#[test]
fn a_full_walk_with_the_typed_back_nonce_emits_the_factory_register_zero_write_once() {
    let issued = walk_to_nonce(locked());
    let (state, outputs) = transition(issued, Event::NonceTyped(NONCE, clean_record()));
    let writes: Vec<_> = outputs
        .as_slice()
        .iter()
        .filter_map(|output| match output {
            Output::WriteRegisterZero(write) => Some(register_zero_factory_bus_bytes(*write)),
            _ => None,
        })
        .collect();
    assert_eq!(writes, [[0x00, 0x00]]);
    assert!(matches!(state, State::Replaying(_)));
}

#[test]
fn the_sealed_write_sends_exactly_the_one_register_zero_write_of_the_factory_init() {
    let factory: Vec<[u8; 2]> = FACTORY_DAC_INIT
        .iter()
        .filter_map(|step| match *step {
            FactoryStep::Write { register: 0, value } => Some([0, value]),
            FactoryStep::Write { .. }
            | FactoryStep::Read { .. }
            | FactoryStep::Wait(_)
            | FactoryStep::DrivePin { .. } => None,
        })
        .collect();
    let issued = walk_to_nonce(locked());
    let (_, outputs) = transition(issued, Event::NonceTyped(NONCE, clean_record()));
    let sent: Vec<[u8; 2]> = outputs
        .as_slice()
        .iter()
        .filter_map(|output| match output {
            Output::WriteRegisterZero(write) => Some(register_zero_factory_bus_bytes(*write)),
            _ => None,
        })
        .collect();
    assert_eq!(sent, factory);
}

#[test]
fn the_request_step_prints_the_boot_nonce_for_the_operator() {
    let (state, outputs) = run(&clean_walk()[..4]);
    assert_eq!(outputs, [Output::ShowNonce(NONCE)]);
    assert!(matches!(state, State::NonceIssued(_)));
}

#[test]
fn each_d9_precondition_missing_alone_blocks_the_replay() {
    let walk = clean_walk();
    let without = |step: usize| -> Vec<Event> {
        walk.iter()
            .enumerate()
            .filter(|&(index, _)| index != step)
            .map(|(_, event)| *event)
            .collect()
    };
    let at_request = |record: BoardRecord| full_walk(record, NONCE, clean_record()).to_vec();
    let cases: Vec<(&str, Vec<Event>, DisarmCause)> = vec![
        ("1 protocol not signed", without(0), DisarmCause::OutOfOrder),
        ("2 spare not confirmed", without(1), DisarmCause::OutOfOrder),
        (
            "3 outputs not unplugged",
            without(2),
            DisarmCause::OutOfOrder,
        ),
        (
            "4 volume not confirmed at full attenuation",
            at_request(BoardRecord {
                volume: VolumeEvidence::Unconfirmed,
                ..clean_record()
            }),
            DisarmCause::Unmet(BoardFact::VolumeFullyAttenuated),
        ),
        (
            "5 transmit buffers live",
            at_request(BoardRecord {
                transmit: TransmitBuffers::Live,
                ..clean_record()
            }),
            DisarmCause::Unmet(BoardFact::TransmitSilent),
        ),
        (
            "5 phantom one requested",
            at_request(BoardRecord {
                phantom: phantom_requested(Input::One),
                ..clean_record()
            }),
            DisarmCause::Unmet(BoardFact::PhantomRequestsClear),
        ),
        (
            "5 phantom two requested",
            at_request(BoardRecord {
                phantom: phantom_requested(Input::Two),
                ..clean_record()
            }),
            DisarmCause::Unmet(BoardFact::PhantomRequestsClear),
        ),
        (
            "6 nonce typed back wrong",
            full_walk(clean_record(), wrong_nonce(), clean_record()).to_vec(),
            DisarmCause::WrongNonce,
        ),
    ];
    for (name, events, cause) in cases {
        let (state, outputs) = run(&events);
        assert_eq!(register_zero_writes(&outputs), 0, "{name}");
        assert_eq!(state, locked(), "{name}");
        assert!(
            outputs.contains(&Output::Disarmed(cause)),
            "{name}: {outputs:?}"
        );
    }
}

#[test]
fn a_board_fact_lost_between_request_and_confirm_blocks_the_replay() {
    for (record, fact) in [
        (
            BoardRecord {
                volume: VolumeEvidence::Unconfirmed,
                ..clean_record()
            },
            BoardFact::VolumeFullyAttenuated,
        ),
        (
            BoardRecord {
                transmit: TransmitBuffers::Live,
                ..clean_record()
            },
            BoardFact::TransmitSilent,
        ),
        (
            BoardRecord {
                phantom: phantom_requested(Input::Two),
                ..clean_record()
            },
            BoardFact::PhantomRequestsClear,
        ),
    ] {
        let (state, outputs) = run(&full_walk(clean_record(), NONCE, record));
        assert_eq!(register_zero_writes(&outputs), 0);
        assert_eq!(state, locked());
        assert_eq!(
            outputs.last(),
            Some(&Output::Disarmed(DisarmCause::Unmet(fact)))
        );
    }
}

#[test]
fn abort_port_close_fault_lost_fact_timeout_and_stray_input_each_disarm_every_arming_step() {
    let walk = clean_walk();
    let interruptions = [
        (Event::Aborted, DisarmCause::Aborted),
        (Event::PortClosed, DisarmCause::PortClosed),
        (
            Event::FaultRaised(FaultCode::BoardStalled),
            DisarmCause::FaultRaised,
        ),
        (
            Event::PreconditionLost(BoardFact::TransmitSilent),
            DisarmCause::Unmet(BoardFact::TransmitSilent),
        ),
        (Event::Elapsed(ARMING_STEP_TIMEOUT), DisarmCause::TimedOut),
        (Event::I2cAcked, DisarmCause::OutOfOrder),
        (Event::ProtocolSigned, DisarmCause::OutOfOrder),
    ];
    for reached in 1..walk.len() {
        for (interruption, cause) in interruptions {
            let mut events = walk[..reached].to_vec();
            events.push(interruption);
            let (state, outputs) = run(&events);
            assert_eq!(state, locked(), "after {reached} steps, {interruption:?}");
            assert_eq!(outputs.last(), Some(&Output::Disarmed(cause)));
            events.extend_from_slice(&walk[reached..]);
            let (_, outputs) = run(&events);
            assert_eq!(
                register_zero_writes(&outputs),
                0,
                "the walk does not resume"
            );
        }
    }
}

#[test]
fn time_inside_the_step_window_keeps_the_step_and_the_window_restarts_at_each_step() {
    let short = Event::Elapsed(Milliseconds(ARMING_STEP_TIMEOUT.0 - 1));
    let (_, outputs) = run(&[
        Event::ProtocolSigned,
        short,
        Event::SpareConfirmed,
        short,
        Event::OutputsUnplugged,
        short,
        Event::ReplayRequested(clean_record()),
        short,
        Event::NonceTyped(NONCE, clean_record()),
    ]);
    assert_eq!(register_zero_writes(&outputs), 1);
}

#[test]
fn after_a_disarm_a_fresh_walk_from_the_start_can_still_arm() {
    let mut events = vec![Event::ProtocolSigned, Event::Aborted];
    events.extend(clean_walk());
    let (_, outputs) = run(&events);
    assert_eq!(register_zero_writes(&outputs), 1);
}

#[test]
fn after_the_replay_registers_16_to_23_go_to_full_attenuation_then_b8_reruns_and_the_session_holds()
{
    let issued = walk_to_nonce(locked());
    let (state, outputs) =
        replay_with_a_willing_board(issued, Event::NonceTyped(NONCE, clean_record()));
    let mut expected = fully_attenuated_volume_writes();
    expected.extend(SAFE_DAC_INIT.iter().filter_map(|step| match *step {
        ScriptStep::Emit(command) => Some(command),
        ScriptStep::Hold(_) => None,
    }));
    assert_eq!(commands(&outputs), expected);
    assert_eq!(register_zero_writes(&outputs), 1);
    assert_eq!(state, State::HeldAttenuated);
    assert!(state.forbids_volume_changes());
}

#[test]
fn the_held_state_absorbs_every_event_and_emits_nothing() {
    let issued = walk_to_nonce(locked());
    let (held, _) = replay_with_a_willing_board(issued, Event::NonceTyped(NONCE, clean_record()));
    for event in clean_walk().into_iter().chain([
        Event::Aborted,
        Event::PortClosed,
        Event::I2cAcked,
        Event::Elapsed(ARMING_STEP_TIMEOUT),
        Event::FaultRaised(FaultCode::BoardStalled),
    ]) {
        let (next, outputs) = transition(held, event);
        assert_eq!(next, State::HeldAttenuated);
        assert_eq!(outputs.as_slice(), []);
    }
}

#[test]
fn a_nack_of_the_register_zero_write_fails_without_a_second_attempt_and_attenuates() {
    let (state, outputs) = transition(replaying(), Event::I2cNacked);
    let code = FaultCode::Unanswered(I2cPeer::Dac);
    assert_eq!(state, State::Failed(code));
    assert_eq!(register_zero_writes(outputs.as_slice()), 0);
    assert_eq!(
        outputs.as_slice()[0],
        Output::Supervisor(supervisor::Event::FaultRaised(code, At::Here))
    );
    assert_eq!(
        commands(outputs.as_slice()),
        fully_attenuated_volume_writes()
    );
}

#[test]
fn a_restore_write_is_retried_up_to_the_transfer_limit_then_fails_attenuated() {
    let (mut state, _) = transition(replaying(), Event::I2cAcked);
    let first = fully_attenuated_volume_writes()[0];
    for _ in 0..3 {
        let (next, outputs) = transition(state, Event::I2cNacked);
        assert_eq!(commands(outputs.as_slice()), [first]);
        state = next;
    }
    let (state, outputs) = transition(state, Event::I2cNacked);
    assert_eq!(state, State::Failed(FaultCode::Unanswered(I2cPeer::Dac)));
    assert_eq!(
        commands(outputs.as_slice()),
        fully_attenuated_volume_writes()
    );
}

#[test]
fn port_close_and_operator_input_during_the_replay_do_not_stop_the_restore() {
    let replaying = replaying();
    for event in [Event::PortClosed, Event::Aborted, Event::ProtocolSigned] {
        let (next, outputs) = transition(replaying, event);
        assert_eq!(next, replaying);
        assert_eq!(outputs.as_slice(), []);
    }
}

#[test]
fn a_fault_from_elsewhere_during_the_replay_ends_failed_and_attenuated() {
    let (state, outputs) = transition(replaying(), Event::FaultRaised(FaultCode::BoardStalled));
    assert_eq!(state, State::Failed(FaultCode::BoardStalled));
    assert_eq!(
        commands(outputs.as_slice()),
        fully_attenuated_volume_writes()
    );
}

#[test]
fn the_nonce_prints_as_six_ascii_digits_with_leading_zeros() {
    assert_eq!(
        ConfirmationNonce::from_boot_entropy(4_213).digits(),
        *b"004213"
    );
    assert_eq!(
        ConfirmationNonce::from_boot_entropy(1_424_242).digits(),
        *b"424242"
    );
    assert_eq!(ConfirmationNonce::new(1_000_000), None);
}
