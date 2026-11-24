use device::analog_word::AnalogWord;
use device::command::{Command, CommandList, CommandListFull};
use device::output_pin::OutputPin;
use device::phantom::PinLevel;
use device::pin::Pin;

#[test]
fn a_command_list_keeps_commands_in_push_order() {
    let mut list = CommandList::<3>::new();
    assert_eq!(
        list.push(Command::SendAnalogWord(AnalogWord::ALL_OFF)),
        Ok(())
    );
    assert_eq!(
        list.push(Command::DrivePin(OutputPin::PhantomActive, PinLevel::Low)),
        Ok(())
    );
    assert_eq!(list.push(Command::StopDma), Ok(()));
    assert_eq!(
        list.as_slice(),
        &[
            Command::SendAnalogWord(AnalogWord::ALL_OFF),
            Command::DrivePin(OutputPin::PhantomActive, PinLevel::Low),
            Command::StopDma,
        ]
    );
}

#[test]
fn a_full_command_list_refuses_another_command_and_keeps_what_it_holds() {
    let mut list = CommandList::<1>::new();
    assert_eq!(list.push(Command::AttachUsb), Ok(()));
    assert_eq!(list.push(Command::StopDma), Err(CommandListFull));
    assert_eq!(list.as_slice(), &[Command::AttachUsb]);
}

#[test]
fn extending_a_command_list_stops_at_capacity_with_an_error() {
    let mut list = CommandList::<2>::new();
    assert_eq!(
        list.extend([Command::StartDma, Command::StopDma, Command::AttachUsb]),
        Err(CommandListFull)
    );
    assert_eq!(list.as_slice(), &[Command::StartDma, Command::StopDma]);
}

#[test]
fn a_new_command_list_is_empty() {
    assert!(CommandList::<4>::new().as_slice().is_empty());
}

#[test]
fn output_pins_name_the_factory_pins_by_port_and_number() {
    let expected = [
        (OutputPin::PhantomActive, Pin { port: 1, number: 4 }),
        (
            OutputPin::FirstBringUp,
            Pin {
                port: 1,
                number: 22,
            },
        ),
        (
            OutputPin::SecondBringUp,
            Pin {
                port: 1,
                number: 26,
            },
        ),
        (
            OutputPin::ThirdBringUp,
            Pin {
                port: 1,
                number: 10,
            },
        ),
        (OutputPin::ConverterEnable, Pin { port: 1, number: 3 }),
        (
            OutputPin::AdcEnable,
            Pin {
                port: 1,
                number: 29,
            },
        ),
        (
            OutputPin::TransportActive,
            Pin {
                port: 0,
                number: 28,
            },
        ),
    ];
    for (output, pin) in expected {
        assert_eq!(output.pin(), pin, "{output:?}");
    }
    assert_eq!(OutputPin::ALL.len(), expected.len());
}
