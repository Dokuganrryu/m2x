use crate::adc::{AdcAddress, AdcKind, AdcWrite};
use crate::command::Command;
use crate::dac::{
    Attenuation, ConfigRegister, DacWrite, MasterTrim, StatusRegister, VolumeRegister,
};
use crate::output_pin::OutputPin;
use crate::phantom::{Milliseconds, PinLevel};
use crate::steps::ScriptStep;

const WAIT_AFTER_STATUS_READS: Milliseconds = Milliseconds(100);
const ADC_ENABLE_PULSE_LOW: Milliseconds = Milliseconds(5);

const fn run(command: Command) -> ScriptStep<Command> {
    ScriptStep::Emit(command)
}

const fn config(register: ConfigRegister) -> ScriptStep<Command> {
    run(Command::WriteDac(DacWrite::Config(register)))
}

const fn fully_attenuated(register: VolumeRegister) -> ScriptStep<Command> {
    run(Command::WriteDac(DacWrite::Volume(
        register,
        Attenuation::FULL,
    )))
}

const READ_STATUS: ScriptStep<Command> = run(Command::ReadDac(StatusRegister::Register113));

pub const SAFE_DAC_INIT: [ScriptStep<Command>; 32] = [
    config(ConfigRegister::Register2),
    config(ConfigRegister::Register1),
    config(ConfigRegister::Register7),
    config(ConfigRegister::Register10),
    config(ConfigRegister::Register12),
    config(ConfigRegister::Register13),
    config(ConfigRegister::Register38),
    config(ConfigRegister::Register39),
    config(ConfigRegister::Register40),
    config(ConfigRegister::Register41),
    config(ConfigRegister::Register8),
    config(ConfigRegister::Register46),
    run(Command::WriteDac(DacWrite::MasterTrim(
        MasterTrim::MINUS_ONE_DB,
    ))),
    config(ConfigRegister::Register47),
    config(ConfigRegister::Register48),
    config(ConfigRegister::Register49),
    config(ConfigRegister::Register50),
    config(ConfigRegister::Register51),
    config(ConfigRegister::Register52),
    config(ConfigRegister::Register63),
    READ_STATUS,
    READ_STATUS,
    READ_STATUS,
    ScriptStep::Hold(WAIT_AFTER_STATUS_READS),
    fully_attenuated(VolumeRegister::FactoryZeroA),
    fully_attenuated(VolumeRegister::FactoryZeroB),
    fully_attenuated(VolumeRegister::MainA),
    fully_attenuated(VolumeRegister::MainB),
    fully_attenuated(VolumeRegister::HeadphoneA),
    fully_attenuated(VolumeRegister::HeadphoneB),
    fully_attenuated(VolumeRegister::FactoryHeldA),
    fully_attenuated(VolumeRegister::FactoryHeldB),
];

const ADC_AT_13_INIT: [ScriptStep<Command>; 4] = [
    run(Command::WriteAdc(AdcWrite::At13Register0)),
    run(Command::WriteAdc(AdcWrite::At13Register1)),
    run(Command::WriteAdc(AdcWrite::At13Register2)),
    run(Command::WriteAdc(AdcWrite::At13Register4)),
];

const ADC_AT_4F_INIT: [ScriptStep<Command>; 1] = [run(Command::WriteAdc(AdcWrite::At4FRegister1))];

const ADC_UNANSWERED_INIT: [ScriptStep<Command>; 3] = [
    run(Command::DrivePin(OutputPin::AdcEnable, PinLevel::Low)),
    ScriptStep::Hold(ADC_ENABLE_PULSE_LOW),
    run(Command::DrivePin(OutputPin::AdcEnable, PinLevel::High)),
];

pub const fn adc_init(kind: AdcKind) -> &'static [ScriptStep<Command>] {
    match kind {
        AdcKind::Answered(AdcAddress::At13) => &ADC_AT_13_INIT,
        AdcKind::Answered(AdcAddress::At4F) => &ADC_AT_4F_INIT,
        AdcKind::Unanswered => &ADC_UNANSWERED_INIT,
    }
}
