mod research_tables;

use device::command::Command;
use device::dac::{Attenuation, DacRegister, DacWrite, MasterTrim, VolumeRegister};
use device::factory_init::{FACTORY_DAC_INIT, FactoryStep};
use device::safe_init::SAFE_DAC_INIT;
use device::steps::ScriptStep;
use research_tables::bus_steps;

const FACTORY_REGISTER_ZERO_WRITE: FactoryStep = FactoryStep::Write {
    register: 0,
    value: 0x00,
};

const FACTORY_TRIM_REWRITE_TO_ZERO_DB: [FactoryStep; 4] = [
    FactoryStep::Write {
        register: 24,
        value: 0xFF,
    },
    FactoryStep::Write {
        register: 25,
        value: 0xFF,
    },
    FactoryStep::Write {
        register: 26,
        value: 0xFF,
    },
    FactoryStep::Write {
        register: 27,
        value: 0x7F,
    },
];

fn writes_to(steps: &[FactoryStep], wanted: u8) -> Vec<u8> {
    steps
        .iter()
        .filter_map(|step| match *step {
            FactoryStep::Write { register, value } if register == wanted => Some(value),
            FactoryStep::Write { .. }
            | FactoryStep::Read { .. }
            | FactoryStep::Wait(_)
            | FactoryStep::DrivePin { .. } => None,
        })
        .collect()
}

#[test]
fn the_safe_dac_init_skips_the_factory_register_zero_write() {
    assert!(writes_to(&bus_steps(&SAFE_DAC_INIT), 0).is_empty());
    assert_eq!(writes_to(&FACTORY_DAC_INIT, 0), vec![0x00]);
}

#[test]
fn the_safe_dac_init_writes_master_trim_once_at_minus_one_db_and_never_rewrites_it_to_zero_db() {
    let trim_writes: Vec<_> = SAFE_DAC_INIT
        .iter()
        .filter(|step| {
            matches!(
                step,
                ScriptStep::Emit(Command::WriteDac(DacWrite::MasterTrim(_)))
            )
        })
        .collect();
    assert_eq!(
        trim_writes,
        vec![&ScriptStep::Emit(Command::WriteDac(DacWrite::MasterTrim(
            MasterTrim::MINUS_ONE_DB
        )))]
    );
    let safe = bus_steps(&SAFE_DAC_INIT);
    for register in 24..=27 {
        assert_eq!(writes_to(&safe, register).len(), 1, "register {register}");
        assert_eq!(
            writes_to(&FACTORY_DAC_INIT, register).len(),
            2,
            "register {register}"
        );
    }
}

#[test]
fn the_safe_dac_init_leaves_every_volume_register_at_full_attenuation_where_factory_sets_16_and_17_to_zero_db()
 {
    let safe = bus_steps(&SAFE_DAC_INIT);
    for register in VolumeRegister::ALL {
        let number = DacRegister::Volume(register).number();
        assert_eq!(writes_to(&safe, number), vec![0xFF], "register {number}");
    }
    assert_eq!(writes_to(&FACTORY_DAC_INIT, 16), vec![0x00]);
    assert_eq!(writes_to(&FACTORY_DAC_INIT, 17), vec![0x00]);
    assert!(!SAFE_DAC_INIT.iter().any(|step| matches!(
        step,
        ScriptStep::Emit(Command::WriteDac(DacWrite::Volume(_, level))) if *level != Attenuation::FULL
    )));
}

#[test]
fn the_safe_dac_init_keeps_the_three_register_113_reads_after_the_config_writes() {
    let safe = bus_steps(&SAFE_DAC_INIT);
    let reads: Vec<usize> = safe
        .iter()
        .enumerate()
        .filter(|(_, step)| matches!(step, FactoryStep::Read { register: 113 }))
        .map(|(at, _)| at)
        .collect();
    assert_eq!(reads.len(), 3);
    let last_config_write = safe
        .iter()
        .rposition(|step| matches!(step, FactoryStep::Write { register: 63, .. }))
        .expect("register 63 is written");
    assert!(reads.iter().all(|&at| at > last_config_write));
}

#[test]
fn apart_from_the_named_differences_the_safe_dac_init_is_the_factory_order_step_for_step() {
    let expected: Vec<FactoryStep> = FACTORY_DAC_INIT
        .iter()
        .copied()
        .filter(|step| *step != FACTORY_REGISTER_ZERO_WRITE)
        .filter(|step| !FACTORY_TRIM_REWRITE_TO_ZERO_DB.contains(step))
        .map(|step| match step {
            FactoryStep::Write {
                register: register @ (16 | 17),
                value: 0x00,
            } => FactoryStep::Write {
                register,
                value: 0xFF,
            },
            other => other,
        })
        .collect();
    assert_eq!(bus_steps(&SAFE_DAC_INIT), expected);
}

#[test]
fn every_config_write_in_the_safe_init_carries_the_value_factory_writes_to_that_register() {
    let factory = &FACTORY_DAC_INIT;
    for step in SAFE_DAC_INIT {
        if let ScriptStep::Emit(Command::WriteDac(DacWrite::Config(register))) = step {
            let number = DacRegister::Config(register).number();
            assert_eq!(
                writes_to(factory, number),
                vec![register.factory_value()],
                "register {number}"
            );
        }
    }
}
