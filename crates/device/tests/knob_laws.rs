use device::analog_word::PreampCode;
use device::dac::Attenuation;
use device::knob::KnobPosition;
use device::knob_law::{MonitorBlend, PreampGain, monitor_blend, preamp_gain, volume_attenuation};
use device::phantom::Jack;
use dsp::Gain;

const Q5_27_UNITY: f64 = (1u64 << 27) as f64;

fn every_position() -> impl DoubleEndedIterator<Item = KnobPosition> {
    (0..=240).map(|k| KnobPosition::new(k).unwrap())
}

fn at(k: u8) -> KnobPosition {
    KnobPosition::new(k).unwrap()
}

fn q5_27_of_decibels(decibels: f64) -> Gain {
    Gain::from_q5_27((10f64.powf(decibels / 20.0) * Q5_27_UNITY).round() as i32)
}

fn factory_trim_attenuation(k: u32) -> u8 {
    let cache = k;
    let att = if cache <= 204 {
        k * 46 / 100
    } else {
        (k * 45 / 10 - 57) & 0xFF
    };
    att as u8
}

#[test]
fn the_trim_law_matches_the_factory_formula_at_every_position() {
    for position in every_position() {
        assert_eq!(
            volume_attenuation(position),
            Attenuation::from_half_decibel_steps(factory_trim_attenuation(u32::from(
                position.value()
            ))),
            "position {}",
            position.value()
        );
    }
}

#[test]
fn the_trim_law_hits_the_factory_anchors_from_0_db_to_full_attenuation() {
    let steps = |k| volume_attenuation(at(k)).half_decibel_steps();
    assert_eq!(steps(0), 0);
    assert_eq!(steps(204), 93);
    assert_eq!(steps(205), 97);
    assert_eq!(steps(240), 255);
}

#[test]
fn the_trim_law_never_gets_louder_as_the_position_rises() {
    let levels: Vec<u8> = every_position()
        .map(|p| volume_attenuation(p).half_decibel_steps())
        .collect();
    assert!(levels.windows(2).all(|pair| pair[0] <= pair[1]));
}

fn factory_preamp(k: u32, jack: Jack) -> (u8, Gain) {
    let pin_offset = match jack {
        Jack::Removed => 6,
        Jack::Inserted => 9,
    };
    let g = (((240 - k) >> 2) + pin_offset).min(66);
    let (code, digital) = if g < 42 {
        (g / 3, g - 3 * (g / 3))
    } else {
        (14, g - 42)
    };
    (code as u8, q5_27_of_decibels(f64::from(digital)))
}

#[test]
fn the_preamp_law_matches_the_factory_formula_at_every_position_with_and_without_a_jack() {
    for jack in [Jack::Removed, Jack::Inserted] {
        for position in every_position() {
            let (code, digital) = factory_preamp(u32::from(position.value()), jack);
            assert_eq!(
                preamp_gain(position, jack),
                PreampGain {
                    code: PreampCode::new(code).unwrap(),
                    digital,
                },
                "position {} {jack:?}",
                position.value()
            );
        }
    }
}

#[test]
fn the_preamp_law_spans_analog_codes_2_to_14_and_a_digital_remainder_of_0_to_24_db() {
    let lowest = preamp_gain(at(240), Jack::Removed);
    let lowest_with_jack = preamp_gain(at(240), Jack::Inserted);
    let highest = preamp_gain(at(0), Jack::Removed);
    assert_eq!(lowest.code.value(), 2);
    assert_eq!(lowest.digital, Gain::UNITY);
    assert_eq!(lowest_with_jack.code.value(), 3);
    assert_eq!(highest.code.value(), 14);
    assert_eq!(highest.digital, Gain::from_q5_27(0x7ECA_9CD2));
    assert_eq!(
        preamp_gain(at(16), Jack::Removed).digital,
        Gain::from_q5_27(0x5000_0000)
    );
}

#[test]
fn the_total_preamp_gain_never_falls_as_the_knob_turns_toward_zero() {
    for jack in [Jack::Removed, Jack::Inserted] {
        let totals: Vec<u8> = every_position()
            .rev()
            .map(|p| preamp_gain(p, jack))
            .map(|g| 3 * g.code.value() + whole_decibels(g.digital))
            .collect();
        assert!(totals.windows(2).all(|pair| pair[0] <= pair[1]), "{jack:?}");
    }
}

fn whole_decibels(gain: Gain) -> u8 {
    (0..=24)
        .find(|decibels| q5_27_of_decibels(f64::from(*decibels)) == gain)
        .expect("a whole-decibel digital gain")
}

fn factory_blend(m: u32) -> MonitorBlend {
    let a = 240 - m;
    let decibels = |x: u32, steep: bool| {
        if steep {
            0.515 * f64::from(x) - 60.0
        } else {
            0.025 * f64::from(x) - 6.0
        }
    };
    let inputs = if m == 240 {
        Gain::ZERO
    } else {
        q5_27_of_decibels(decibels(a, m > 129))
    };
    let playback = if m == 0 {
        Gain::ZERO
    } else {
        q5_27_of_decibels(decibels(m, m <= 110))
    };
    MonitorBlend { inputs, playback }
}

#[test]
fn the_monitor_mix_law_matches_the_factory_formula_between_its_end_positions() {
    for m in 1..240 {
        assert_eq!(monitor_blend(at(m as u8)), factory_blend(m), "position {m}");
    }
}

#[test]
fn the_monitor_mix_ends_snap_to_exact_zero_and_exact_unity() {
    assert_eq!(
        monitor_blend(at(0)),
        MonitorBlend {
            inputs: Gain::from_q5_27(0x0800_0000),
            playback: Gain::ZERO,
        }
    );
    assert_eq!(
        monitor_blend(at(240)),
        MonitorBlend {
            inputs: Gain::ZERO,
            playback: Gain::from_q5_27(0x0800_0000),
        }
    );
}

#[test]
fn the_monitor_mix_crosses_over_at_minus_3_db_at_position_120() {
    let crossover = monitor_blend(at(120));
    assert_eq!(crossover.inputs, crossover.playback);
    assert_eq!(crossover.inputs, q5_27_of_decibels(-3.0));
}
