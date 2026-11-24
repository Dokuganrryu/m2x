#![cfg(feature = "register-zero-protocol")]

mod register_zero_support;

use device::command::Command;
use device::dac::DacWrite;
use device::fault::FaultCode;
use device::input::Input;
use device::phantom::Milliseconds;
use device::register_zero::{
    ARMING_STEP_TIMEOUT, BoardFact, BoardRecord, Checklist, ConfirmationNonce, DisarmCause, Event,
    Output, State, TransmitBuffers, transition,
};
use device::supervisor;
use register_zero_support::{
    NONCE, board_answer, clean_record, locked, phantom_requested, walk_to_nonce,
};

const TABLE: &str = include_str!("register_zero_table.txt");

fn compact(debug: String) -> String {
    debug
        .replace("ConfirmationNonce(424242)", "boot-nonce")
        .replace("Milliseconds", "ms")
        .replace("Attempt", "attempt")
        .replace("Arming { nonce: boot-nonce, waited: ", "{waited ")
        .replace("Replay { ", "{ ")
}

fn state_label(state: State) -> String {
    compact(match state {
        State::Locked(nonce) => format!("locked {nonce:?}"),
        State::Arming(checklist, arming) => {
            let step = match checklist {
                Checklist::Signed => "signed",
                Checklist::OnSpare => "on-spare",
                Checklist::Unplugged => "unplugged",
            };
            format!("arming {step} {arming:?}")
        }
        State::NonceIssued(arming) => format!("nonce-issued {arming:?}"),
        State::Replaying(replay) => format!("replaying {replay:?}"),
        State::HeldAttenuated => "held".into(),
        State::Failed(code) => format!("failed {}", code.number()),
    })
}

fn record_label(record: BoardRecord) -> &'static str {
    if record == clean_record() {
        "clean"
    } else if record.transmit == TransmitBuffers::Live {
        "tx-live"
    } else {
        "phantom"
    }
}

fn event_label(event: Event) -> String {
    match event {
        Event::ProtocolSigned => "signed".into(),
        Event::SpareConfirmed => "spare".into(),
        Event::OutputsUnplugged => "unplugged".into(),
        Event::ReplayRequested(record) => format!("request {}", record_label(record)),
        Event::NonceTyped(nonce, record) => {
            let which = if nonce == NONCE { "right" } else { "wrong" };
            format!("nonce {which} {}", record_label(record))
        }
        Event::Aborted => "abort".into(),
        Event::PortClosed => "port-closed".into(),
        Event::PreconditionLost(fact) => format!("lost {fact:?}"),
        Event::FaultRaised(code) => format!("fault {}", code.number()),
        Event::I2cAcked => "ack".into(),
        Event::I2cNacked => "nack".into(),
        Event::StatusRead(byte) => format!("status {byte}"),
        Event::Elapsed(Milliseconds(ms)) => format!("elapsed {ms}"),
    }
}

fn output_label(output: Output) -> String {
    match output {
        Output::ShowNonce(nonce) => format!("show {}", String::from_utf8_lossy(&nonce.digits())),
        Output::Disarmed(cause) => match cause {
            DisarmCause::OutOfOrder
            | DisarmCause::TimedOut
            | DisarmCause::Aborted
            | DisarmCause::PortClosed
            | DisarmCause::FaultRaised
            | DisarmCause::WrongNonce
            | DisarmCause::Unmet(_) => format!("disarmed {cause:?}"),
        },
        Output::WriteRegisterZero(_) => "WRITE-REGISTER-0".into(),
        Output::Command(Command::WriteDac(DacWrite::Volume(register, level))) => {
            format!("vol{}={:#04x}", register as u8, level.half_decibel_steps())
        }
        Output::Command(command) => format!("{command:?}"),
        Output::Log(record) => format!("log {record:?}"),
        Output::Supervisor(supervisor::Event::FaultRaised(code, _)) => {
            format!("raise {}", code.number())
        }
        Output::Supervisor(other) => format!("{other:?}"),
    }
}

