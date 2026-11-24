use device::adc::AdcAddress;
use device::analog_settings::AnalogSettings;
use device::command::Command;
use device::dac::{Attenuation, ConfigRegister, DacWrite, MasterTrim, VolumeRegister};
use device::expander::ExpanderOutputs;
use device::input::Input;
use device::output_pin::OutputPin;
use device::phantom::{self, Milliseconds, PhantomGrant, PhantomPower, PinLevel};
use device::silent::silenced;
use dsp::{FramesPerTick, RateFamily};

const PHANTOM_BITS: u16 = (1 << 14) | (1 << 6);

fn both_granted() -> PhantomGrant {
    let mut state = phantom::State::Disarmed;
    let mut feed = |event| state = phantom::transition(state, event).0;
    feed(phantom::Event::Boot);
    feed(phantom::Event::Elapsed(Milliseconds(2000)));
    feed(phantom::Event::BootReachedRunning);
    for input in Input::BOTH {
        feed(phantom::Event::JackRemoved(input));
        feed(phantom::Event::ButtonPressed(input));
        feed(phantom::Event::Elapsed(Milliseconds(100)));
        feed(phantom::Event::ButtonReleased(input));
    }
    let grant = state.grant();
    assert_eq!(grant.power(Input::One), PhantomPower::On);
    assert_eq!(grant.power(Input::Two), PhantomPower::On);
    grant
}

fn reaches_the_outputs_or_the_microphones(command: Command) -> bool {
    match command {
        Command::DrivePin(
            OutputPin::TransportActive | OutputPin::PhantomActive,
            PinLevel::High,
        ) => true,
        Command::WriteDac(DacWrite::Volume(_, level)) => level != Attenuation::FULL,
        Command::SendAnalogWord(word) => word.bits() & PHANTOM_BITS != 0,
        _ => false,
    }
}

fn every_command_shape() -> Vec<Command> {
    let mut all = Vec::new();
    for pin in OutputPin::ALL {
        for level in [PinLevel::Low, PinLevel::High] {
            all.push(Command::DrivePin(pin, level));
        }
    }
    for register in VolumeRegister::ALL {
        for steps in [0x00, 0x01, 0x40, 0xFE, 0xFF] {
            all.push(Command::WriteDac(DacWrite::Volume(
                register,
                Attenuation::from_half_decibel_steps(steps),
            )));
        }
    }
    let settings = AnalogSettings::AT_BOOT;
    for grant in [PhantomGrant::NONE, both_granted()] {
        all.push(Command::SendAnalogWord(settings.word(grant)));
    }
    all.extend([
        Command::WriteDac(DacWrite::Config(ConfigRegister::Register7)),
        Command::WriteDac(DacWrite::MasterTrim(MasterTrim::MINUS_ONE_DB)),
        Command::ProbeAdc(AdcAddress::At13),
        Command::SwitchPll0Family(RateFamily::FortyEightKilohertz),
        Command::StartI2s(FramesPerTick::Six),
        Command::StartDma,
        Command::StartTickTimer(FramesPerTick::Six),
        Command::StartCorePll,
        Command::SelectCorePllAsMainClock,
        Command::WriteExpanderDirection,
        Command::WriteExpanderOutputs(ExpanderOutputs::showing(
            both_granted(),
            [device::control_pass::Monitoring::On; 2],
        )),
        Command::ReadExpanderInputs,
        Command::StartKnobScan,
        Command::AttachUsb,
        Command::MuteInput(Input::One),
        Command::UnmuteInput(Input::Two),
    ]);
    all
}

#[test]
fn no_command_leaves_the_filter_able_to_reach_the_outputs_or_the_microphones() {
    for command in every_command_shape() {
        let after = silenced(command);
        assert!(
            !reaches_the_outputs_or_the_microphones(after),
            "{command:?} left the filter as {after:?}"
        );
    }
}

#[test]
fn the_output_connect_and_phantom_rail_pins_are_driven_low_instead_of_high() {
    for pin in [OutputPin::TransportActive, OutputPin::PhantomActive] {
        assert_eq!(
            silenced(Command::DrivePin(pin, PinLevel::High)),
            Command::DrivePin(pin, PinLevel::Low)
        );
    }
}

#[test]
fn every_volume_write_becomes_full_attenuation_on_the_same_register() {
    for register in VolumeRegister::ALL {
        assert_eq!(
            silenced(Command::WriteDac(DacWrite::Volume(
                register,
                Attenuation::ZERO_DB
            ))),
            Command::WriteDac(DacWrite::Volume(register, Attenuation::FULL))
        );
    }
}

#[test]
fn the_analog_word_keeps_its_preamp_and_speed_bits_and_loses_only_phantom() {
    let settings = AnalogSettings::AT_BOOT;
    let granted = settings.word(both_granted());
    let plain = settings.word(PhantomGrant::NONE);
    assert_eq!(granted.bits() & PHANTOM_BITS, PHANTOM_BITS);
    assert_eq!(
        silenced(Command::SendAnalogWord(granted)),
        Command::SendAnalogWord(plain)
    );
}

#[test]
fn every_other_command_passes_unchanged() {
    for command in every_command_shape() {
        let touched = matches!(
            command,
            Command::DrivePin(
                OutputPin::TransportActive | OutputPin::PhantomActive,
                PinLevel::High
            ) | Command::WriteDac(DacWrite::Volume(..))
                | Command::SendAnalogWord(_)
        );
        if !touched {
            assert_eq!(silenced(command), command);
        }
    }
}

#[test]
fn the_led_byte_still_shows_the_granted_phantom_and_monitoring() {
    let command = Command::WriteExpanderOutputs(ExpanderOutputs::showing(
        both_granted(),
        [device::control_pass::Monitoring::On; 2],
    ));
    assert_eq!(silenced(command), command);
}
