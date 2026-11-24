use crate::input::Input;
use crate::monitor::Monitoring;
use crate::phantom::{Event, PhantomGrant, PhantomPower};

pub const EXPANDER_ADDRESS: u8 = 0x39;
pub const EXPANDER_DIRECTION_REGISTER: u8 = 3;
pub const EXPANDER_BUTTONS_AS_INPUTS: u8 = 0x8D;
pub const EXPANDER_INPUT_REGISTER: u8 = 0;
pub const EXPANDER_OUTPUT_REGISTER: u8 = 1;

const EVERY_LED_DARK: u8 =
    PHANTOM_ONE_LED_MASK | PHANTOM_TWO_LED_MASK | MONITOR_ONE_LED_MASK | MONITOR_TWO_LED_MASK;
const PHANTOM_ONE_LED_MASK: u8 = 1 << 6;
const PHANTOM_TWO_LED_MASK: u8 = 1 << 1;
const MONITOR_ONE_LED_MASK: u8 = 1 << 5;
const MONITOR_TWO_LED_MASK: u8 = 1 << 4;

const PHANTOM_ONE_BUTTON_MASK: u8 = 1 << 3;
const PHANTOM_TWO_BUTTON_MASK: u8 = 1 << 0;
const MONITOR_ONE_BUTTON_MASK: u8 = 1 << 7;
const MONITOR_TWO_BUTTON_MASK: u8 = 1 << 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonLevel {
    Pressed,
    Released,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExpanderInputs(pub u8);

impl ExpanderInputs {
    pub const fn phantom_button_event(self, input: Input) -> Event {
        let mask = match input {
            Input::One => PHANTOM_ONE_BUTTON_MASK,
            Input::Two => PHANTOM_TWO_BUTTON_MASK,
        };
        if self.0 & mask == 0 {
            Event::ButtonPressed(input)
        } else {
            Event::ButtonReleased(input)
        }
    }

    pub const fn monitor_button(self, input: Input) -> ButtonLevel {
        let mask = match input {
            Input::One => MONITOR_ONE_BUTTON_MASK,
            Input::Two => MONITOR_TWO_BUTTON_MASK,
        };
        if self.0 & mask == 0 {
            ButtonLevel::Pressed
        } else {
            ButtonLevel::Released
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExpanderOutputs(u8);

impl ExpanderOutputs {
    pub const fn showing(grant: PhantomGrant, monitoring: [Monitoring; 2]) -> Self {
        let [monitor_one, monitor_two] = monitoring;
        let lit = phantom_led(grant.power(Input::One), PHANTOM_ONE_LED_MASK)
            | phantom_led(grant.power(Input::Two), PHANTOM_TWO_LED_MASK)
            | monitor_led(monitor_one, MONITOR_ONE_LED_MASK)
            | monitor_led(monitor_two, MONITOR_TWO_LED_MASK);
        Self(EVERY_LED_DARK & !lit)
    }

    pub const fn bits(self) -> u8 {
        self.0
    }
}

const fn phantom_led(power: PhantomPower, mask: u8) -> u8 {
    match power {
        PhantomPower::Off => 0,
        PhantomPower::On => mask,
    }
}

const fn monitor_led(monitoring: Monitoring, mask: u8) -> u8 {
    match monitoring {
        Monitoring::Off => 0,
        Monitoring::On => mask,
    }
}
