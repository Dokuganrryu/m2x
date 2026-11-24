mod usb_support;

use device::usb::BusSpeed;
use device::usb::function::{Event, Output, State};
use device::usb::request::{HaltableEndpoint, InterfaceSelection};
use stream::AlternateSetting;
use usb_support::{assigned, run, step};

const STATES: [&str; 13] = [
    "detached",
    "powered",
    "default_hs",
    "default_fs",
    "addressed_hs",
    "addressed_fs",
    "configured_hs",
    "streaming_hs",
    "configured_fs",
    "suspended_powered",
    "suspended_addressed_hs",
    "suspended_streaming_hs",
    "suspended_configured_fs",
];

const EVENTS: [&str; 20] = [
    "attach",
    "detach",
    "reset_hs",
    "reset_fs",
    "suspend",
    "resume",
    "address_9",
    "address_0",
    "configure",
    "unconfigure",
    "playback_1",
    "playback_0",
    "capture_1",
    "capture_0",
    "audio_control_0",
    "serial_control_0",
    "serial_data_0",
    "halt_receive",
    "clear_receive",
    "halt_notify",
];

const UNCHANGED: &str = "unchanged";

const CHANGES: &[(&str, &str, &str, &str)] = &[
    ("detached", "attach", "", "powered"),
    ("powered", "detach", "", "detached"),
    ("powered", "reset_hs", "", "default_hs"),
    ("powered", "reset_fs", "", "default_fs"),
    ("powered", "suspend", "F S", "suspended_powered"),
    ("default_hs", "detach", "", "detached"),
    ("default_hs", "reset_fs", "", "default_fs"),
    ("default_hs", "suspend", "F S", "suspended_default_hs"),
    ("default_hs", "address_9", "", "addressed_hs"),
    ("default_hs", "address_0", "", UNCHANGED),
    ("default_fs", "detach", "", "detached"),
    ("default_fs", "reset_hs", "", "default_hs"),
    ("default_fs", "suspend", "F S", "suspended_default_fs"),
    ("default_fs", "address_9", "", "addressed_fs"),
    ("default_fs", "address_0", "", UNCHANGED),
    ("addressed_hs", "detach", "", "detached"),
    ("addressed_hs", "reset_hs", "", "default_hs"),
    ("addressed_hs", "reset_fs", "", "default_fs"),
    ("addressed_hs", "suspend", "F S", "suspended_addressed_hs"),
    ("addressed_hs", "address_9", "", UNCHANGED),
    ("addressed_hs", "address_0", "", "default_hs"),
    ("addressed_hs", "configure", "", "configured_hs"),
    ("addressed_fs", "detach", "", "detached"),
    ("addressed_fs", "reset_hs", "", "default_hs"),
    ("addressed_fs", "reset_fs", "", "default_fs"),
    ("addressed_fs", "suspend", "F S", "suspended_addressed_fs"),
    ("addressed_fs", "address_9", "", UNCHANGED),
    ("addressed_fs", "address_0", "", "default_fs"),
    ("addressed_fs", "configure", "", "configured_fs_clean"),
    ("configured_hs", "detach", "", "detached"),
    ("configured_hs", "reset_hs", "", "default_hs"),
    ("configured_hs", "reset_fs", "", "default_fs"),
    ("configured_hs", "suspend", "F S", "suspended_configured_hs"),
    ("configured_hs", "unconfigure", "", "addressed_hs"),
    ("configured_hs", "playback_1", "P1", "playing_hs"),
    ("configured_hs", "capture_1", "C1", "capturing_hs"),
    ("configured_hs", "halt_receive", "H:rx", "receive_halted_hs"),
    ("configured_hs", "clear_receive", "X:rx", UNCHANGED),
    ("configured_hs", "halt_notify", "H:nt", "notify_halted_hs"),
    ("streaming_hs", "detach", "P0 C0", "detached"),
    ("streaming_hs", "reset_hs", "P0 C0", "default_hs"),
    ("streaming_hs", "reset_fs", "P0 C0", "default_fs"),
    (
        "streaming_hs",
        "suspend",
        "F P0 C0 S",
        "suspended_streaming_hs",
    ),
    ("streaming_hs", "configure", "P0 C0 X:rx", "configured_hs"),
    ("streaming_hs", "unconfigure", "P0 C0 X:rx", "addressed_hs"),
    (
        "streaming_hs",
        "playback_0",
        "P0",
        "streaming_hs_without_playback",
    ),
    (
        "streaming_hs",
        "capture_0",
        "C0",
        "streaming_hs_without_capture",
    ),
    (
        "streaming_hs",
        "serial_data_0",
        "X:rx",
        "streaming_hs_without_halt",
    ),
    (
        "streaming_hs",
        "clear_receive",
        "X:rx",
        "streaming_hs_without_halt",
    ),
    (
        "streaming_hs",
        "halt_notify",
        "H:nt",
        "streaming_hs_with_two_halts",
    ),
    ("configured_fs", "detach", "", "detached"),
    ("configured_fs", "reset_hs", "", "default_hs"),
    ("configured_fs", "reset_fs", "", "default_fs"),
    ("configured_fs", "suspend", "F S", "suspended_configured_fs"),
    ("configured_fs", "configure", "X:nt", "configured_fs_clean"),
    ("configured_fs", "unconfigure", "X:nt", "addressed_fs"),
    (
        "configured_fs",
        "serial_control_0",
        "X:nt",
        "configured_fs_clean",
    ),
    (
        "configured_fs",
        "halt_receive",
        "H:rx",
        "configured_fs_with_two_halts",
    ),
    ("configured_fs", "clear_receive", "X:rx", UNCHANGED),
    ("suspended_powered", "detach", "R W", "detached"),
    ("suspended_powered", "reset_hs", "R W", "default_hs"),
    ("suspended_powered", "reset_fs", "R W", "default_fs"),
    ("suspended_powered", "resume", "R W", "powered"),
    ("suspended_addressed_hs", "detach", "R W", "detached"),
    ("suspended_addressed_hs", "reset_hs", "R W", "default_hs"),
    ("suspended_addressed_hs", "reset_fs", "R W", "default_fs"),
    ("suspended_addressed_hs", "resume", "R W", "addressed_hs"),
    ("suspended_streaming_hs", "detach", "R W", "detached"),
    ("suspended_streaming_hs", "reset_hs", "R W", "default_hs"),
    ("suspended_streaming_hs", "reset_fs", "R W", "default_fs"),
    (
        "suspended_streaming_hs",
        "resume",
        "R W P1 C1",
        "streaming_hs",
    ),
    ("suspended_configured_fs", "detach", "R W", "detached"),
    ("suspended_configured_fs", "reset_hs", "R W", "default_hs"),
    ("suspended_configured_fs", "reset_fs", "R W", "default_fs"),
    ("suspended_configured_fs", "resume", "R W", "configured_fs"),
];

