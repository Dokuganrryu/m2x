use super::trace::{Event, Write};
use std::ops::Range;

// WHY: SVD28 rows IOCON PIO1_4 0x40001090, GPIO B[1].B_[4] 0x4008C024, W[1].W_[4] 0x4008D090, DIR[1] 0x4008E004, PIN[1] 0x4008E104, SET[1] 0x4008E204, CLR[1] 0x4008E284, NOT[1] 0x4008E304, DIRSET[1] 0x4008E384 and DIRCLR[1] 0x4008E404, IOCON PIO[2][32] steps 0x80 a port and 0x4 a pin from 0x40001000, B steps one byte a pin and W one word a pin; every GPIO output pad takes the word 0x100
const IOCON_PIO1_0: u32 = 0x4000_1080;
const GPIO_B1_0: u32 = 0x4008_C020;
const GPIO_W1_0: u32 = 0x4008_D080;
const FACTORY_GPIO_OUTPUT_PAD: u32 = 0x100;
const GPIO_DIR1: u32 = 0x4008_E004;
const GPIO_PIN1: u32 = 0x4008_E104;
const GPIO_SET1: u32 = 0x4008_E204;
const GPIO_CLR1: u32 = 0x4008_E284;
const GPIO_NOT1: u32 = 0x4008_E304;
const GPIO_DIRSET1: u32 = 0x4008_E384;
const GPIO_DIRCLR1: u32 = 0x4008_E404;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeldPin {
    pub name: &'static str,
    pub role: &'static str,
    pub number: u32,
}

// WHY: P1_4 is the phantom enable, high applies 48 V; device output_pin.rs, P1_3 is ConverterEnable and P1_29 AdcEnable
pub const PHANTOM: HeldPin = HeldPin {
    name: "P1_4",
    role: "the phantom pin",
    number: 4,
};
pub const CONVERTER_ENABLE: HeldPin = HeldPin {
    name: "P1_3",
    role: "the converter enable pin",
    number: 3,
};
pub const ADC_ENABLE: HeldPin = HeldPin {
    name: "P1_29",
    role: "the ADC enable pin",
    number: 29,
};
pub const HELD_LOW: [HeldPin; 3] = [PHANTOM, CONVERTER_ENABLE, ADC_ENABLE];

impl HeldPin {
    const fn bit(self) -> u32 {
        1 << self.number
    }

    const fn pad(self) -> u32 {
        IOCON_PIO1_0 + 4 * self.number
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinAtEntry {
    Inherited,
    PadSelected,
    LevelLow,
    DrivenLow,
    Disturbed(Write),
}

impl PinAtEntry {
    fn follow(self, pin: HeldPin, write: &Write, table: &Range<u32>) -> Self {
        let names_the_pin = write.value & pin.bit() != 0;
        let level_is_low = matches!(self, PinAtEntry::LevelLow | PinAtEntry::DrivenLow);
        let disturbs = match write.address {
            address if address == pin.pad() => write.value != FACTORY_GPIO_OUTPUT_PAD,
            address
                if address == GPIO_B1_0 + pin.number || address == GPIO_W1_0 + 4 * pin.number =>
            {
                write.value != 0
            }
            GPIO_DIR1 => !names_the_pin || !level_is_low,
            GPIO_DIRSET1 => names_the_pin && !level_is_low,
            GPIO_PIN1 | GPIO_SET1 | GPIO_NOT1 | GPIO_DIRCLR1 => names_the_pin,
            _ => false,
        };
        if disturbs && !matches!(self, PinAtEntry::Disturbed(_)) {
            return PinAtEntry::Disturbed(*write);
        }
        if !table.contains(&write.pc) {
            return self;
        }
        match (self, write.address) {
            (PinAtEntry::Inherited, address) if address == pin.pad() => PinAtEntry::PadSelected,
            (PinAtEntry::PadSelected, GPIO_CLR1) if names_the_pin => PinAtEntry::LevelLow,
            (PinAtEntry::LevelLow, GPIO_DIRSET1) if names_the_pin => PinAtEntry::DrivenLow,
            (unchanged, _) => unchanged,
        }
    }

    pub fn describe(self, pin: HeldPin) -> String {
        let HeldPin { name, role, .. } = pin;
        let missing = match self {
            PinAtEntry::DrivenLow => {
                return format!(
                    "{name}, {role}, is driven low by the hand-over table: pad word 0x100, clear, direction"
                );
            }
            PinAtEntry::Inherited => format!("writes no pad word 0x100 to IOCON PIO{}", &name[1..]),
            PinAtEntry::PadSelected => {
                format!("does not clear {name} in CLR[1] after the pad word")
            }
            PinAtEntry::LevelLow => {
                format!("does not make {name} an output in DIRSET[1] after the clear")
            }
            PinAtEntry::Disturbed(write) => {
                return format!(
                    "{name}, {role}, is not driven low and held low up to `main`: the instruction at {:#010x} (step {}) wrote {:#x} to {:#010x}, which raises it, releases it, changes its pad or makes it an output before its level is cleared",
                    write.pc, write.index, write.value, write.address
                );
            }
        };
        format!(
            "{name}, {role}, is not driven low by the hand-over table before `main`: the table {missing}"
        )
    }
}

pub fn pin_at_entry(events: &[Event], table: &Range<u32>, pin: HeldPin) -> PinAtEntry {
    let mut state = PinAtEntry::Inherited;
    for event in events {
        match event {
            Event::Write(write) => state = state.follow(pin, write, table),
            Event::Reached { .. } => break,
            _ => {}
        }
    }
    state
}
