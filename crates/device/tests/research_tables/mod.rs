#![allow(dead_code)]

use device::command::Command;
use device::dac::DacRegister;
use device::factory_init::FactoryStep;
use device::phantom::Milliseconds;
use device::pin::Pin;
use device::steps::ScriptStep;

pub fn table_after<'a>(note: &'a str, heading: &str) -> Vec<Vec<&'a str>> {
    let start = note
        .find(heading)
        .unwrap_or_else(|| panic!("the note has the heading {heading}"));
    note[start..]
        .lines()
        .skip_while(|line| !line.starts_with('|'))
        .take_while(|line| line.starts_with('|'))
        .skip(2)
        .map(|line| {
            line.trim_matches('|')
                .split('|')
                .map(str::trim)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn hex(text: &str) -> u8 {
    let digits = text
        .trim()
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("{text} is a hex byte"));
    u8::from_str_radix(digits, 16).unwrap_or_else(|_| panic!("{text} is a hex byte"))
}

fn decimal_list(text: &str) -> Vec<u8> {
    text.split(',')
        .map(|part| part.trim().parse().expect("a register number"))
        .collect()
}

fn hex_list(text: &str) -> Vec<u8> {
    text.split(',').map(hex).collect()
}

fn milliseconds(text: &str) -> Milliseconds {
    let count = text
        .trim()
        .strip_suffix(" ms")
        .unwrap_or_else(|| panic!("{text} is a delay in ms"));
    Milliseconds(count.parse().expect("a whole number of ms"))
}

fn pin(text: &str) -> Pin {
    let (port, number) = text
        .trim()
        .strip_prefix('P')
        .and_then(|rest| rest.split_once('_'))
        .unwrap_or_else(|| panic!("{text} is a pin name"));
    Pin {
        port: port.parse().expect("a port number"),
        number: number.parse().expect("a pin number"),
    }
}

pub fn bus_steps(script: &[ScriptStep<Command>]) -> Vec<FactoryStep> {
    script
        .iter()
        .flat_map(|step| match *step {
            ScriptStep::Hold(duration) => vec![FactoryStep::Wait(duration)],
            ScriptStep::Emit(command) => bus_steps_of(command),
        })
        .collect()
}

fn bus_steps_of(command: Command) -> Vec<FactoryStep> {
    match command {
        Command::WriteDac(write) => write
            .transactions()
            .as_slice()
            .iter()
            .map(|transaction| FactoryStep::Write {
                register: transaction.register.number(),
                value: transaction.value,
            })
            .collect(),
        Command::ReadDac(register) => vec![FactoryStep::Read {
            register: DacRegister::Status(register).number(),
        }],
        Command::WriteAdc(write) => vec![FactoryStep::Write {
            register: write.register(),
            value: write.value(),
        }],
        Command::DrivePin(output, level) => vec![FactoryStep::DrivePin {
            pin: output.pin(),
            level,
        }],
        other => panic!("{other:?} is not a bus, timer or pin step"),
    }
}
