mod support;

use device::command::Command;
use device::input::Input;
use device::output_pin::OutputPin;
use device::phantom::{Event, Milliseconds, PinLevel, State};
use support::{Seen, run, step};

const STATES: [&str; 14] = [
    "disarmed",
    "booting",
    "booting_ready",
    "running_blocked",
    "running_ready",
    "held_short",
    "held_counted",
    "held_too_long",
    "powered_muted",
    "powered_settled",
    "powered_held_counted",
    "unpowered_muted",
    "suspended",
    "suspended_muted",
];

const EVENTS: [&str; 11] = [
    "boot", "running", "press", "release", "jack_in", "jack_out", "tick", "second", "suspend",
    "resume", "fault",
];

const TABLE: &[(&str, &str, &str, &str)] = &[
    ("disarmed", "boot", "W0 L", "booting"),
    ("disarmed", "running", "", "disarmed"),
    ("disarmed", "press", "", "disarmed"),
    ("disarmed", "release", "", "disarmed"),
    ("disarmed", "jack_in", "", "disarmed"),
    ("disarmed", "jack_out", "", "disarmed"),
    ("disarmed", "tick", "", "disarmed"),
    ("disarmed", "second", "", "disarmed"),
    ("disarmed", "suspend", "", "disarmed"),
    ("disarmed", "resume", "", "disarmed"),
    ("booting", "boot", "W0 L", "booting"),
    ("booting", "running", "", "running_blocked"),
    ("booting", "press", "", "booting"),
    ("booting", "release", "", "booting"),
    ("booting", "jack_in", "", "booting"),
    ("booting", "jack_out", "", "booting_ready"),
    ("booting", "tick", "", "booting"),
    ("booting", "second", "", "booting"),
    ("booting", "suspend", "", "booting"),
    ("booting", "resume", "", "booting"),
    ("booting_ready", "boot", "W0 L", "booting"),
    ("booting_ready", "running", "", "running_ready"),
    ("booting_ready", "press", "", "booting_ready"),
    ("booting_ready", "release", "", "booting_ready"),
    ("booting_ready", "jack_in", "", "booting"),
    ("booting_ready", "jack_out", "", "booting_ready"),
    ("booting_ready", "tick", "", "booting_ready"),
    ("booting_ready", "second", "", "booting_ready"),
    ("booting_ready", "suspend", "", "booting_ready"),
    ("booting_ready", "resume", "", "booting_ready"),
    ("running_blocked", "boot", "W0 L", "booting"),
    ("running_blocked", "running", "", "running_blocked"),
    ("running_blocked", "press", "", "unnamed"),
    ("running_blocked", "release", "", "running_blocked"),
    ("running_blocked", "jack_in", "", "running_blocked"),
    ("running_blocked", "jack_out", "", "running_ready"),
    ("running_blocked", "tick", "", "running_blocked"),
    ("running_blocked", "second", "", "running_blocked"),
    ("running_blocked", "suspend", "", "unnamed"),
    ("running_blocked", "resume", "", "running_blocked"),
    ("running_ready", "boot", "W0 L", "booting"),
    ("running_ready", "running", "", "running_ready"),
    ("running_ready", "press", "", "held_short"),
    ("running_ready", "release", "", "running_ready"),
    ("running_ready", "jack_in", "", "running_blocked"),
    ("running_ready", "jack_out", "", "running_ready"),
    ("running_ready", "tick", "", "running_ready"),
    ("running_ready", "second", "", "running_ready"),
    ("running_ready", "suspend", "", "suspended"),
    ("running_ready", "resume", "", "running_ready"),
    ("held_short", "boot", "W0 L", "booting"),
    ("held_short", "running", "", "held_short"),
    ("held_short", "press", "", "held_short"),
    ("held_short", "release", "", "running_ready"),
    ("held_short", "jack_in", "", "unnamed"),
    ("held_short", "jack_out", "", "held_short"),
    ("held_short", "tick", "", "unnamed"),
    ("held_short", "second", "", "unnamed"),
    ("held_short", "suspend", "", "suspended"),
    ("held_short", "resume", "", "held_short"),
    ("held_counted", "boot", "W0 L", "booting"),
    ("held_counted", "running", "", "held_counted"),
    ("held_counted", "press", "", "held_counted"),
    ("held_counted", "release", "M W1 H", "powered_muted"),
    ("held_counted", "jack_in", "", "unnamed"),
    ("held_counted", "jack_out", "", "held_counted"),
    ("held_counted", "tick", "", "unnamed"),
    ("held_counted", "second", "", "held_too_long"),
    ("held_counted", "suspend", "", "suspended"),
    ("held_counted", "resume", "", "held_counted"),
    ("held_too_long", "boot", "W0 L", "booting"),
    ("held_too_long", "running", "", "held_too_long"),
    ("held_too_long", "press", "", "held_too_long"),
    ("held_too_long", "release", "", "running_ready"),
    ("held_too_long", "jack_in", "", "unnamed"),
    ("held_too_long", "jack_out", "", "held_too_long"),
    ("held_too_long", "tick", "", "held_too_long"),
    ("held_too_long", "second", "", "held_too_long"),
    ("held_too_long", "suspend", "", "suspended"),
    ("held_too_long", "resume", "", "held_too_long"),
    ("powered_muted", "boot", "W0 L", "booting"),
    ("powered_muted", "running", "", "powered_muted"),
    ("powered_muted", "press", "", "unnamed"),
    ("powered_muted", "release", "", "powered_muted"),
    ("powered_muted", "jack_in", "M W0 L", "unnamed"),
    ("powered_muted", "jack_out", "", "powered_muted"),
    ("powered_muted", "tick", "", "unnamed"),
    ("powered_muted", "second", "U", "powered_settled"),
    ("powered_muted", "suspend", "M W0 L", "suspended_muted"),
    ("powered_muted", "resume", "", "powered_muted"),
    ("powered_settled", "boot", "W0 L", "booting"),
    ("powered_settled", "running", "", "powered_settled"),
    ("powered_settled", "press", "", "unnamed"),
    ("powered_settled", "release", "", "powered_settled"),
    ("powered_settled", "jack_in", "M W0 L", "unnamed"),
    ("powered_settled", "jack_out", "", "powered_settled"),
    ("powered_settled", "tick", "", "powered_settled"),
    ("powered_settled", "second", "", "powered_settled"),
    ("powered_settled", "suspend", "M W0 L", "suspended_muted"),
    ("powered_settled", "resume", "", "powered_settled"),
    ("powered_held_counted", "boot", "W0 L", "booting"),
    (
        "powered_held_counted",
        "running",
        "",
        "powered_held_counted",
    ),
    ("powered_held_counted", "press", "", "powered_held_counted"),
    (
        "powered_held_counted",
        "release",
        "M W0 L",
        "unpowered_muted",
    ),
    ("powered_held_counted", "jack_in", "M W0 L", "unnamed"),
    (
        "powered_held_counted",
        "jack_out",
        "",
        "powered_held_counted",
    ),
    ("powered_held_counted", "tick", "", "unnamed"),
    ("powered_held_counted", "second", "", "unnamed"),
    (
        "powered_held_counted",
        "suspend",
        "M W0 L",
        "suspended_muted",
    ),
    ("powered_held_counted", "resume", "", "powered_held_counted"),
    ("unpowered_muted", "boot", "W0 L", "booting"),
    ("unpowered_muted", "running", "", "unpowered_muted"),
    ("unpowered_muted", "press", "", "unnamed"),
    ("unpowered_muted", "release", "", "unpowered_muted"),
    ("unpowered_muted", "jack_in", "", "unnamed"),
    ("unpowered_muted", "jack_out", "", "unpowered_muted"),
    ("unpowered_muted", "tick", "", "unnamed"),
    ("unpowered_muted", "second", "U", "running_ready"),
    ("unpowered_muted", "suspend", "", "suspended_muted"),
    ("unpowered_muted", "resume", "", "unpowered_muted"),
    ("suspended", "boot", "W0 L", "booting"),
    ("suspended", "running", "", "suspended"),
    ("suspended", "press", "", "suspended"),
    ("suspended", "release", "", "suspended"),
    ("suspended", "jack_in", "", "unnamed"),
    ("suspended", "jack_out", "", "suspended"),
    ("suspended", "tick", "", "suspended"),
    ("suspended", "second", "", "suspended"),
    ("suspended", "suspend", "", "suspended"),
    ("suspended", "resume", "", "running_ready"),
    ("suspended_muted", "boot", "W0 L", "booting"),
    ("suspended_muted", "running", "", "suspended_muted"),
    ("suspended_muted", "press", "", "suspended_muted"),
    ("suspended_muted", "release", "", "suspended_muted"),
    ("suspended_muted", "jack_in", "", "unnamed"),
    ("suspended_muted", "jack_out", "", "suspended_muted"),
    ("suspended_muted", "tick", "", "unnamed"),
    ("suspended_muted", "second", "U", "suspended"),
    ("suspended_muted", "suspend", "", "suspended_muted"),
    ("suspended_muted", "resume", "", "unpowered_muted"),
    ("disarmed", "fault", "", "disarmed"),
    ("booting", "fault", "W0 L", "disarmed"),
    ("booting_ready", "fault", "W0 L", "disarmed"),
    ("running_blocked", "fault", "W0 L", "disarmed"),
    ("running_ready", "fault", "W0 L", "disarmed"),
    ("held_short", "fault", "W0 L", "disarmed"),
    ("held_counted", "fault", "W0 L", "disarmed"),
    ("held_too_long", "fault", "W0 L", "disarmed"),
    ("powered_muted", "fault", "M W0 L", "disarmed"),
    ("powered_settled", "fault", "M W0 L", "disarmed"),
    ("powered_held_counted", "fault", "M W0 L", "disarmed"),
    ("unpowered_muted", "fault", "W0 L", "disarmed"),
    ("suspended", "fault", "W0 L", "disarmed"),
    ("suspended_muted", "fault", "W0 L", "disarmed"),
];

