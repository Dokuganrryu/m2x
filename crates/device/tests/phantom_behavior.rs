mod support;

use device::input::Input;
use device::phantom::{Event, Milliseconds, PinLevel};
use support::{Seen, run};

const OFF_WORD: Seen = Seen::Word(0x0000);
const ONE_ON_WORD: Seen = Seen::Word(0x4000);
const TWO_ON_WORD: Seen = Seen::Word(0x0040);
const BOTH_ON_WORD: Seen = Seen::Word(0x4040);
const LOW: Seen = Seen::Pin(PinLevel::Low);
const HIGH: Seen = Seen::Pin(PinLevel::High);

fn elapsed(ms: u32) -> Event {
    Event::Elapsed(Milliseconds(ms))
}

fn ready(input: Input) -> Vec<Event> {
    vec![
        Event::Boot,
        Event::BootReachedRunning,
        Event::JackRemoved(input),
    ]
}

fn press(input: Input, held_ms: u32) -> [Event; 3] {
    [
        Event::ButtonPressed(input),
        elapsed(held_ms),
        Event::ButtonReleased(input),
    ]
}

#[test]
fn boot_sends_the_all_off_word_then_drives_the_phantom_pin_low() {
    let (_, actions) = run(&[Event::Boot]);
    assert_eq!(actions, vec![OFF_WORD, LOW]);
}

#[test]
fn nothing_is_emitted_before_boot() {
    let (_, actions) = run(&[
        Event::BootReachedRunning,
        Event::JackRemoved(Input::One),
        Event::ButtonPressed(Input::One),
        elapsed(100),
        Event::ButtonReleased(Input::One),
        Event::Suspend,
        Event::Resume,
    ]);
    assert_eq!(actions, vec![]);
}

#[test]
fn a_short_press_after_running_turns_phantom_one_on_with_a_mute_word_and_pin() {
    let mut events = ready(Input::One);
    events.extend(press(Input::One, 100));
    let (_, actions) = run(&events);
    assert_eq!(
        actions,
        vec![OFF_WORD, LOW, Seen::Mute(Input::One), ONE_ON_WORD, HIGH]
    );
}

#[test]
fn input_two_maps_to_bit_six() {
    let mut events = ready(Input::Two);
    events.extend(press(Input::Two, 100));
    let (_, actions) = run(&events);
    assert_eq!(
        actions,
        vec![OFF_WORD, LOW, Seen::Mute(Input::Two), TWO_ON_WORD, HIGH]
    );
}

#[test]
fn a_second_press_turns_phantom_back_off_and_drops_the_pin() {
    let mut events = ready(Input::One);
    events.extend(press(Input::One, 100));
    events.extend(press(Input::One, 100));
    let (_, actions) = run(&events);
    assert_eq!(actions[5..], [Seen::Mute(Input::One), OFF_WORD, LOW]);
}

#[test]
fn the_pin_stays_high_while_either_input_is_powered() {
    let mut events = ready(Input::One);
    events.push(Event::JackRemoved(Input::Two));
    events.extend(press(Input::One, 100));
    events.extend(press(Input::Two, 100));
    events.extend(press(Input::One, 100));
    let (_, actions) = run(&events);
    assert_eq!(
        actions[2..],
        [
            Seen::Mute(Input::One),
            ONE_ON_WORD,
            HIGH,
            Seen::Mute(Input::Two),
            BOTH_ON_WORD,
            HIGH,
            Seen::Mute(Input::One),
            TWO_ON_WORD,
            HIGH,
        ]
    );
}

#[test]
fn a_press_counts_only_when_released_between_30_ms_and_1_s() {
    for (held_ms, toggles) in [
        (0, false),
        (29, false),
        (30, true),
        (500, true),
        (1000, true),
        (1001, false),
        (60_000, false),
    ] {
        let mut events = ready(Input::One);
        events.extend(press(Input::One, held_ms));
        let (_, actions) = run(&events);
        assert_eq!(actions.contains(&ONE_ON_WORD), toggles, "held {held_ms} ms");
    }
}