fn select(selection: InterfaceSelection) -> Event {
    Event::InterfaceSelected(selection)
}

fn event(name: &str) -> Event {
    match name {
        "attach" => Event::Attached,
        "detach" => Event::Detached,
        "reset_hs" => Event::BusReset(BusSpeed::High),
        "reset_fs" => Event::BusReset(BusSpeed::Full),
        "suspend" => Event::Suspended,
        "resume" => Event::Resumed,
        "address_9" => Event::AddressSet(assigned(9)),
        "address_0" => Event::AddressSet(assigned(0)),
        "configure" => Event::ConfigurationSet,
        "unconfigure" => Event::ConfigurationCleared,
        "playback_1" => select(InterfaceSelection::Playback(AlternateSetting::One)),
        "playback_0" => select(InterfaceSelection::Playback(AlternateSetting::Zero)),
        "capture_1" => select(InterfaceSelection::Capture(AlternateSetting::One)),
        "capture_0" => select(InterfaceSelection::Capture(AlternateSetting::Zero)),
        "audio_control_0" => select(InterfaceSelection::AudioControl),
        "serial_control_0" => select(InterfaceSelection::SerialControl),
        "serial_data_0" => select(InterfaceSelection::SerialData),
        "halt_receive" => Event::EndpointHalted(HaltableEndpoint::SerialReceive),
        "clear_receive" => Event::EndpointHaltCleared(HaltableEndpoint::SerialReceive),
        "halt_notify" => Event::EndpointHalted(HaltableEndpoint::SerialNotification),
        other => panic!("unknown event {other}"),
    }
}

fn recipe(name: &str) -> Vec<Event> {
    let with = |base: &str, more: &[&str]| {
        let mut events = recipe(base);
        events.extend(more.iter().map(|name| event(name)));
        events
    };
    match name {
        "detached" => vec![],
        "powered" => vec![Event::Attached],
        "default_hs" => with("powered", &["reset_hs"]),
        "default_fs" => with("powered", &["reset_fs"]),
        "addressed_hs" => with("default_hs", &["address_9"]),
        "addressed_fs" => with("default_fs", &["address_9"]),
        "configured_hs" => with("addressed_hs", &["configure"]),
        "streaming_hs" => with(
            "configured_hs",
            &["playback_1", "capture_1", "halt_receive"],
        ),
        "configured_fs_clean" => with("addressed_fs", &["configure"]),
        "configured_fs" => with("configured_fs_clean", &["halt_notify"]),
        "suspended_powered" => with("powered", &["suspend"]),
        "suspended_default_hs" => with("default_hs", &["suspend"]),
        "suspended_default_fs" => with("default_fs", &["suspend"]),
        "suspended_addressed_hs" => with("addressed_hs", &["suspend"]),
        "suspended_addressed_fs" => with("addressed_fs", &["suspend"]),
        "suspended_configured_hs" => with("configured_hs", &["suspend"]),
        "suspended_streaming_hs" => with("streaming_hs", &["suspend"]),
        "suspended_configured_fs" => with("configured_fs", &["suspend"]),
        "playing_hs" => with("configured_hs", &["playback_1"]),
        "capturing_hs" => with("configured_hs", &["capture_1"]),
        "receive_halted_hs" => with("configured_hs", &["halt_receive"]),
        "notify_halted_hs" => with("configured_hs", &["halt_notify"]),
        "streaming_hs_without_playback" => with("configured_hs", &["capture_1", "halt_receive"]),
        "streaming_hs_without_capture" => with("configured_hs", &["playback_1", "halt_receive"]),
        "streaming_hs_without_halt" => with("configured_hs", &["playback_1", "capture_1"]),
        "streaming_hs_with_two_halts" => with("streaming_hs", &["halt_notify"]),
        "configured_fs_with_two_halts" => with("configured_fs", &["halt_receive"]),
        other => panic!("unknown state {other}"),
    }
}