fn elapsed(ms: u32) -> Event {
    Event::Elapsed(Milliseconds(ms))
}

fn event(name: &str, focus: Input) -> Event {
    match name {
        "boot" => Event::Boot,
        "running" => Event::BootReachedRunning,
        "press" => Event::ButtonPressed(focus),
        "release" => Event::ButtonReleased(focus),
        "jack_in" => Event::JackInserted(focus),
        "jack_out" => Event::JackRemoved(focus),
        "tick" => elapsed(5),
        "second" => elapsed(1001),
        "suspend" => Event::Suspend,
        "resume" => Event::Resume,
        "fault" => Event::FaultRaised,
        other => panic!("unknown event {other}"),
    }
}

fn recipe(name: &str, focus: Input) -> Vec<Event> {
    let ready = vec![
        Event::Boot,
        Event::BootReachedRunning,
        Event::JackRemoved(focus),
    ];
    let with = |base: Vec<Event>, more: &[Event]| [base, more.to_vec()].concat();
    let press = Event::ButtonPressed(focus);
    let release = Event::ButtonReleased(focus);
    let toggle = [press, elapsed(500), release];
    match name {
        "disarmed" => vec![],
        "booting" => vec![Event::Boot],
        "booting_ready" => vec![Event::Boot, Event::JackRemoved(focus)],
        "running_blocked" => vec![Event::Boot, Event::BootReachedRunning],
        "running_ready" => ready,
        "held_short" => with(ready, &[press]),
        "held_counted" => with(ready, &[press, elapsed(500)]),
        "held_too_long" => with(ready, &[press, elapsed(5000)]),
        "powered_muted" => with(ready, &toggle),
        "powered_settled" => with(recipe("powered_muted", focus), &[elapsed(1001)]),
        "powered_held_counted" => with(recipe("powered_settled", focus), &[press, elapsed(500)]),
        "unpowered_muted" => with(recipe("powered_settled", focus), &toggle),
        "suspended" => with(ready, &[Event::Suspend]),
        "suspended_muted" => with(recipe("powered_muted", focus), &[Event::Suspend]),
        other => panic!("unknown state {other}"),
    }
}

