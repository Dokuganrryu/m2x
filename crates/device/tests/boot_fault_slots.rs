mod boot_board;

use boot_board::{Bench, adc_init_write, first_attenuation_write, run_bench};
use device::boot::{ClockInput, PllLock};
use device::breadcrumb::{At, Breadcrumb, Place, Weight};
use device::fault::{FaultCode, I2cPeer};

const ENOUGH_PASSES: usize = 5_000;

const DAC: FaultCode = FaultCode::Unanswered(I2cPeer::Dac);
const ADC: FaultCode = FaultCode::Unanswered(I2cPeer::Adc);
const EXPANDER: FaultCode = FaultCode::Unanswered(I2cPeer::Expander);

const FIRST_ATTENUATION_WRITE: Place = Place { stage: 8, step: 0 };
const FIRST_ADC_INIT_WRITE: Place = Place { stage: 13, step: 0 };
const EXPANDER_DIRECTION_WRITE: Place = Place { stage: 14, step: 0 };
const CRYSTAL_READ: Place = Place { stage: 2, step: 1 };
const USB_ATTACH: Place = Place { stage: 3, step: 0 };

const RUNNING: u8 = 16;
const DEGRADED: u8 = 17;

struct Case {
    what: &'static str,
    bench: Bench,
    records: Vec<(FaultCode, At, Weight)>,
    crumb: Breadcrumb,
}

fn kept(code: FaultCode, place: Place, weight: Weight, latest: u8) -> Breadcrumb {
    Breadcrumb {
        place,
        code: code.number(),
        weight,
        latest,
    }
}

#[test]
fn the_first_boot_fault_is_the_one_recorded() {
    let cases = [
        Case {
            what: "a clean boot records nothing",
            bench: Bench::ALL_ACK,
            records: vec![],
            crumb: Breadcrumb {
                place: Place {
                    stage: RUNNING,
                    step: 0,
                },
                code: 0,
                weight: Weight::Noted,
                latest: RUNNING,
            },
        },
        Case {
            what: "a raised fault alone",
            bench: Bench {
                unanswered_transfer: Some(adc_init_write()),
                ..Bench::ALL_ACK
            },
            records: vec![(ADC, At::Step(FIRST_ADC_INIT_WRITE), Weight::Raised)],
            crumb: kept(ADC, FIRST_ADC_INIT_WRITE, Weight::Raised, DEGRADED),
        },
        Case {
            what: "the crystal fault, with the ladder's stages after it",
            bench: Bench {
                clock_input: ClockInput::NotReady,
                ..Bench::ALL_ACK
            },
            records: vec![(
                FaultCode::CrystalNotReady,
                At::Step(CRYSTAL_READ),
                Weight::Raised,
            )],
            crumb: kept(
                FaultCode::CrystalNotReady,
                CRYSTAL_READ,
                Weight::Raised,
                DEGRADED,
            ),
        },
        Case {
            what: "a noted skip, then a raised fault",
            bench: Bench {
                unanswered_transfer: Some(first_attenuation_write()),
                also_unanswered: Some(adc_init_write()),
                ..Bench::ALL_ACK
            },
            records: vec![
                (DAC, At::Step(FIRST_ATTENUATION_WRITE), Weight::Noted),
                (ADC, At::Step(FIRST_ADC_INIT_WRITE), Weight::Raised),
            ],
            crumb: kept(ADC, FIRST_ADC_INIT_WRITE, Weight::Raised, DEGRADED),
        },
        Case {
            what: "a raised fault, then a noted panel fault",
            bench: Bench {
                unanswered_transfer: Some(adc_init_write()),
                expander_silent: true,
                ..Bench::ALL_ACK
            },
            records: vec![
                (ADC, At::Step(FIRST_ADC_INIT_WRITE), Weight::Raised),
                (EXPANDER, At::Step(EXPANDER_DIRECTION_WRITE), Weight::Noted),
            ],
            crumb: kept(ADC, FIRST_ADC_INIT_WRITE, Weight::Raised, DEGRADED),
        },
        Case {
            what: "two noted faults",
            bench: Bench {
                unanswered_transfer: Some(first_attenuation_write()),
                expander_silent: true,
                ..Bench::ALL_ACK
            },
            records: vec![
                (DAC, At::Step(FIRST_ATTENUATION_WRITE), Weight::Noted),
                (EXPANDER, At::Step(EXPANDER_DIRECTION_WRITE), Weight::Noted),
            ],
            crumb: kept(DAC, FIRST_ATTENUATION_WRITE, Weight::Noted, RUNNING),
        },
        Case {
            what: "a noted USB PHY fault, then a raised fault",
            bench: Bench {
                usb_phy_pll_lock: PllLock::TimedOut,
                unanswered_transfer: Some(adc_init_write()),
                ..Bench::ALL_ACK
            },
            records: vec![
                (
                    FaultCode::UsbPhyPllUnlocked,
                    At::Step(USB_ATTACH),
                    Weight::Noted,
                ),
                (ADC, At::Step(FIRST_ADC_INIT_WRITE), Weight::Raised),
            ],
            crumb: kept(ADC, FIRST_ADC_INIT_WRITE, Weight::Raised, DEGRADED),
        },
    ];
    for case in cases {
        let caller = run_bench(case.bench, ENOUGH_PASSES);
        assert_eq!(caller.records(), case.records, "{}: the records", case.what);
        assert_eq!(caller.crumb, case.crumb, "{}: the breadcrumb", case.what);
    }
}

#[test]
fn a_ramp_fault_is_recorded_at_the_volume_ramp() {
    let bench = Bench {
        dac_stops_at_ramp_write: Some(3),
        ..Bench::ALL_ACK
    };
    let caller = run_bench(bench, ENOUGH_PASSES);
    let volume_ramp = Place { stage: 15, step: 0 };
    assert_eq!(
        caller.records(),
        vec![(DAC, At::Step(volume_ramp), Weight::Raised)]
    );
    assert_eq!(
        caller.crumb,
        kept(DAC, volume_ramp, Weight::Raised, DEGRADED)
    );
}
