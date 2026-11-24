use device::dac::{
    Attenuation, ConfigRegister, DAC_ADDRESS, DAC_FLEXCOMM, DacRegister, DacTransaction, DacWrite,
    MasterTrim, VolumeRegister,
};
use device::factory_init::{FACTORY_DAC_INIT, FactoryStep};
use std::collections::BTreeSet;

fn numbered(write: DacWrite) -> Vec<(u8, u8)> {
    write
        .transactions()
        .as_slice()
        .iter()
        .map(|transaction: &DacTransaction| (transaction.register.number(), transaction.value))
        .collect()
}

#[test]
fn the_dac_sits_on_flexcomm_three_at_address_0x49() {
    assert_eq!(DAC_FLEXCOMM, 3);
    assert_eq!(DAC_ADDRESS, 0x49);
}

#[test]
fn the_dac_register_type_lists_exactly_the_registers_the_factory_init_touches_except_register_zero()
{
    let listed: BTreeSet<u8> = DacRegister::ALL.iter().map(|r| r.number()).collect();
    let touched_by_factory: BTreeSet<u8> = FACTORY_DAC_INIT
        .iter()
        .filter_map(|step| match *step {
            FactoryStep::Write { register, .. } | FactoryStep::Read { register } => Some(register),
            FactoryStep::Wait(_) | FactoryStep::DrivePin { .. } => None,
        })
        .filter(|&register| register != 0)
        .collect();
    assert_eq!(listed, touched_by_factory);
    assert_eq!(listed.len(), DacRegister::ALL.len());
    assert!(!listed.contains(&0));
    assert!(!listed.contains(&62));
}

#[test]
fn main_trim_is_registers_18_and_19_and_headphone_trim_is_registers_20_and_21() {
    assert_eq!(
        [VolumeRegister::MainA, VolumeRegister::MainB].map(|r| DacRegister::Volume(r).number()),
        [18, 19]
    );
    assert_eq!(
        [VolumeRegister::HeadphoneA, VolumeRegister::HeadphoneB]
            .map(|r| DacRegister::Volume(r).number()),
        [20, 21]
    );
}

#[test]
fn volume_is_counted_in_half_decibel_steps_from_zero_db_to_full_attenuation_at_0xff() {
    assert_eq!(Attenuation::ZERO_DB.half_decibel_steps(), 0x00);
    assert_eq!(Attenuation::FULL.half_decibel_steps(), 0xFF);
    assert_eq!(
        f64::from(Attenuation::FULL.half_decibel_steps()) * 0.5,
        127.5
    );
    assert_eq!(
        Attenuation::from_half_decibel_steps(0xFF),
        Attenuation::FULL
    );
}

#[test]
fn a_volume_write_is_one_byte_in_one_transaction() {
    assert_eq!(
        numbered(DacWrite::Volume(
            VolumeRegister::HeadphoneB,
            Attenuation::from_half_decibel_steps(12)
        )),
        vec![(21, 12)]
    );
}

#[test]
fn a_config_write_sends_the_factory_value_for_its_register() {
    assert_eq!(
        numbered(DacWrite::Config(ConfigRegister::Register2)),
        vec![(2, 0x1C)]
    );
    assert_eq!(
        numbered(DacWrite::Config(ConfigRegister::Register8)),
        vec![(8, 0x8D)]
    );
    assert_eq!(
        numbered(DacWrite::Config(ConfigRegister::Register63)),
        vec![(63, 0x4A)]
    );
}

#[test]
fn master_trim_is_held_at_minus_one_db_and_cannot_be_made_louder() {
    assert_eq!(MasterTrim::MINUS_ONE_DB.raw(), 0x7214_82BF);
    assert_eq!(MasterTrim::new(0x7214_82BF), Some(MasterTrim::MINUS_ONE_DB));
    assert_eq!(MasterTrim::new(0x7214_82C0), None);
    assert_eq!(MasterTrim::new(0x7FFF_FFFF), None);
    assert_eq!(MasterTrim::new(u32::MAX), None);
    assert!(MasterTrim::new(0x4000_0000).is_some());
}

#[test]
fn a_master_trim_write_sends_registers_24_to_27_low_byte_first_one_byte_per_transaction() {
    assert_eq!(
        numbered(DacWrite::MasterTrim(MasterTrim::MINUS_ONE_DB)),
        vec![(24, 0xBF), (25, 0x82), (26, 0x14), (27, 0x72)]
    );
}