fn state(name: &str) -> State {
    run(&recipe(name))
}

fn token(output: Output) -> String {
    let alternate = |setting| match setting {
        AlternateSetting::Zero => "0",
        AlternateSetting::One => "1",
    };
    let endpoint = |endpoint| match endpoint {
        HaltableEndpoint::SerialNotification => "nt",
        HaltableEndpoint::SerialReceive => "rx",
        HaltableEndpoint::SerialTransmit => "tx",
    };
    match output {
        Output::SelectPlayback(setting) => format!("P{}", alternate(setting)),
        Output::SelectCapture(setting) => format!("C{}", alternate(setting)),
        Output::HaltEndpoint(halted) => format!("H:{}", endpoint(halted)),
        Output::ClearEndpointHalt(cleared) => format!("X:{}", endpoint(cleared)),
        Output::FadeOutputsToSilence => "F".to_string(),
        Output::ReleaseOutputSilence => "R".to_string(),
        Output::SuspendPhantom => "S".to_string(),
        Output::ResumePhantom => "W".to_string(),
    }
}

fn tokens(outputs: &[Output]) -> String {
    outputs
        .iter()
        .map(|output| token(*output))
        .collect::<Vec<_>>()
        .join(" ")
}

fn expected(state_name: &'static str, event_name: &str) -> (&'static str, &'static str) {
    let rows: Vec<_> = CHANGES
        .iter()
        .filter(|(s, e, _, _)| *s == state_name && *e == event_name)
        .collect();
    match rows.as_slice() {
        [] => ("", state_name),
        [(_, _, outputs, UNCHANGED)] => (outputs, state_name),
        [(_, _, outputs, next)] => (outputs, next),
        _ => panic!("{state_name} on {event_name} is tabled twice"),
    }
}

#[test]
fn the_changes_name_only_tabled_states_and_events() {
    for (state_name, event_name, _, _) in CHANGES {
        assert!(STATES.contains(state_name), "{state_name}");
        assert!(EVENTS.contains(event_name), "{event_name}");
        expected(state_name, event_name);
    }
}

#[test]
fn the_representative_states_are_all_distinct() {
    for (i, a) in STATES.iter().enumerate() {
        for b in &STATES[i + 1..] {
            assert_ne!(state(a), state(b), "{a} and {b}");
        }
    }
}

#[test]
fn every_state_and_event_pair_emits_the_tabled_outputs_and_next_state() {
    for state_name in STATES {
        for event_name in EVENTS {
            let (outputs, next) = expected(state_name, event_name);
            let (after, emitted) = step(state(state_name), event(event_name));
            assert_eq!(
                tokens(&emitted),
                outputs,
                "{state_name} on {event_name} emits"
            );
            assert_eq!(
                after,
                state(next),
                "{state_name} on {event_name} lands in {next}"
            );
        }
    }
}

fn state_variant(state: State) -> usize {
    match state {
        State::Detached => 0,
        State::Awake(_) => 1,
        State::Suspended(_) => 2,
    }
}

fn event_variant(event: Event) -> usize {
    match event {
        Event::Attached => 0,
        Event::Detached => 1,
        Event::BusReset(_) => 2,
        Event::Suspended => 3,
        Event::Resumed => 4,
        Event::AddressSet(_) => 5,
        Event::ConfigurationSet => 6,
        Event::ConfigurationCleared => 7,
        Event::InterfaceSelected(_) => 8,
        Event::EndpointHalted(_) => 9,
        Event::EndpointHaltCleared(_) => 10,
    }
}

#[test]
fn the_table_holds_one_value_of_every_state_and_event_variant() {
    let mut states: Vec<usize> = STATES
        .iter()
        .map(|name| state_variant(state(name)))
        .collect();
    states.sort_unstable();
    states.dedup();
    assert_eq!(states, vec![0, 1, 2]);
    let mut events: Vec<usize> = EVENTS
        .iter()
        .map(|name| event_variant(event(name)))
        .collect();
    events.sort_unstable();
    events.dedup();
    assert_eq!(events, (0..=10).collect::<Vec<_>>());
}
