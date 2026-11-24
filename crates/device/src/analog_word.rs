use dsp::FramesPerTick;

use crate::input::Input;
use crate::phantom::{PhantomGrant, PhantomPower};

pub const SHIFT_BYTE: u8 = 0x00;

const PREAMP_ONE_CODE_SHIFT: u32 = 8;
const PREAMP_TWO_CODE_SHIFT: u32 = 0;
const PHANTOM_ONE_BIT: u16 = 1 << 14;
const PHANTOM_TWO_BIT: u16 = 1 << 6;
const DOUBLE_SPEED_BIT: u16 = 1 << 15;
const QUAD_SPEED_BIT: u16 = 1 << 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreampCode(u8);

impl PreampCode {
    pub const MINIMUM: Self = Self(0);
    pub const MAXIMUM: Self = Self(14);

    pub const fn new(value: u8) -> Option<Self> {
        if value <= Self::MAXIMUM.0 {
            Some(Self(value))
        } else {
            None
        }
    }

    pub const fn value(self) -> u8 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnalogWord(u16);

impl AnalogWord {
    pub const ALL_OFF: Self = Self(0);

    pub const fn new(
        preamp_one: PreampCode,
        preamp_two: PreampCode,
        speed: FramesPerTick,
        phantom: PhantomGrant,
    ) -> Self {
        Self(
            (preamp_one.0 as u16) << PREAMP_ONE_CODE_SHIFT
                | (preamp_two.0 as u16) << PREAMP_TWO_CODE_SHIFT
                | speed_bits(speed)
                | phantom_bit(phantom.power(Input::One), PHANTOM_ONE_BIT)
                | phantom_bit(phantom.power(Input::Two), PHANTOM_TWO_BIT),
        )
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn without_phantom(self) -> Self {
        Self(self.0 & !(PHANTOM_ONE_BIT | PHANTOM_TWO_BIT))
    }
}

const fn speed_bits(speed: FramesPerTick) -> u16 {
    match speed {
        FramesPerTick::Six => 0,
        FramesPerTick::Twelve => DOUBLE_SPEED_BIT,
        FramesPerTick::TwentyFour => QUAD_SPEED_BIT,
    }
}

const fn phantom_bit(power: PhantomPower, bit: u16) -> u16 {
    match power {
        PhantomPower::Off => 0,
        PhantomPower::On => bit,
    }
}
