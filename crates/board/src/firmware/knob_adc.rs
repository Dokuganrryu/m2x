use device::knob_scan::{Awaiting, Event, GainCalibrationCode, Output, ResultWord, ScanResults};

use super::clock_gates::{self, BusClock};
use super::map::{
    ADC0_CFG, ADC0_CFG_FACTORY_KEPT, ADC0_CFG_FACTORY_SET, ADC0_CTRL, ADC0_CTRL_ADCEN,
    ADC0_CTRL_CAL_REQ, ADC0_CTRL_CALOFS, ADC0_CTRL_FACTORY_SIXTEEN_CALIBRATION_AVERAGES,
    ADC0_CTRL_RST, ADC0_CTRL_RSTFIFO0, ADC0_CTRL_RSTFIFO1, ADC0_FACTORY_KNOB_COMMANDS,
    ADC0_FCTRL_FCOUNT_MASK, ADC0_FCTRL0, ADC0_FCTRL0_FACTORY_WATERMARK_FOUR, ADC0_GCC,
    ADC0_GCC_GAIN_CAL_MASK, ADC0_GCC_INDEX_STEP, ADC0_GCC_RDY, ADC0_GCR, ADC0_GCR_INDEX_STEP,
    ADC0_PAUSE, ADC0_PAUSE_OFF, ADC0_RESFIFO0, ADC0_STAT, ADC0_STAT_CAL_RDY, ADC0_STAT_RDY0,
    ADC0_SWTRIG, ADC0_SWTRIG_SWT0, ADC0_TCTRL, ADC0_TCTRL_FACTORY_TRIGGERS_START_COMMANDS_1_TO_4,
    ADC0_TCTRL_INDEX_STEP, GPIO_DIRCLR, GPIO_DIRCLR0_FACTORY_KNOB_INPUTS,
    IOCON_FACTORY_ANALOG_INPUT, IOCON_PIO0_10, IOCON_PIO0_15, IOCON_PIO0_16, IOCON_PIO0_23,
    IOCON_PIO0_31, PMC_PDRUNCFG0_PDEN_AUXBIAS, PMC_PDRUNCFGCLR0, SYSCON_ADCCLKDIV,
    SYSCON_ADCCLKDIV_FACTORY_DIVIDE_BY_SIX, SYSCON_ADCCLKSEL, SYSCON_ADCCLKSEL_FACTORY_FRO_96_MHZ,
    SYSCON_AHBCLKCTRL0_ADC, SYSCON_AHBCLKCTRLSET0, SYSCON_PRESETCTRL0, SYSCON_PRESETCTRL0_ADC_RST,
};

const KNOB_PINS: [super::mmio::Register; 5] = [
    IOCON_PIO0_23,
    IOCON_PIO0_10,
    IOCON_PIO0_15,
    IOCON_PIO0_31,
    IOCON_PIO0_16,
];
const ADC_RESETS: u32 = ADC0_CTRL_RST | ADC0_CTRL_RSTFIFO0 | ADC0_CTRL_RSTFIFO1;

pub fn start_with_offset_calibration() {
    PMC_PDRUNCFGCLR0.write(PMC_PDRUNCFG0_PDEN_AUXBIAS);
    SYSCON_ADCCLKSEL.write(SYSCON_ADCCLKSEL_FACTORY_FRO_96_MHZ);
    SYSCON_ADCCLKDIV.write(SYSCON_ADCCLKDIV_FACTORY_DIVIDE_BY_SIX);
    SYSCON_AHBCLKCTRLSET0.write(SYSCON_AHBCLKCTRL0_ADC);
    SYSCON_PRESETCTRL0.set(SYSCON_PRESETCTRL0_ADC_RST);
    SYSCON_PRESETCTRL0.write(SYSCON_PRESETCTRL0.read() & !SYSCON_PRESETCTRL0_ADC_RST);
    clock_gates::enable(&[BusClock::Iocon, BusClock::Gpio0]);
    for pin in KNOB_PINS {
        pin.write(IOCON_FACTORY_ANALOG_INPUT);
    }
    GPIO_DIRCLR.write(GPIO_DIRCLR0_FACTORY_KNOB_INPUTS);
    ADC0_CTRL.write(ADC_RESETS);
    ADC0_CTRL.write(ADC0_CTRL.read() & !ADC_RESETS);
    ADC0_CFG.write((ADC0_CFG.read() & ADC0_CFG_FACTORY_KEPT) | ADC0_CFG_FACTORY_SET);
    ADC0_PAUSE.write(ADC0_PAUSE_OFF);
    ADC0_CTRL.set(ADC0_CTRL_ADCEN);
    ADC0_CTRL
        .write(ADC0_CTRL_ADCEN | ADC0_CTRL_CALOFS | ADC0_CTRL_FACTORY_SIXTEEN_CALIBRATION_AVERAGES);
}

fn gain_calibration(index: usize) -> Option<GainCalibrationCode> {
    let word = ADC0_GCC.bytes_after(index * ADC0_GCC_INDEX_STEP).read();
    (word & ADC0_GCC_RDY != 0)
        .then_some(GainCalibrationCode((word & ADC0_GCC_GAIN_CAL_MASK) as u16))
}

pub fn poll(awaiting: Awaiting) -> Option<Event> {
    match awaiting {
        Awaiting::CalibrationReady => {
            (ADC0_STAT.read() & ADC0_STAT_CAL_RDY != 0).then_some(Event::CalibrationReady)
        }
        Awaiting::GainCalibration => Some(Event::GainCalibrated([
            gain_calibration(0)?,
            gain_calibration(1)?,
        ])),
        Awaiting::Results => {
            if ADC0_STAT.read() & ADC0_STAT_RDY0 == 0 {
                return None;
            }
            let waiting = ADC0_FCTRL0.read() & ADC0_FCTRL_FCOUNT_MASK;
            let results = (0..waiting).fold(ScanResults::EMPTY, |results, _| {
                results.with(ResultWord(ADC0_RESFIFO0.read()))
            });
            Some(Event::ResultsReady(results))
        }
    }
}

pub fn act(output: Output) {
    match output {
        Output::RequestGainCalibration => ADC0_CTRL.set(ADC0_CTRL_CAL_REQ),
        Output::WriteGainCorrections(corrections) => {
            for (index, correction) in corrections.into_iter().enumerate() {
                ADC0_GCR
                    .bytes_after(index * ADC0_GCR_INDEX_STEP)
                    .write(correction.0);
            }
        }
        Output::ConfigureCommands => {
            for (low, low_word, high, high_word) in ADC0_FACTORY_KNOB_COMMANDS {
                low.write(low_word);
                high.write(high_word);
            }
            for (index, word) in ADC0_TCTRL_FACTORY_TRIGGERS_START_COMMANDS_1_TO_4
                .into_iter()
                .enumerate()
            {
                ADC0_TCTRL
                    .bytes_after(index * ADC0_TCTRL_INDEX_STEP)
                    .write(word);
            }
            ADC0_FCTRL0.write(ADC0_FCTRL0_FACTORY_WATERMARK_FOUR);
        }
        Output::Trigger => ADC0_SWTRIG.write(ADC0_SWTRIG_SWT0),
        Output::Readings(_) => {}
    }
}
