use device::adc::AdcKind;
use device::boot::{self, Outcome, Stage, State};
use device::breadcrumb::{At, Breadcrumb, MAGIC, Place, Weight};
use device::fault::{FaultCode, I2cPeer};

const fn place(stage: u8, step: u8) -> Place {
    Place { stage, step }
}

const NOTED_DAC_AT_8_3: u32 = 0x0800_0000 | 3 << 16 | 3 << 8 | 8;
const RAISED_ADC_AT_13_2: u32 = 0x0D00_0000 | 1 << 23 | 4 << 16 | 2 << 8 | 13;

#[test]
fn a_word_without_the_magic_gives_no_previous_record() {
    let stale_words = [
        [0, 0],
        [u32::MAX, u32::MAX],
        [MAGIC ^ 1, RAISED_ADC_AT_13_2],
        [MAGIC.swap_bytes(), RAISED_ADC_AT_13_2],
        [RAISED_ADC_AT_13_2, MAGIC],
    ];
    for words in stale_words {
        assert_eq!(Breadcrumb::previous(words), None, "{words:#x?}");
    }
    assert_eq!(
        Breadcrumb::previous([MAGIC, RAISED_ADC_AT_13_2]),
        Some(Breadcrumb {
            place: place(13, 2),
            code: 4,
            weight: Weight::Raised,
            latest: 13,
        })
    );
    assert!(![0, u32::MAX].contains(&MAGIC));
}

#[test]
fn a_boot_starts_at_place_one_with_no_code() {
    assert_eq!(
        Breadcrumb::START,
        Breadcrumb {
            place: place(1, 0),
            code: 0,
            weight: Weight::Noted,
            latest: 1,
        }
    );
    assert_eq!(Breadcrumb::START.pack(), 0x0100_0001);
}

#[test]
fn marks_before_a_fault_move_the_place_and_the_latest_place() {
    let marked = Breadcrumb::START.marked(place(1, 12)).marked(place(2, 3));
    assert_eq!(
        marked,
        Breadcrumb {
            place: place(2, 3),
            code: 0,
            weight: Weight::Noted,
            latest: 2,
        }
    );
}

#[test]
fn the_first_fault_freezes_the_place_and_a_later_fault_changes_nothing() {
    let first = Breadcrumb::START.marked(place(11, 4)).recorded(
        FaultCode::Unanswered(I2cPeer::Dac),
        At::Here,
        Weight::Raised,
    );
    let frozen = Breadcrumb {
        place: place(11, 4),
        code: 3,
        weight: Weight::Raised,
        latest: 11,
    };
    assert_eq!(first, frozen);
    let later = [
        (FaultCode::RefusedCommand, At::Here, Weight::Raised),
        (
            FaultCode::BoardStalled,
            At::Step(place(12, 0)),
            Weight::Raised,
        ),
        (FaultCode::HardFault, At::Here, Weight::Raised),
        (
            FaultCode::Unanswered(I2cPeer::Expander),
            At::Step(place(14, 0)),
            Weight::Noted,
        ),
    ];
    let mut crumb = first;
    for (code, at, weight) in later {
        crumb = crumb.recorded(code, at, weight);
        assert_eq!(crumb, frozen, "{code:?} {at:?} {weight:?}");
    }
}

#[test]
fn a_boot_stage_fault_is_recorded_at_its_own_stage_and_step() {
    let waiting_in_the_next_stage = Breadcrumb::START.marked(place(4, 1));
    let recorded = waiting_in_the_next_stage.recorded(
        FaultCode::CrystalNotReady,
        At::Step(place(2, 2)),
        Weight::Raised,
    );
    assert_eq!(
        recorded,
        Breadcrumb {
            place: place(2, 2),
            code: 12,
            weight: Weight::Raised,
            latest: 4,
        }
    );
}

#[test]
fn a_raised_fault_replaces_a_noted_code_and_its_place() {
    let noted = Breadcrumb::START.marked(place(8, 4)).recorded(
        FaultCode::Unanswered(I2cPeer::Dac),
        At::Step(place(8, 3)),
        Weight::Noted,
    );
    assert_eq!(
        noted,
        Breadcrumb {
            place: place(8, 3),
            code: 3,
            weight: Weight::Noted,
            latest: 8,
        }
    );
    assert_eq!(noted.pack(), NOTED_DAC_AT_8_3);

    let raised_at_its_step = noted.marked(place(17, 0)).recorded(
        FaultCode::Unanswered(I2cPeer::Adc),
        At::Step(place(13, 2)),
        Weight::Raised,
    );
    assert_eq!(
        raised_at_its_step,
        Breadcrumb {
            place: place(13, 2),
            code: 4,
            weight: Weight::Raised,
            latest: 17,
        }
    );

    let raised_here =
        noted
            .marked(place(11, 6))
            .recorded(FaultCode::HardFault, At::Here, Weight::Raised);
    assert_eq!(
        raised_here,
        Breadcrumb {
            place: place(11, 0),
            code: 8,
            weight: Weight::Raised,
            latest: 11,
        },
        "the noted code froze the step, so a fault with no place of its own takes the latest stage"
    );

    let raised = Breadcrumb::START.marked(place(13, 2)).recorded(
        FaultCode::Unanswered(I2cPeer::Adc),
        At::Here,
        Weight::Raised,
    );
    let noted_after = raised.marked(place(14, 1)).recorded(
        FaultCode::Unanswered(I2cPeer::Expander),
        At::Step(place(14, 0)),
        Weight::Noted,
    );
    assert_eq!(noted_after, raised.marked(place(14, 1)));
    assert_eq!((noted_after.code, noted_after.place), (4, place(13, 2)));

    let second_note = noted.recorded(
        FaultCode::Unanswered(I2cPeer::Expander),
        At::Step(place(14, 0)),
        Weight::Noted,
    );
    assert_eq!(
        second_note, noted,
        "a noted code never replaces a noted code"
    );
}