#[test]
fn hold_time_accumulates_across_elapsed_events() {
    let mut events = ready(Input::One);
    events.push(Event::ButtonPressed(Input::One));
    events.extend([elapsed(5); 6]);
    events.push(Event::ButtonReleased(Input::One));
    let (_, actions) = run(&events);
    assert!(actions.contains(&ONE_ON_WORD));
}

#[test]
fn a_press_during_boot_never_raises_phantom() {
    let (_, actions) = run(&[
        Event::Boot,
        Event::JackRemoved(Input::One),
        Event::ButtonPressed(Input::One),
        elapsed(100),
        Event::ButtonReleased(Input::One),
        Event::BootReachedRunning,
    ]);
    assert_eq!(actions, vec![OFF_WORD, LOW]);
}

#[test]
fn a_press_begun_during_boot_does_not_count_after_running() {
    let (_, actions) = run(&[
        Event::Boot,
        Event::JackRemoved(Input::One),
        Event::ButtonPressed(Input::One),
        Event::BootReachedRunning,
        elapsed(100),
        Event::ButtonReleased(Input::One),
    ]);
    assert_eq!(actions, vec![OFF_WORD, LOW]);
}

#[test]
fn phantom_is_blocked_until_the_jack_pin_has_read_zero() {
    let (_, actions) = run(&[
        Event::Boot,
        Event::BootReachedRunning,
        Event::ButtonPressed(Input::One),
        elapsed(100),
        Event::ButtonReleased(Input::One),
    ]);
    assert_eq!(actions, vec![OFF_WORD, LOW]);
}

#[test]
fn the_jack_pin_clears_a_powered_input() {
    let mut events = ready(Input::One);
    events.extend(press(Input::One, 100));
    events.push(Event::JackInserted(Input::One));
    let (_, actions) = run(&events);
    assert_eq!(actions[5..], [Seen::Mute(Input::One), OFF_WORD, LOW]);
}

#[test]
fn a_press_while_the_jack_pin_reads_nonzero_changes_nothing() {
    let mut events = ready(Input::One);
    events.push(Event::JackInserted(Input::One));
    events.extend(press(Input::One, 100));
    let (_, actions) = run(&events);
    assert_eq!(actions, vec![OFF_WORD, LOW]);
}

#[test]
fn removing_the_jack_does_not_restore_a_cleared_request() {
    let mut events = ready(Input::One);
    events.extend(press(Input::One, 100));
    events.push(Event::JackInserted(Input::One));
    events.push(Event::JackRemoved(Input::One));
    let (_, actions) = run(&events);
    assert_eq!(actions.last(), Some(&LOW));
    assert_eq!(actions.iter().filter(|a| **a == ONE_ON_WORD).count(), 1);
}

#[test]
fn jack_and_button_levels_repeated_every_pass_are_idempotent() {
    let mut events = ready(Input::One);
    events.extend([Event::JackRemoved(Input::One); 3]);
    events.extend([Event::ButtonPressed(Input::One); 3]);
    events.push(elapsed(100));
    events.extend([Event::ButtonReleased(Input::One); 3]);
    events.extend([Event::JackRemoved(Input::One); 3]);
    let (_, actions) = run(&events);
    assert_eq!(
        actions,
        vec![OFF_WORD, LOW, Seen::Mute(Input::One), ONE_ON_WORD, HIGH]
    );
}

#[test]
fn the_input_unmutes_one_second_after_a_phantom_change() {
    let mut events = ready(Input::One);
    events.extend(press(Input::One, 100));
    events.extend([elapsed(5); 200]);
    let (_, before) = run(&events);
    assert!(!before.contains(&Seen::Unmute(Input::One)));
    events.push(elapsed(1));
    let (_, after) = run(&events);
    assert_eq!(after.last(), Some(&Seen::Unmute(Input::One)));
}

#[test]
fn a_change_during_the_mute_restarts_the_second() {
    let mut events = ready(Input::One);
    events.extend(press(Input::One, 100));
    events.push(elapsed(600));
    events.extend(press(Input::One, 100));
    events.push(elapsed(1000));
    let (_, before) = run(&events);
    assert!(!before.contains(&Seen::Unmute(Input::One)));
    events.push(elapsed(1));
    let (_, after) = run(&events);
    assert_eq!(after.last(), Some(&Seen::Unmute(Input::One)));
}

