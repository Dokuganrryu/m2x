use device::expander::{EXPANDER_ADDRESS, ExpanderInputs};
use device::input::Input;
use device::phantom::Event;
use device::pin::{PHANTOM_ACTIVE_PIN, Pin, jack_sense_pin};

#[test]
fn the_panel_expander_answers_at_seven_bit_address_0x39() {
    assert_eq!(EXPANDER_ADDRESS, 0x39);
}

#[test]
fn bit_3_low_is_the_phantom_one_button_pressed() {
    let inputs = ExpanderInputs(0b1111_0111);
    assert_eq!(
        inputs.phantom_button_event(Input::One),
        Event::ButtonPressed(Input::One)
    );
    assert_eq!(
        inputs.phantom_button_event(Input::Two),
        Event::ButtonReleased(Input::Two)
    );
}

#[test]
fn bit_0_low_is_the_phantom_two_button_pressed() {
    let inputs = ExpanderInputs(0b1111_1110);
    assert_eq!(
        inputs.phantom_button_event(Input::One),
        Event::ButtonReleased(Input::One)
    );
    assert_eq!(
        inputs.phantom_button_event(Input::Two),
        Event::ButtonPressed(Input::Two)
    );
}

#[test]
fn monitor_bits_7_and_2_never_read_as_a_phantom_button() {
    let inputs = ExpanderInputs(0b0111_1011);
    for input in Input::BOTH {
        assert_eq!(
            inputs.phantom_button_event(input),
            Event::ButtonReleased(input)
        );
    }
}

#[test]
fn jack_sense_pins_are_p1_15_and_p0_2() {
    assert_eq!(
        jack_sense_pin(Input::One),
        Pin {
            port: 1,
            number: 15
        }
    );
    assert_eq!(jack_sense_pin(Input::Two), Pin { port: 0, number: 2 });
}

#[test]
fn the_phantom_active_pin_is_p1_4() {
    assert_eq!(PHANTOM_ACTIVE_PIN, Pin { port: 1, number: 4 });
}

#[test]
fn a_nonzero_jack_pin_byte_reads_as_inserted() {
    for input in Input::BOTH {
        assert_eq!(
            Event::from_jack_pin_byte(input, 0),
            Event::JackRemoved(input)
        );
        for byte in 1..=u8::MAX {
            assert_eq!(
                Event::from_jack_pin_byte(input, byte),
                Event::JackInserted(input)
            );
        }
    }
}
