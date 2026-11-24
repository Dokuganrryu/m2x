#![cfg(not(feature = "bands14"))]

use dsp::{
    Band, BandCoefficients, BandFrequency, BandGain, BandQ, CoefficientSet, FilterKind, Generation,
    PEQ_BANDS, PeqPreset, PresetBand, QuantisedBiquad, SampleRate,
};

struct Cookbook {
    feedforward: [f64; 3],
    feedback: [f64; 2],
}

fn cookbook(kind: FilterKind, f0: f64, fs: f64, gain_db: f64, q: f64) -> Cookbook {
    let a = 10f64.powf(gain_db / 40.0);
    let sqrt_a = a.sqrt();
    let w = 2.0 * std::f64::consts::PI * f0 / fs;
    let (s, c) = (w.sin(), w.cos());
    let alpha = s / (2.0 * q);
    let (b, den) = match kind {
        FilterKind::Peaking => (
            [1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a],
            [1.0 + alpha / a, -2.0 * c, 1.0 - alpha / a],
        ),
        FilterKind::LowShelf => (
            [
                a * ((a + 1.0) - (a - 1.0) * c + 2.0 * sqrt_a * alpha),
                2.0 * a * ((a - 1.0) - (a + 1.0) * c),
                a * ((a + 1.0) - (a - 1.0) * c - 2.0 * sqrt_a * alpha),
            ],
            [
                (a + 1.0) + (a - 1.0) * c + 2.0 * sqrt_a * alpha,
                -2.0 * ((a - 1.0) + (a + 1.0) * c),
                (a + 1.0) + (a - 1.0) * c - 2.0 * sqrt_a * alpha,
            ],
        ),
        FilterKind::HighShelf => (
            [
                a * ((a + 1.0) + (a - 1.0) * c + 2.0 * sqrt_a * alpha),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
                a * ((a + 1.0) + (a - 1.0) * c - 2.0 * sqrt_a * alpha),
            ],
            [
                (a + 1.0) - (a - 1.0) * c + 2.0 * sqrt_a * alpha,
                2.0 * ((a - 1.0) - (a + 1.0) * c),
                (a + 1.0) - (a - 1.0) * c - 2.0 * sqrt_a * alpha,
            ],
        ),
        FilterKind::HighPass => (
            [(1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0],
            [1.0 + alpha, -2.0 * c, 1.0 - alpha],
        ),
        FilterKind::LowPass => (
            [(1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0],
            [1.0 + alpha, -2.0 * c, 1.0 - alpha],
        ),
    };
    Cookbook {
        feedforward: b.map(|v| v / den[0]),
        feedback: [-den[1] / den[0], -den[2] / den[0]],
    }
}

fn magnitude(feedforward: [f64; 3], feedback: [f64; 2], f: f64, fs: f64) -> f64 {
    let w = -2.0 * std::f64::consts::PI * f / fs;
    let z = (w.cos(), w.sin());
    let z2 = (z.0 * z.0 - z.1 * z.1, 2.0 * z.0 * z.1);
    let num = (
        feedforward[0] + feedforward[1] * z.0 + feedforward[2] * z2.0,
        feedforward[1] * z.1 + feedforward[2] * z2.1,
    );
    let den = (
        1.0 - feedback[0] * z.0 - feedback[1] * z2.0,
        -feedback[0] * z.1 - feedback[1] * z2.1,
    );
    (num.0.hypot(num.1)) / (den.0.hypot(den.1))
}

fn dequantised(biquad: &QuantisedBiquad) -> ([f64; 3], [f64; 2]) {
    let scale = (1u64 << biquad.format().fraction_bits()) as f64;
    (
        biquad.feedforward().map(|w| w as f64 / scale),
        biquad.feedback().map(|w| w as f64 / scale),
    )
}

fn band(kind: FilterKind, hz: f64, db: f64, q: f64) -> Band {
    Band {
        kind,
        frequency: BandFrequency::from_hz(hz).unwrap(),
        gain: BandGain::from_db(db).unwrap(),
        q: BandQ::new(q).unwrap(),
    }
}

fn filtered(coefficients: BandCoefficients) -> QuantisedBiquad {
    match coefficients {
        BandCoefficients::Filter(biquad) => biquad,
        BandCoefficients::Skip => panic!("expected a filtered band"),
    }
}

fn gain_error_db(kind: FilterKind, hz: f64, db: f64, q: f64, rate: SampleRate, at: f64) -> f64 {
    let fs = rate.hz() as f64;
    let reference = cookbook(kind, hz, fs, db, q);
    let biquad = filtered(BandCoefficients::design(&band(kind, hz, db, q), rate));
    let (feedforward, feedback) = dequantised(&biquad);
    20.0 * (magnitude(feedforward, feedback, at, fs)
        / magnitude(reference.feedforward, reference.feedback, at, fs))
    .log10()
}

const COEFFICIENT_ACCURACY_GATE_DB: f64 = 0.01;

#[test]
fn a_20_hz_peak_at_192_khz_lands_within_a_hundredth_of_a_db_of_the_f64_design() {
    for at in [10.0, 20.0, 40.0] {
        let error = gain_error_db(
            FilterKind::Peaking,
            20.0,
            6.0,
            1.0,
            SampleRate::Hz192000,
            at,
        );
        assert!(
            error.abs() <= COEFFICIENT_ACCURACY_GATE_DB,
            "gain error {error:+.5} dB at {at} Hz"
        );
    }
}

#[test]
fn band_parameters_outside_the_console_ranges_are_refused() {
    use dsp::BandParameterError::*;
    for hz in [19.999, 20_000.001, f64::NAN, f64::INFINITY] {
        assert_eq!(
            BandFrequency::from_hz(hz),
            Err(FrequencyOutsideAudibleBand),
            "{hz} Hz"
        );
    }
    for db in [-12.001, 12.001, f64::NAN] {
        assert_eq!(
            BandGain::from_db(db),
            Err(GainBeyondTwelveDecibels),
            "{db} dB"
        );
    }
    for q in [0.299, 10.001, f64::NAN] {
        assert_eq!(BandQ::new(q), Err(QOutsideSupportedRange), "Q {q}");
    }
}

#[test]
fn band_parameters_at_the_console_range_edges_are_accepted() {
    assert_eq!(BandFrequency::from_hz(20.0), Ok(BandFrequency::LOWEST));
    assert_eq!(BandFrequency::from_hz(20_000.0), Ok(BandFrequency::HIGHEST));
    assert_eq!(BandGain::from_db(-12.0), Ok(BandGain::LARGEST_CUT));
    assert_eq!(BandGain::from_db(12.0), Ok(BandGain::LARGEST_BOOST));
    assert_eq!(BandQ::new(0.3), Ok(BandQ::LOWEST));
    assert_eq!(BandQ::new(10.0), Ok(BandQ::HIGHEST));
}

const GAIN_KINDS: [FilterKind; 3] = [
    FilterKind::Peaking,
    FilterKind::LowShelf,
    FilterKind::HighShelf,
];
const ALL_KINDS: [FilterKind; 5] = [
    FilterKind::Peaking,
    FilterKind::LowShelf,
    FilterKind::HighShelf,
    FilterKind::HighPass,
    FilterKind::LowPass,
];
const GRID_HZ: [f64; 9] = [
    20.0, 31.5, 63.0, 250.0, 1_000.0, 4_000.0, 10_000.0, 16_000.0, 20_000.0,
];
const GRID_DB: [f64; 6] = [-12.0, -6.0, -0.5, 0.5, 6.0, 12.0];
const GRID_Q: [f64; 5] = [0.3, 0.707, 1.0, 4.0, 10.0];

fn grid() -> impl Iterator<Item = (FilterKind, SampleRate, f64, f64, f64)> {
    ALL_KINDS.into_iter().flat_map(|kind| {
        SampleRate::ALL.into_iter().flat_map(move |rate| {
            GRID_HZ.into_iter().flat_map(move |hz| {
                GRID_DB
                    .into_iter()
                    .flat_map(move |db| GRID_Q.into_iter().map(move |q| (kind, rate, hz, db, q)))
            })
        })
    })
}

#[test]
fn a_band_at_zero_db_is_skipped_at_every_rate() {
    for kind in GAIN_KINDS {
        for rate in SampleRate::ALL {
            for hz in GRID_HZ {
                for q in GRID_Q {
                    assert_eq!(
                        BandCoefficients::design(&band(kind, hz, 0.0, q), rate),
                        BandCoefficients::Skip,
                        "{kind:?} {hz} Hz Q {q} at {} Hz",
                        rate.hz()
                    );
                }
            }
        }
    }
}

#[test]
fn high_and_low_pass_bands_are_filtered_whatever_their_gain() {
    for kind in [FilterKind::HighPass, FilterKind::LowPass] {
        for rate in SampleRate::ALL {
            let designed = BandCoefficients::design(&band(kind, 1_000.0, 0.0, 0.707), rate);
            assert!(
                matches!(designed, BandCoefficients::Filter(_)),
                "{kind:?} at {} Hz",
                rate.hz()
            );
        }
    }
}

fn narrowest_format_holding(reference: &Cookbook) -> dsp::CoefficientFormat {
    let largest = reference
        .feedforward
        .iter()
        .chain(&reference.feedback)
        .fold(0.0f64, |m, v| m.max(v.abs()));
    if largest < 2.0 - 1e-9 {
        dsp::CoefficientFormat::Q2_30
    } else if largest < 4.0 - 1e-9 {
        dsp::CoefficientFormat::Q3_29
    } else {
        dsp::CoefficientFormat::Q4_28
    }
}

#[test]
fn every_word_is_its_coefficient_rounded_to_the_narrowest_format_that_holds_the_band() {
    for (kind, rate, hz, db, q) in grid() {
        let reference = cookbook(kind, hz, rate.hz() as f64, db, q);
        let biquad = filtered(BandCoefficients::design(&band(kind, hz, db, q), rate));
        let label = format!("{kind:?} {hz} Hz {db} dB Q {q} at {} Hz", rate.hz());
        assert_eq!(
            biquad.format(),
            narrowest_format_holding(&reference),
            "{label}"
        );
        let step = 1.0 / (1u64 << biquad.format().fraction_bits()) as f64;
        let (feedforward, feedback) = dequantised(&biquad);
        for (word, exact) in feedforward
            .iter()
            .chain(&feedback)
            .zip(reference.feedforward.iter().chain(&reference.feedback))
        {
            assert!(
                (word - exact).abs() <= 0.5 * step * (1.0 + 1e-6),
                "{label}: {word} against {exact}"
            );
        }
    }
}

#[test]
fn twelve_db_shelves_at_twenty_hz_take_the_wider_formats() {
    let at_192k = |kind, db| {
        filtered(BandCoefficients::design(
            &band(kind, 20.0, db, 0.707),
            SampleRate::Hz192000,
        ))
        .format()
    };
    assert_eq!(
        at_192k(FilterKind::Peaking, 12.0),
        dsp::CoefficientFormat::Q2_30
    );
    assert_eq!(
        at_192k(FilterKind::HighShelf, 6.0),
        dsp::CoefficientFormat::Q3_29
    );
    assert_eq!(
        at_192k(FilterKind::HighShelf, 12.0),
        dsp::CoefficientFormat::Q4_28
    );
    assert_eq!(
        dsp::CoefficientFormat::Q4_28.post_shift(),
        2,
        "Q4.28 words shift left by two after accumulation"
    );
}

const WHOLE_RANGE_ACCURACY_DB: f64 = 0.1;

#[test]
fn every_band_at_every_rate_stays_within_a_tenth_of_a_db_of_its_f64_design() {
    let mut worst = (0.0f64, String::new());
    for (kind, rate, hz, db, q) in grid() {
        for at in [hz / 2.0, hz, (2.0 * hz).min(0.49 * rate.hz() as f64)] {
            let error = gain_error_db(kind, hz, db, q, rate, at).abs();
            if error > worst.0 {
                worst = (
                    error,
                    format!(
                        "{kind:?} {hz} Hz {db} dB Q {q} at {} Hz, measured at {at} Hz",
                        rate.hz()
                    ),
                );
            }
        }
    }
    println!("worst gain error {:.4} dB for {}", worst.0, worst.1);
    assert!(
        worst.0 <= WHOLE_RANGE_ACCURACY_DB,
        "{:.4} dB for {}",
        worst.0,
        worst.1
    );
}

fn host_word(value: f64, format: dsp::CoefficientFormat) -> i32 {
    (value * (1u64 << format.fraction_bits()) as f64).round() as i32
}

#[test]
fn design_without_the_host_math_library_gives_the_same_words_as_the_host_f64_design() {
    for (kind, rate, hz, db, q) in grid() {
        let reference = cookbook(kind, hz, rate.hz() as f64, db, q);
        let biquad = filtered(BandCoefficients::design(&band(kind, hz, db, q), rate));
        let format = biquad.format();
        assert_eq!(
            (biquad.feedforward(), biquad.feedback()),
            (
                reference.feedforward.map(|v| host_word(v, format)),
                reference.feedback.map(|v| host_word(v, format))
            ),
            "{kind:?} {hz} Hz {db} dB Q {q} at {} Hz",
            rate.hz()
        );
    }
}

fn preset_slot(kind: FilterKind, hz: f64, db: f64, enabled: bool) -> PresetBand {
    PresetBand {
        band: band(kind, hz, db, 1.0),
        enabled,
    }
}

#[test]
fn a_coefficient_set_filters_only_enabled_bands_that_change_the_signal() {
    let preset = PeqPreset {
        bands: [
            preset_slot(FilterKind::LowShelf, 80.0, 3.0, true),
            preset_slot(FilterKind::Peaking, 1_000.0, -4.0, false),
            preset_slot(FilterKind::Peaking, 2_000.0, 0.0, true),
            preset_slot(FilterKind::HighPass, 30.0, 0.0, true),
            preset_slot(FilterKind::HighShelf, 8_000.0, 2.0, false),
        ],
    };
    let set = CoefficientSet::design(&preset, SampleRate::Hz96000, Generation::FIRST.next());
    assert_eq!(set.enabled_mask(), 0b0_1001);
    assert_eq!(set.rate(), SampleRate::Hz96000);
    assert_eq!(set.generation(), Generation::FIRST.next());
    for (index, band) in set.bands().iter().enumerate() {
        let filtered = matches!(band, BandCoefficients::Filter(_));
        assert_eq!(
            filtered,
            set.enabled_mask() & (1 << index) != 0,
            "band {index}"
        );
    }
    assert_eq!(
        set.bands()[0],
        BandCoefficients::design(&preset.bands[0].band, SampleRate::Hz96000)
    );
}

#[test]
fn the_flat_preset_filters_nothing_at_any_rate() {
    for rate in SampleRate::ALL {
        let set = CoefficientSet::design(&PeqPreset::FLAT, rate, Generation::FIRST);
        assert_eq!(set.enabled_mask(), 0);
        assert_eq!(set.bands(), &[BandCoefficients::Skip; PEQ_BANDS]);
    }
}

#[test]
fn generations_advance_by_one_from_the_first() {
    assert_eq!(Generation::FIRST.value(), 0);
    assert_eq!(Generation::FIRST.next().value(), 1);
    let mut generation = Generation::FIRST;
    for _ in 0..3 {
        generation = generation.next();
    }
    assert_eq!(generation.value(), 3);
}

#[test]
fn coefficient_design_source_never_names_single_precision() {
    let design_sources = [
        ("mod.rs", include_str!("../src/peq/mod.rs")),
        ("band.rs", include_str!("../src/peq/band.rs")),
        ("cookbook.rs", include_str!("../src/peq/cookbook.rs")),
        (
            "coefficients.rs",
            include_str!("../src/peq/coefficients.rs"),
        ),
        ("series.rs", include_str!("../src/peq/series.rs")),
    ];
    for (name, source) in design_sources {
        assert!(!source.contains("f32"), "{name} names f32");
    }
}