fn events() -> Vec<Event> {
    let wrong = ConfirmationNonce::new(123_456).expect("a six digit nonce");
    let live = BoardRecord {
        transmit: TransmitBuffers::Live,
        ..clean_record()
    };
    let phantom = BoardRecord {
        phantom: phantom_requested(Input::One),
        ..clean_record()
    };
    vec![
        Event::ProtocolSigned,
        Event::SpareConfirmed,
        Event::OutputsUnplugged,
        Event::ReplayRequested(clean_record()),
        Event::ReplayRequested(phantom),
        Event::NonceTyped(NONCE, clean_record()),
        Event::NonceTyped(wrong, clean_record()),
        Event::NonceTyped(NONCE, live),
        Event::Aborted,
        Event::PortClosed,
        Event::PreconditionLost(BoardFact::TransmitSilent),
        Event::FaultRaised(FaultCode::BoardStalled),
        Event::I2cAcked,
        Event::I2cNacked,
        Event::StatusRead(7),
        Event::Elapsed(Milliseconds(5)),
        Event::Elapsed(ARMING_STEP_TIMEOUT),
    ]
}

fn after(state: State, events: &[Event]) -> State {
    events
        .iter()
        .fold(state, |state, &event| transition(state, event).0)
}

fn replay_until(found: impl Fn(&[Output]) -> bool) -> State {
    let (mut state, outputs) = transition(
        walk_to_nonce(locked()),
        Event::NonceTyped(NONCE, clean_record()),
    );
    let mut answer = board_answer(outputs.as_slice());
    for _ in 0..LONGEST_REPLAY {
        let (next, outputs) = transition(state, answer);
        state = next;
        if found(outputs.as_slice()) {
            return state;
        }
        answer = board_answer(outputs.as_slice());
    }
    panic!("the replay never reached the sought step, ending in {state:?}");
}

const LONGEST_REPLAY: usize = 1_000;

fn states() -> Vec<State> {
    let issued = walk_to_nonce(locked());
    let replay_start = after(issued, &[Event::NonceTyped(NONCE, clean_record())]);
    let first_restore = after(replay_start, &[Event::I2cAcked]);
    let last_attempt = after(
        first_restore,
        &[Event::I2cNacked, Event::I2cNacked, Event::I2cNacked],
    );
    let status_read =
        replay_until(|outputs| matches!(outputs, [Output::Command(Command::ReadDac(_))]));
    let hold = replay_until(|outputs| matches!(outputs, [Output::Log(_)]));
    vec![
        locked(),
        after(locked(), &[Event::ProtocolSigned]),
        after(
            locked(),
            &[Event::ProtocolSigned, Event::Elapsed(Milliseconds(1_000))],
        ),
        after(locked(), &[Event::ProtocolSigned, Event::SpareConfirmed]),
        after(
            locked(),
            &[
                Event::ProtocolSigned,
                Event::SpareConfirmed,
                Event::OutputsUnplugged,
            ],
        ),
        issued,
        replay_start,
        first_restore,
        last_attempt,
        status_read,
        hold,
        State::HeldAttenuated,
        after(replay_start, &[Event::I2cNacked]),
    ]
}

fn kind(state: State) -> &'static str {
    match state {
        State::Locked(_) => "locked",
        State::Arming(Checklist::Signed, _) => "arming signed",
        State::Arming(Checklist::OnSpare, _) => "arming on-spare",
        State::Arming(Checklist::Unplugged, _) => "arming unplugged",
        State::NonceIssued(_) => "nonce-issued",
        State::Replaying(replay) if format!("{replay:?}").contains("wait: Hold") => {
            "replaying hold"
        }
        State::Replaying(_) => "replaying answer",
        State::HeldAttenuated => "held",
        State::Failed(_) => "failed",
    }
}

#[test]
fn the_table_walks_every_kind_of_state_including_a_timed_hold_inside_the_replay() {
    let walked: std::collections::BTreeSet<&str> = states().into_iter().map(kind).collect();
    let every: std::collections::BTreeSet<&str> = [
        "locked",
        "arming signed",
        "arming on-spare",
        "arming unplugged",
        "nonce-issued",
        "replaying hold",
        "replaying answer",
        "held",
        "failed",
    ]
    .into_iter()
    .collect();
    assert_eq!(walked, every);
}

#[test]
fn every_register_zero_state_and_event_pair_has_its_pinned_result() {
    let mut actual = String::new();
    for state in states() {
        for event in events() {
            let (next, outputs) = transition(state, event);
            let produced: Vec<String> = outputs
                .as_slice()
                .iter()
                .map(|output| output_label(*output))
                .collect();
            actual.push_str(&format!(
                "{} | {} => {} | {}\n",
                state_label(state),
                event_label(event),
                state_label(next),
                produced.join(" ")
            ));
        }
    }
    let lines = |text: &str| -> Vec<String> {
        text.lines()
            .map(|line| line.trim_end().to_string())
            .collect()
    };
    assert_eq!(lines(&actual), lines(TABLE), "actual:\n{actual}");
}
