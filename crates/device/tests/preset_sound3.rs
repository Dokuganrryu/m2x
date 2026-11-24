#![cfg(feature = "sound3")]

use device::preset::Preset;
use dsp::{
    BandCoefficients, CoefficientSet, FramesPerTick, Generation, PEQ_BANDS, PresetTrim, SampleRate,
    bands_running_at, lift_bands_at_twelve_frames,
};

const RAIL: f64 = 4.0;
const RESPONSE: usize = 1 << 15;
const RATES: [SampleRate; 4] = [
    SampleRate::Hz44100,
    SampleRate::Hz48000,
    SampleRate::Hz88200,
    SampleRate::Hz96000,
];

fn biquad_of(band: &BandCoefficients) -> Option<([f64; 3], [f64; 2])> {
    let BandCoefficients::Filter(biquad) = band else {
        return None;
    };
    let unity = f64::from(1u32 << biquad.format().fraction_bits());
    Some((
        biquad.feedforward().map(|v| f64::from(v) / unity),
        biquad.feedback().map(|v| f64::from(v) / unity),
    ))
}

fn largest_prefix_l1(set: &CoefficientSet) -> f64 {
    let mut signal = vec![0.0; RESPONSE];
    signal[0] = 1.0;
    let mut largest = 1.0f64;
    for band in set.bands() {
        let Some(([b0, b1, b2], [f1, f2])) = biquad_of(band) else {
            continue;
        };
        let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
        for sample in signal.iter_mut() {
            let x = *sample;
            let y = b0 * x + b1 * x1 + b2 * x2 + f1 * y1 + f2 * y2;
            (x2, x1, y2, y1) = (x1, x, y1, y);
            *sample = y;
        }
        largest = largest.max(signal.iter().map(|v| v.abs()).sum());
    }
    largest
}

fn peak_gain_db(set: &CoefficientSet, rate: SampleRate) -> f64 {
    let fs = f64::from(rate.hz());
    (0..2_000)
        .map(|step| 20.0 * (1000.0f64).powf(f64::from(step) / 1999.0))
        .map(|hz| {
            let w = 2.0 * std::f64::consts::PI * hz / fs;
            let gain: f64 = set
                .bands()
                .iter()
                .filter_map(biquad_of)
                .map(|([b0, b1, b2], [f1, f2])| {
                    let z1 = (w.cos(), -w.sin());
                    let z2 = ((2.0 * w).cos(), -(2.0 * w).sin());
                    let num = (b0 + b1 * z1.0 + b2 * z2.0, b1 * z1.1 + b2 * z2.1);
                    let den = (1.0 - f1 * z1.0 - f2 * z2.0, -f1 * z1.1 - f2 * z2.1);
                    (num.0.hypot(num.1)) / (den.0.hypot(den.1))
                })
                .product();
            20.0 * gain.log10()
        })
        .fold(f64::NEG_INFINITY, f64::max)
}

fn designed(preset: Preset, rate: SampleRate) -> CoefficientSet {
    let bands = preset.bands().unwrap();
    let (high, trim) = bands.for_twelve_frame_bands(bands_running_at(FramesPerTick::Twelve));
    let chosen = match rate.frames_per_tick() {
        FramesPerTick::Twelve => high,
        FramesPerTick::Six | FramesPerTick::TwentyFour => &bands.preset,
    };
    CoefficientSet::design_trimmed(chosen, trim, rate, Generation::FIRST)
}

#[test]
fn at_100_mhz_96k_keeps_sound2s_seven_bands_and_at_150_mhz_runs_all_fourteen() {
    for preset in [Preset::Clarity, Preset::Warm] {
        let bands = preset.bands().unwrap();
        let (seven, trim) = bands.for_twelve_frame_bands(7);
        let set =
            CoefficientSet::design_trimmed(seven, trim, SampleRate::Hz96000, Generation::FIRST);
        assert!(set.enabled_mask().count_ones() <= 7);
    }
    lift_bands_at_twelve_frames();
    assert_eq!(bands_running_at(FramesPerTick::Twelve), PEQ_BANDS);
    for preset in [Preset::Clarity, Preset::Warm] {
        for rate in RATES {
            assert_eq!(
                designed(preset, rate).enabled_mask().count_ones(),
                14,
                "{preset:?} {rate:?}"
            );
        }
        let bands = preset.bands().unwrap();
        assert_eq!(bands.for_twelve_frame_bands(14).1, bands.full_trim);
        assert_eq!(bands.for_twelve_frame_bands(7).1, bands.trim);
    }
    assert_eq!(
        Preset::Clarity.bands().unwrap().full_trim,
        PresetTrim::from_db(-2.6).unwrap()
    );
    assert_eq!(
        Preset::Warm.bands().unwrap().full_trim,
        PresetTrim::from_db(-4.2).unwrap()
    );
}

#[test]
fn with_the_150_mhz_trims_no_steady_tone_gains_above_full_scale_and_no_stage_can_clamp() {
    lift_bands_at_twelve_frames();
    for preset in [Preset::Clarity, Preset::Warm] {
        for rate in RATES {
            let set = designed(preset, rate);
            let peak = peak_gain_db(&set, rate);
            assert!(peak <= 0.0, "{preset:?} {rate:?} peaks at {peak:+.3} dB");
            assert!(
                peak > -0.5,
                "{preset:?} {rate:?} trimmed too far: {peak:+.3} dB"
            );
            let largest = largest_prefix_l1(&set);
            assert!(
                largest < RAIL,
                "{preset:?} {rate:?}: prefix L1 {largest:.3}"
            );
        }
    }
}