#[test]
fn suspend_clears_both_requests() {
    let mut events = ready(Input::One);
    events.push(Event::JackRemoved(Input::Two));
    events.extend(press(Input::One, 100));
    events.extend(press(Input::Two, 100));
    events.push(Event::Suspend);
    let (_, actions) = run(&events);
    assert_eq!(
        actions[8..],
        [
            Seen::Mute(Input::One),
            Seen::Mute(Input::Two),
            OFF_WORD,
            LOW
        ]
    );
}

#[test]
fn buttons_do_nothing_while_suspended_and_requests_stay_clear_after_resume() {
    let mut events = ready(Input::One);
    events.extend(press(Input::One, 100));
    events.push(Event::Suspend);
    events.extend(press(Input::One, 100));
    events.push(Event::Resume);
    let (_, suspended) = run(&events);
    assert_eq!(suspended.last(), Some(&LOW));
    events.extend(press(Input::One, 100));
    let (_, resumed) = run(&events);
    assert_eq!(resumed.last(), Some(&HIGH));
}

#[test]
fn boot_from_running_with_phantom_on_sends_the_all_off_word() {
    let mut events = ready(Input::One);
    events.extend(press(Input::One, 100));
    events.push(Event::Boot);
    let (_, actions) = run(&events);
    assert_eq!(actions[5..], [OFF_WORD, LOW]);
}

#[test]
fn a_fault_with_phantom_on_mutes_and_clears_it_and_no_press_raises_it_until_the_next_boot() {
    let mut events = ready(Input::One);
    events.extend(press(Input::One, 100));
    let (_, before_fault) = run(&events);
    events.push(Event::FaultRaised);
    let (_, faulted) = run(&events);
    assert_eq!(
        faulted[before_fault.len()..],
        [Seen::Mute(Input::One), OFF_WORD, LOW]
    );
    events.extend([Event::BootReachedRunning, Event::JackRemoved(Input::One)]);
    events.extend(press(Input::One, 100));
    events.extend(press(Input::Two, 100));
    events.push(elapsed(2000));
    let (_, after_fault) = run(&events);
    assert_eq!(after_fault, faulted);
    events.extend(ready(Input::One));
    events.extend(press(Input::One, 100));
    let (_, rebooted) = run(&events);
    assert_eq!(rebooted.last(), Some(&HIGH));
}

#[test]
fn a_press_begun_while_powered_never_turns_phantom_back_on_after_a_jack_goes_in_and_out() {
    let mut events = ready(Input::One);
    events.extend(press(Input::One, 100));
    events.extend([
        elapsed(1000),
        Event::ButtonPressed(Input::One),
        elapsed(100),
        Event::JackInserted(Input::One),
        elapsed(100),
        Event::JackRemoved(Input::One),
        elapsed(100),
        Event::ButtonReleased(Input::One),
    ]);
    let (state, _) = run(&events);
    assert_eq!(
        state.grant().power(Input::One),
        device::phantom::PhantomPower::Off
    );
}

#[test]
fn a_press_begun_with_a_jack_in_never_turns_phantom_on_after_the_jack_comes_out() {
    let mut events = vec![Event::Boot, Event::BootReachedRunning];
    events.extend([
        Event::ButtonPressed(Input::One),
        elapsed(100),
        Event::JackRemoved(Input::One),
        elapsed(100),
        Event::ButtonReleased(Input::One),
    ]);
    let (state, _) = run(&events);
    assert_eq!(
        state.grant().power(Input::One),
        device::phantom::PhantomPower::Off
    );
}

#[test]
fn a_press_still_counts_while_the_jack_reading_repeats_unchanged() {
    let mut events = ready(Input::One);
    events.extend([
        Event::ButtonPressed(Input::One),
        elapsed(50),
        Event::JackRemoved(Input::One),
        elapsed(50),
        Event::JackRemoved(Input::One),
        Event::ButtonReleased(Input::One),
    ]);
    let (state, _) = run(&events);
    assert_eq!(
        state.grant().power(Input::One),
        device::phantom::PhantomPower::On
    );
}