fn expected(tokens: &str, focus: Input) -> Vec<Seen> {
    let focus_on_word = match focus {
        Input::One => 0x4000,
        Input::Two => 0x0040,
    };
    tokens
        .split_whitespace()
        .map(|token| match token {
            "M" => Seen::Mute(focus),
            "U" => Seen::Unmute(focus),
            "W0" => Seen::Word(0x0000),
            "W1" => Seen::Word(focus_on_word),
            "L" => Seen::Pin(PinLevel::Low),
            "H" => Seen::Pin(PinLevel::High),
            other => panic!("unknown token {other}"),
        })
        .collect()
}

fn state(name: &str, focus: Input) -> State {
    run(&recipe(name, focus)).0
}

#[test]
fn every_fault_drives_the_phantom_enable_pin_low_by_gpio() {
    for state_name in STATES {
        let (before, _) = run(&recipe(state_name, Input::One));
        if before == State::Disarmed {
            continue;
        }
        let (after, outputs) = step(before, Event::FaultRaised);
        assert_eq!(after, State::Disarmed, "{state_name}");
        assert_eq!(
            outputs.last(),
            Some(&Seen::Pin(PinLevel::Low)),
            "{state_name}: a fault left the phantom enable pin alone: {outputs:?}"
        );
    }
    assert_eq!(
        Command::from_phantom(
            device::phantom::Output::DrivePhantomActivePin(PinLevel::Low),
            device::analog_settings::AnalogSettings::AT_BOOT,
        ),
        Command::DrivePin(OutputPin::PhantomActive, PinLevel::Low)
    );
}

#[test]
fn the_table_names_every_state_and_event_pair_exactly_once() {
    for state_name in STATES {
        for event_name in EVENTS {
            let rows = TABLE
                .iter()
                .filter(|(s, e, _, _)| *s == state_name && *e == event_name)
                .count();
            assert_eq!(rows, 1, "{state_name} on {event_name}");
        }
    }
    assert_eq!(TABLE.len(), STATES.len() * EVENTS.len());
}

#[test]
fn the_representative_states_are_all_distinct() {
    for focus in Input::BOTH {
        for (i, a) in STATES.iter().enumerate() {
            for b in &STATES[i + 1..] {
                assert_ne!(state(a, focus), state(b, focus), "{a} and {b}");
            }
        }
    }
}

#[test]
fn every_state_and_event_pair_emits_the_tabled_actions_and_next_state() {
    for focus in Input::BOTH {
        for (state_name, event_name, tokens, next_name) in TABLE {
            let before = state(state_name, focus);
            let (after, actions) = step(before, event(event_name, focus));
            assert_eq!(
                actions,
                expected(tokens, focus),
                "{state_name} on {event_name} for {focus:?}"
            );
            if *next_name != "unnamed" {
                assert_eq!(
                    after,
                    state(next_name, focus),
                    "{state_name} on {event_name} for {focus:?} lands in {next_name}"
                );
            }
        }
    }
}
