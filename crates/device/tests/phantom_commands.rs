use device::analog_settings::AnalogSettings;
use device::analog_word::{AnalogWord, PreampCode};
use device::command::{Command, CommandList};
use device::input::Input;
use device::output_pin::OutputPin;
use device::phantom::{Event, MOST_OUTPUTS_PER_EVENT, Milliseconds, PinLevel, State, transition};
use dsp::FramesPerTick;

const SETTINGS: AnalogSettings = AnalogSettings {
    preamp_one: match PreampCode::new(5) {
        Some(code) => code,
        None => PreampCode::MINIMUM,
    },
    preamp_two: match PreampCode::new(9) {
        Some(code) => code,
        None => PreampCode::MINIMUM,
    },
    speed: FramesPerTick::Twelve,
};

fn commands_for(events: &[Event], settings: AnalogSettings) -> Vec<Command> {
    let mut state = State::Disarmed;
    let mut all = Vec::new();
    for event in events {
        let (next, actions) = transition(state, *event);
        let mut list = CommandList::<MOST_OUTPUTS_PER_EVENT>::new();
        assert_eq!(
            list.extend(
                actions
                    .as_slice()
                    .iter()
                    .map(|action| Command::from_phantom(*action, settings))
            ),
            Ok(()),
            "one event's actions fit one command list"
        );
        all.extend_from_slice(list.as_slice());
        state = next;
    }
    all
}

#[test]
fn boot_maps_to_the_all_off_word_then_the_phantom_pin_low() {
    assert_eq!(
        commands_for(&[Event::Boot], AnalogSettings::AT_BOOT),
        vec![
            Command::SendAnalogWord(AnalogWord::ALL_OFF),
            Command::DrivePin(OutputPin::PhantomActive, PinLevel::Low),
        ]
    );
}

#[test]
fn a_phantom_rise_maps_to_mute_then_the_composed_word_then_the_phantom_pin_high_in_that_order() {
    let commands = commands_for(
        &[
            Event::Boot,
            Event::BootReachedRunning,
            Event::JackRemoved(Input::One),
            Event::ButtonPressed(Input::One),
            Event::Elapsed(Milliseconds(100)),
            Event::ButtonReleased(Input::One),
        ],
        SETTINGS,
    );
    let rise = &commands[2..];
    assert_eq!(rise.len(), 3);
    assert_eq!(rise[0], Command::MuteInput(Input::One));
    match rise[1] {
        Command::SendAnalogWord(sent) => assert_eq!(sent.bits(), 0xC509),
        other => panic!("expected the analog word, got {other:?}"),
    }
    assert_eq!(
        rise[2],
        Command::DrivePin(OutputPin::PhantomActive, PinLevel::High)
    );
}

#[test]
fn the_mute_after_a_phantom_change_ends_with_an_unmute_command_a_second_later() {
    let commands = commands_for(
        &[
            Event::Boot,
            Event::BootReachedRunning,
            Event::JackRemoved(Input::Two),
            Event::ButtonPressed(Input::Two),
            Event::Elapsed(Milliseconds(100)),
            Event::ButtonReleased(Input::Two),
            Event::Elapsed(Milliseconds(1000)),
            Event::Elapsed(Milliseconds(1)),
        ],
        SETTINGS,
    );
    assert_eq!(commands.last(), Some(&Command::UnmuteInput(Input::Two)));
    assert_eq!(
        commands
            .iter()
            .filter(|command| matches!(command, Command::UnmuteInput(_)))
            .count(),
        1
    );
}

#[test]
fn a_jack_insert_on_a_powered_input_maps_to_mute_off_word_and_pin_low() {
    let commands = commands_for(
        &[
            Event::Boot,
            Event::BootReachedRunning,
            Event::JackRemoved(Input::One),
            Event::ButtonPressed(Input::One),
            Event::Elapsed(Milliseconds(100)),
            Event::ButtonReleased(Input::One),
            Event::JackInserted(Input::One),
        ],
        AnalogSettings::AT_BOOT,
    );
    assert_eq!(
        commands[commands.len() - 3..],
        [
            Command::MuteInput(Input::One),
            Command::SendAnalogWord(AnalogWord::ALL_OFF),
            Command::DrivePin(OutputPin::PhantomActive, PinLevel::Low),
        ]
    );
}