#[test]
fn marks_after_a_fault_move_only_the_latest_place() {
    for weight in [Weight::Noted, Weight::Raised] {
        let faulted = Breadcrumb::START.marked(place(7, 1)).recorded(
            FaultCode::AudioPllUnlocked,
            At::Step(place(7, 1)),
            weight,
        );
        let marked = faulted.marked(place(14, 2)).marked(place(17, 0));
        assert_eq!(
            marked,
            Breadcrumb {
                place: place(7, 1),
                code: 14,
                weight,
                latest: 17,
            }
        );
    }
}

#[test]
fn the_packed_words_round_trip_every_place_step_code_and_weight() {
    for code in FaultCode::ALL {
        for weight in [Weight::Noted, Weight::Raised] {
            for stage in 0..=u8::MAX {
                for step in 0..=u8::MAX {
                    let latest = stage ^ step;
                    let crumb = Breadcrumb::START.marked(place(latest, 0)).recorded(
                        code,
                        At::Step(place(stage, step)),
                        weight,
                    );
                    let word = crumb.pack();
                    assert_eq!(Breadcrumb::unpack(word), crumb);
                    assert_eq!(Breadcrumb::unpack(word).code, code.number());
                    assert_eq!(Breadcrumb::unpack(word).weight, weight);
                    assert_eq!(word & 0xFF, u32::from(stage));
                    assert_eq!(word >> 8 & 0xFF, u32::from(step));
                    assert_eq!(word >> 16 & 0x7F, u32::from(code.number()));
                    assert_eq!(word >> 23 & 1, u32::from(weight == Weight::Raised));
                    assert_eq!(word >> 24, u32::from(latest));
                }
            }
        }
    }
    for word in [0, 1, 0x0100_0001, 0x00FF_FFFF, 0x1234_5678, u32::MAX] {
        assert_eq!(Breadcrumb::unpack(word).pack(), word, "{word:#x}");
    }
    assert_eq!(
        Breadcrumb::unpack(RAISED_ADC_AT_13_2),
        Breadcrumb {
            place: place(13, 2),
            code: 4,
            weight: Weight::Raised,
            latest: 13,
        }
    );
}

#[test]
fn place_numbers_and_fault_codes_are_pinned() {
    let kind = AdcKind::Unanswered;
    let stages = [
        (Stage::Quiesce, 1),
        (Stage::Clocks, 2),
        (Stage::UsbAttach, 3),
        (Stage::AnalogOff, 4),
        (Stage::Rails, 5),
        (Stage::StartLcd, 6),
        (Stage::AudioClocks, 7),
        (Stage::DacAttenuate, 8),
        (Stage::AdcProbe(kind), 9),
        (Stage::TransportStart(kind), 10),
        (Stage::DacInit(kind), 11),
        (Stage::OutputConnect(kind), 12),
        (Stage::AdcInit(kind), 13),
        (Stage::Panel(kind), 14),
    ];
    for (stage, number) in stages {
        assert_eq!(stage.number(), number, "{stage:?}");
    }
    assert_eq!(State::Running(kind, Outcome::START).place(), place(16, 0));
    assert_eq!(State::Degraded(Outcome::START).place(), place(17, 0));
    let (first, outputs) = boot::begin();
    assert_eq!(first.place(), place(1, outputs.as_slice().len() as u8));

    let codes = [
        (FaultCode::Unanswered(I2cPeer::Dac), 3),
        (FaultCode::Unanswered(I2cPeer::Adc), 4),
        (FaultCode::Unanswered(I2cPeer::Expander), 5),
        (FaultCode::BoardStalled, 6),
        (FaultCode::Panic, 7),
        (FaultCode::HardFault, 8),
        (FaultCode::UnexpectedException, 9),
        (FaultCode::FenceUnavailable, 10),
        (FaultCode::RefusedCommand, 11),
        (FaultCode::CrystalNotReady, 12),
        (FaultCode::CorePllUnlocked, 13),
        (FaultCode::AudioPllUnlocked, 14),
        (FaultCode::UsbPhyPllUnlocked, 16),
        #[cfg(feature = "sound3")]
        (FaultCode::CoreClockHeld, 17),
    ];
    assert_eq!(codes.map(|(code, _)| code), FaultCode::ALL);
    for (code, number) in codes {
        assert_eq!(code.number(), number, "{code:?}");
        let recorded = Breadcrumb::START.recorded(code, At::Here, Weight::Raised);
        assert_eq!(recorded.code, number, "{code:?}");
    }
}
