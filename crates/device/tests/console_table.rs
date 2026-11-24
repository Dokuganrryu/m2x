mod console_support;

use console_support::{feed, opened};
use device::console::{DiscardCause, Event, LINE_CAPACITY, Output, Refusal, State, transition};

fn state_label(state: State) -> String {
    match state {
        State::Closed => "closed".into(),
        State::Collecting(line) => format!("collecting {}", line.len()),
        State::Discarding(DiscardCause::LineTooLong) => "discarding long".into(),
        State::Discarding(DiscardCause::NotPrintableAscii) => "discarding ascii".into(),
    }
}

fn event_label(event: Event) -> String {
    match event {
        Event::PortOpened => "open".into(),
        Event::PortClosed => "close".into(),
        Event::Byte(b'\n') => "lf".into(),
        Event::Byte(b'\r') => "cr".into(),
        Event::Byte(byte) if byte.is_ascii_graphic() || byte == b' ' => {
            format!("{:?}", char::from(byte))
        }
        Event::Byte(byte) => format!("{byte:#04x}"),
    }
}

fn refusal_label(refusal: Refusal) -> &'static str {
    match refusal {
        Refusal::NotAllowlisted => "not allowlisted",
        Refusal::MalformedArguments => "malformed",
        Refusal::SettingOutOfRange => "out of range",
        Refusal::LineTooLong => "too long",
        Refusal::NotPrintableAscii => "not ascii",
        Refusal::NotRunning => "not running",
    }
}

fn outputs_label(outputs: &[Output]) -> String {
    match outputs {
        [] => "-".into(),
        [Output::Execute(command)] => format!("execute {command:?}"),
        [Output::Reject(refusal)] => format!("reject {}", refusal_label(*refusal)),
        more => panic!("more than one output {more:?}"),
    }
}

fn every_state_shape() -> Vec<State> {
    let full = vec![b'x'; LINE_CAPACITY];
    let shapes = vec![
        State::Closed,
        opened(),
        feed(opened(), b"status").0,
        feed(opened(), &full).0,
        feed(opened(), &[full.as_slice(), b"x"].concat()).0,
        feed(opened(), &[0xFF]).0,
    ];
    for shape in &shapes {
        match shape {
            State::Closed | State::Collecting(_) | State::Discarding(_) => {}
        }
    }
    shapes
}

fn every_event_shape() -> Vec<Event> {
    let shapes = vec![
        Event::PortOpened,
        Event::PortClosed,
        Event::Byte(b's'),
        Event::Byte(b' '),
        Event::Byte(b'\n'),
        Event::Byte(b'\r'),
        Event::Byte(0x80),
        Event::Byte(0x00),
    ];
    for shape in &shapes {
        match shape {
            Event::PortOpened | Event::PortClosed | Event::Byte(_) => {}
        }
    }
    shapes
}

const TABLE: &str = "
closed | open | collecting 0 | -
closed | close | closed | -
closed | 's' | closed | -
closed | ' ' | closed | -
closed | lf | closed | -
closed | cr | closed | -
closed | 0x80 | closed | -
closed | 0x00 | closed | -
collecting 0 | open | collecting 0 | -
collecting 0 | close | closed | -
collecting 0 | 's' | collecting 1 | -
collecting 0 | ' ' | collecting 1 | -
collecting 0 | lf | collecting 0 | -
collecting 0 | cr | collecting 0 | -
collecting 0 | 0x80 | discarding ascii | -
collecting 0 | 0x00 | discarding ascii | -
collecting 6 | open | collecting 0 | -
collecting 6 | close | closed | -
collecting 6 | 's' | collecting 7 | -
collecting 6 | ' ' | collecting 7 | -
collecting 6 | lf | collecting 0 | execute Report(Status)
collecting 6 | cr | collecting 0 | execute Report(Status)
collecting 6 | 0x80 | discarding ascii | -
collecting 6 | 0x00 | discarding ascii | -
collecting 64 | open | collecting 0 | -
collecting 64 | close | closed | -
collecting 64 | 's' | discarding long | -
collecting 64 | ' ' | discarding long | -
collecting 64 | lf | collecting 0 | reject not allowlisted
collecting 64 | cr | collecting 0 | reject not allowlisted
collecting 64 | 0x80 | discarding ascii | -
collecting 64 | 0x00 | discarding ascii | -
discarding long | open | collecting 0 | -
discarding long | close | closed | -
discarding long | 's' | discarding long | -
discarding long | ' ' | discarding long | -
discarding long | lf | collecting 0 | reject too long
discarding long | cr | collecting 0 | reject too long
discarding long | 0x80 | discarding long | -
discarding long | 0x00 | discarding long | -
discarding ascii | open | collecting 0 | -
discarding ascii | close | closed | -
discarding ascii | 's' | discarding ascii | -
discarding ascii | ' ' | discarding ascii | -
discarding ascii | lf | collecting 0 | reject not ascii
discarding ascii | cr | collecting 0 | reject not ascii
discarding ascii | 0x80 | discarding ascii | -
discarding ascii | 0x00 | discarding ascii | -
";

#[test]
fn every_state_and_event_pair_has_its_pinned_next_state_and_output() {
    let mut walked = Vec::new();
    for state in every_state_shape() {
        for event in every_event_shape() {
            let (next, outputs) = transition(state, event);
            walked.push(format!(
                "{} | {} | {} | {}",
                state_label(state),
                event_label(event),
                state_label(next),
                outputs_label(outputs.as_slice())
            ));
        }
    }
    let pinned: Vec<&str> = TABLE.trim().lines().collect();
    assert_eq!(walked, pinned);
}

#[test]
fn the_table_walk_reaches_every_state_variant_and_discard_cause() {
    let labels: Vec<String> = every_state_shape().into_iter().map(state_label).collect();
    for expected in [
        "closed",
        "collecting 0",
        "discarding long",
        "discarding ascii",
    ] {
        assert!(labels.iter().any(|label| label == expected), "{expected}");
    }
}
