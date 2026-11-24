#![cfg(feature = "sound2")]

mod console_support;

use console_support::typed;
use device::console::{ConsoleCommand, Output, Refusal};
use device::lcd::screen::DspBadge;
use device::preset::Preset;
use dsp::{
    BandCoefficients, CoefficientSet, DspMode, Generation, PEQ_BANDS, PresetTrim, SampleRate,
};

fn executes(line: &str) -> ConsoleCommand {
    match typed(format!("{line}\n").as_bytes()).as_slice() {
        [Output::Execute(command)] => *command,
        other => panic!("{line:?} gave {other:?}"),
    }
}

fn refusal_of(line: &str) -> Refusal {
    match typed(format!("{line}\n").as_bytes()).as_slice() {
        [Output::Reject(refusal)] => *refusal,
        other => panic!("{line:?} gave {other:?}"),
    }
}

#[test]
fn sound2_runs_fourteen_bands() {
    assert_eq!(PEQ_BANDS, 14);
}

#[test]
fn the_boot_preset_is_clarity_with_the_eq_on() {
    assert_eq!(Preset::AT_BOOT, Preset::Flat);
    assert_eq!(Preset::AT_BOOT.mode(), DspMode::Bypass);
    assert_eq!(Preset::AT_BOOT.badge(), DspBadge::Raw);
}

#[test]
fn the_presets_take_every_documented_spelling() {
    for (word, preset) in [
        ("clarity", Preset::Clarity),
        ("eq1", Preset::Clarity),
        ("eq2", Preset::Warm),
        ("warm", Preset::Warm),
        ("flat", Preset::Flat),
        ("raw", Preset::Flat),
        ("bypass", Preset::Flat),
    ] {
        assert_eq!(Preset::named(word.as_bytes()), Some(preset), "{word}");
        assert_eq!(
            executes(&format!("preset {word}")),
            ConsoleCommand::Preset(Some(preset))
        );
    }
    assert_eq!(executes("preset"), ConsoleCommand::Preset(None));
    assert!(executes("preset").only_reads());
    assert!(!executes("preset warm").only_reads());
    assert_eq!(refusal_of("preset loud"), Refusal::MalformedArguments);
}

#[test]
fn the_long_press_toggles_between_clarity_and_the_warm_and_leaves_flat_for_clarity() {
    assert_eq!(Preset::Clarity.toggled(), Preset::Warm);
    assert_eq!(Preset::Warm.toggled(), Preset::Clarity);
    assert_eq!(Preset::Flat.toggled(), Preset::Clarity);
}

#[test]
fn flat_is_the_bypass_path_with_no_bands_and_a_raw_badge() {
    assert_eq!(Preset::Flat.mode(), DspMode::Bypass);
    assert!(Preset::Flat.bands().is_none());
    assert_eq!(Preset::Flat.badge(), DspBadge::Raw);
}

#[test]
fn each_eq_preset_runs_fourteen_bands_at_48k_and_its_seven_band_fit_at_96k() {
    for preset in [Preset::Clarity, Preset::Warm] {
        let bands = preset.bands().unwrap();
        let low = CoefficientSet::design_trimmed(
            &bands.preset,
            bands.trim,
            SampleRate::Hz48000,
            Generation::FIRST,
        );
        assert_eq!(low.enabled_mask().count_ones(), 14, "{preset:?}");
        let high = CoefficientSet::design_trimmed(
            &bands.high_rate,
            bands.trim,
            SampleRate::Hz96000,
            Generation::FIRST,
        );
        assert_eq!(high.enabled_mask().count_ones(), 7, "{preset:?}");
    }
}

#[test]
fn the_trims_are_the_fitted_peaks_rounded_up_so_no_preset_gains_above_0_db() {
    assert_eq!(
        Preset::Clarity.bands().unwrap().trim,
        PresetTrim::from_db(-2.6).unwrap()
    );
    assert_eq!(
        Preset::Warm.bands().unwrap().trim,
        PresetTrim::from_db(-4.2).unwrap()
    );
}

#[test]
fn trim_peq_begin_and_commit_parse() {
    assert_eq!(
        executes("trim -5.5"),
        ConsoleCommand::SetTrim(PresetTrim::from_db(-5.5).unwrap())
    );
    assert_eq!(refusal_of("trim 1"), Refusal::SettingOutOfRange);
    assert_eq!(refusal_of("trim -18.5"), Refusal::SettingOutOfRange);
    assert_eq!(executes("peq begin"), ConsoleCommand::BeginBands);
    assert_eq!(executes("peq commit"), ConsoleCommand::CommitBands);
    assert!(!executes("trim -6").only_reads());
}

const RAIL: f64 = 4.0;
const RESPONSE: usize = 1 << 15;

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

// WHY: A band's output only clamps if its input times its prefix impulse response can pass the Q2.29 rail of 4 x full scale; with every prefix's L1 norm under 4 no full-scale signal ever reaches a clamp, so the cascade stays exactly linear and its accumulator far from the i64 range
#[test]
fn no_stage_of_either_built_in_preset_can_clamp_for_any_full_scale_signal() {
    for preset in [Preset::Clarity, Preset::Warm] {
        let bands = preset.bands().unwrap();
        for (chosen, rate) in [
            (&bands.preset, SampleRate::Hz44100),
            (&bands.preset, SampleRate::Hz48000),
            (&bands.high_rate, SampleRate::Hz88200),
            (&bands.high_rate, SampleRate::Hz96000),
        ] {
            let set = CoefficientSet::design_trimmed(chosen, bands.trim, rate, Generation::FIRST);
            let largest = largest_prefix_l1(&set);
            assert!(
                largest < RAIL,
                "{preset:?} {rate:?}: a prefix L1 norm of {largest:.3}"
            );
        }
    }
}
