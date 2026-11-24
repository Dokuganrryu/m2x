use crate::output_pin::OutputPin;
use crate::phantom::{Milliseconds, PinLevel};
use crate::pin::Pin;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FactoryStep {
    Write { register: u8, value: u8 },
    Read { register: u8 },
    Wait(Milliseconds),
    DrivePin { pin: Pin, level: PinLevel },
}

const fn write(register: u8, value: u8) -> FactoryStep {
    FactoryStep::Write { register, value }
}

const ADC_ENABLE: Pin = OutputPin::AdcEnable.pin();

pub const FACTORY_DAC_INIT: [FactoryStep; 40] = [
    write(0, 0x00),
    write(2, 0x1C),
    write(1, 0x00),
    write(7, 0x40),
    write(10, 0x00),
    write(12, 0x5A),
    write(13, 0x20),
    write(38, 0x50),
    write(39, 0x41),
    write(40, 0x72),
    write(41, 0x63),
    write(8, 0x8D),
    write(46, 0x62),
    write(24, 0xBF),
    write(25, 0x82),
    write(26, 0x14),
    write(27, 0x72),
    write(47, 0x3E),
    write(48, 0x00),
    write(49, 0x40),
    write(50, 0x00),
    write(51, 0x40),
    write(52, 0x00),
    write(63, 0x4A),
    FactoryStep::Read { register: 113 },
    FactoryStep::Read { register: 113 },
    FactoryStep::Read { register: 113 },
    FactoryStep::Wait(Milliseconds(100)),
    write(24, 0xFF),
    write(25, 0xFF),
    write(26, 0xFF),
    write(27, 0x7F),
    write(16, 0x00),
    write(17, 0x00),
    write(18, 0xFF),
    write(19, 0xFF),
    write(20, 0xFF),
    write(21, 0xFF),
    write(22, 0xFF),
    write(23, 0xFF),
];

pub const FACTORY_ADC_AT_13_INIT: [FactoryStep; 4] = [
    write(0, 0x0F),
    write(1, 0x01),
    write(2, 0x7B),
    write(4, 0x00),
];

pub const FACTORY_ADC_AT_4F_INIT: [FactoryStep; 1] = [write(1, 0xA7)];

pub const FACTORY_ADC_UNANSWERED_INIT: [FactoryStep; 3] = [
    FactoryStep::DrivePin {
        pin: ADC_ENABLE,
        level: PinLevel::Low,
    },
    FactoryStep::Wait(Milliseconds(5)),
    FactoryStep::DrivePin {
        pin: ADC_ENABLE,
        level: PinLevel::High,
    },
];
