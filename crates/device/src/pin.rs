use crate::input::Input;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pin {
    pub port: u8,
    pub number: u8,
}

pub const PHANTOM_ACTIVE_PIN: Pin = Pin { port: 1, number: 4 };

const INPUT_ONE_JACK_SENSE_PIN: Pin = Pin {
    port: 1,
    number: 15,
};

const INPUT_TWO_JACK_SENSE_PIN: Pin = Pin { port: 0, number: 2 };

pub const fn jack_sense_pin(input: Input) -> Pin {
    match input {
        Input::One => INPUT_ONE_JACK_SENSE_PIN,
        Input::Two => INPUT_TWO_JACK_SENSE_PIN,
    }
}
