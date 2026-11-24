use crate::command::Command;
use crate::dac::{Attenuation, DacWrite};
use crate::output_pin::OutputPin;
use crate::phantom::PinLevel;

// WHY: Milestone7 runs every boot stage with nothing reaching the outputs or the microphones: the board passes every command through here, so P0_28 and P1_4 are never driven high, every DAC volume write is full attenuation and no phantom bit reaches the analog word, whatever the state machines decide
pub const fn silenced(command: Command) -> Command {
    match phantom_held(command) {
        Command::DrivePin(OutputPin::TransportActive, PinLevel::High) => {
            Command::DrivePin(OutputPin::TransportActive, PinLevel::Low)
        }
        Command::WriteDac(DacWrite::Volume(register, _)) => {
            Command::WriteDac(DacWrite::Volume(register, Attenuation::FULL))
        }
        other => other,
    }
}

// WHY: The sound builds play: P0_28 and the DAC volume follow the boot, rate and control machines, and only P1_4 and the analog word's phantom bits stay held, so no phantom voltage reaches a microphone while the 48V LEDs and the screen still follow the buttons
pub const fn phantom_held(command: Command) -> Command {
    match command {
        Command::DrivePin(OutputPin::PhantomActive, PinLevel::High) => {
            Command::DrivePin(OutputPin::PhantomActive, PinLevel::Low)
        }
        Command::SendAnalogWord(word) => Command::SendAnalogWord(word.without_phantom()),
        other => other,
    }
}
