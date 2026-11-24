use crate::analog_word::AnalogWord;
use crate::breadcrumb::{At, Weight};
use crate::command::Command;
use crate::output_pin::OutputPin;
use crate::phantom::PinLevel;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum I2cPeer {
    Dac,
    Adc,
    Expander,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultCode {
    Unanswered(I2cPeer),
    BoardStalled,
    Panic,
    HardFault,
    UnexpectedException,
    FenceUnavailable,
    RefusedCommand,
    CrystalNotReady,
    CorePllUnlocked,
    AudioPllUnlocked,
    UsbPhyPllUnlocked,
    #[cfg(feature = "sound3")]
    CoreClockHeld,
}

impl FaultCode {
    pub const ALL: [FaultCode; 13 + cfg!(feature = "sound3") as usize] = [
        FaultCode::Unanswered(I2cPeer::Dac),
        FaultCode::Unanswered(I2cPeer::Adc),
        FaultCode::Unanswered(I2cPeer::Expander),
        FaultCode::BoardStalled,
        FaultCode::Panic,
        FaultCode::HardFault,
        FaultCode::UnexpectedException,
        FaultCode::FenceUnavailable,
        FaultCode::RefusedCommand,
        FaultCode::CrystalNotReady,
        FaultCode::CorePllUnlocked,
        FaultCode::AudioPllUnlocked,
        FaultCode::UsbPhyPllUnlocked,
        #[cfg(feature = "sound3")]
        FaultCode::CoreClockHeld,
    ];

    pub const fn number(self) -> u8 {
        match self {
            FaultCode::Unanswered(I2cPeer::Dac) => 3,
            FaultCode::Unanswered(I2cPeer::Adc) => 4,
            FaultCode::Unanswered(I2cPeer::Expander) => 5,
            FaultCode::BoardStalled => 6,
            FaultCode::Panic => 7,
            FaultCode::HardFault => 8,
            FaultCode::UnexpectedException => 9,
            FaultCode::FenceUnavailable => 10,
            FaultCode::RefusedCommand => 11,
            FaultCode::CrystalNotReady => 12,
            FaultCode::CorePllUnlocked => 13,
            FaultCode::AudioPllUnlocked => 14,
            FaultCode::UsbPhyPllUnlocked => 16,
            // WHY: A noted code: the core stayed at 100 MHz and runs the 7-band fallback at 96 kHz
            #[cfg(feature = "sound3")]
            FaultCode::CoreClockHeld => 17,
        }
    }
}

const _: () = {
    assert!(
        core::mem::size_of::<crate::boot::Faults>() * (u8::BITS as usize) == 32,
        "the boot outcome's fault set is 32 bits wide"
    );
    let mut index = 0;
    while index < FaultCode::ALL.len() {
        let number = FaultCode::ALL[index].number();
        assert!(
            (number as usize) < core::mem::size_of::<crate::boot::Faults>() * (u8::BITS as usize),
            "a fault code number is a bit position in the boot outcome's 32-bit fault set"
        );
        index += 1;
    }
};

pub const fn fault_routine(code: FaultCode) -> [Command; 6] {
    [
        Command::StopTransmitDma,
        Command::DrivePin(OutputPin::TransportActive, PinLevel::Low),
        Command::ZeroTransmitBuffers,
        Command::DrivePin(OutputPin::PhantomActive, PinLevel::Low),
        Command::SendAnalogWord(AnalogWord::ALL_OFF),
        Command::RecordFault(code, At::Here, Weight::Raised),
    ]
}
