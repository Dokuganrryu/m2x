mod research_tables;

use device::adc::{AdcAddress, AdcKind, AdcWrite, ProbeReply};
use device::command::Command;
use device::factory_init::{
    FACTORY_ADC_AT_4F_INIT, FACTORY_ADC_AT_13_INIT, FACTORY_ADC_UNANSWERED_INIT,
};
use device::safe_init::adc_init;
use device::steps::ScriptStep;
use research_tables::bus_steps;

#[test]
fn an_ack_at_0x13_selects_the_part_at_0x13_even_when_0x4f_also_acks() {
    for at_4f in [ProbeReply::Acked, ProbeReply::Nacked] {
        assert_eq!(
            AdcKind::from_probes(ProbeReply::Acked, at_4f),
            AdcKind::Answered(AdcAddress::At13)
        );
    }
}

#[test]
fn an_ack_only_at_0x4f_selects_the_part_at_0x4f() {
    assert_eq!(
        AdcKind::from_probes(ProbeReply::Nacked, ProbeReply::Acked),
        AdcKind::Answered(AdcAddress::At4F)
    );
}

#[test]
fn no_ack_leaves_the_adc_unanswered() {
    assert_eq!(
        AdcKind::from_probes(ProbeReply::Nacked, ProbeReply::Nacked),
        AdcKind::Unanswered
    );
}

#[test]
fn the_part_at_0x13_gets_registers_0_1_2_and_4_written_with_0x0f_0x01_0x7b_and_0x00() {
    let commands = adc_init(AdcKind::Answered(AdcAddress::At13));
    assert!(commands.iter().all(|step| matches!(
        step,
        ScriptStep::Emit(Command::WriteAdc(write)) if write.address() == AdcAddress::At13
    )));
    assert_eq!(bus_steps(commands), FACTORY_ADC_AT_13_INIT.to_vec());
}

#[test]
fn the_part_at_0x4f_gets_register_1_written_with_0xa7() {
    let commands = adc_init(AdcKind::Answered(AdcAddress::At4F));
    assert_eq!(
        commands,
        &[ScriptStep::Emit(Command::WriteAdc(AdcWrite::At4FRegister1))]
    );
    assert_eq!(AdcWrite::At4FRegister1.address().address(), 0x4F);
    assert_eq!(bus_steps(commands), FACTORY_ADC_AT_4F_INIT.to_vec());
}

#[test]
fn an_unanswered_adc_gets_no_bus_write_only_the_factory_enable_pin_pulse() {
    let commands = adc_init(AdcKind::Unanswered);
    assert!(commands.iter().all(|step| !matches!(
        step,
        ScriptStep::Emit(Command::WriteAdc(_) | Command::WriteDac(_))
    )));
    assert_eq!(bus_steps(commands), FACTORY_ADC_UNANSWERED_INIT.to_vec());
}
