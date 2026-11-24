use dsp::{
    Band, BandCoefficients, BandFrequency, BandGain, BandQ, CoefficientSet, DspBlock, DspSample,
    FilterKind, FramesPerTick, Generation, MAX_FRAMES, MOST_FULL_BOOST_BANDS, PEQ_BANDS, Peq,
    PeqPreset, PresetBand, QuantisedBiquad, SampleRate,
};

const Q2_29_FULL_SCALE: f64 = (1u64 << 29) as f64;
const LARGEST: f64 = i32::MAX as f64 / Q2_29_FULL_SCALE;
const SMALLEST: f64 = i32::MIN as f64 / Q2_29_FULL_SCALE;
const BAND_HZ: f64 = 1_000.0;
const SIGN_THRESHOLD: f64 = 0.05;
const TRACKING_TOLERANCE: f64 = 0.05;
const KINDS: [FilterKind; 5] = [
    FilterKind::Peaking,
    FilterKind::LowShelf,
    FilterKind::HighShelf,
    FilterKind::HighPass,
    FilterKind::LowPass,
];

#[derive(Clone, Copy, Debug)]
enum Wave {
    Sine,
    Square,
}

fn preset(kind: FilterKind, q: BandQ, count: usize) -> PeqPreset {
    let band = Band {
        kind,
        frequency: BandFrequency::from_hz(BAND_HZ).unwrap(),
        gain: BandGain::LARGEST_BOOST,
        q,
    };
    let mut preset = PeqPreset::FLAT;
    for slot in preset.bands.iter_mut().take(count) {
        *slot = PresetBand {
            band,
            enabled: true,
        };
    }
    preset
}

fn signal(wave: Wave, rate: SampleRate, peak: f64, seconds: f64) -> Vec<f64> {
    let fs = f64::from(rate.hz());
    (0..(seconds * fs) as usize)
        .map(|n| {
            let phase = std::f64::consts::TAU * BAND_HZ * n as f64 / fs;
            match wave {
                Wave::Sine => peak * phase.sin(),
                Wave::Square if phase.sin() >= 0.0 => peak,
                Wave::Square => -peak,
            }
        })
        .map(|v| {
            (v * Q2_29_FULL_SCALE)
                .round()
                .clamp(i32::MIN as f64, i32::MAX as f64)
                / Q2_29_FULL_SCALE
        })
        .collect()
}

fn frames_for(rate: SampleRate) -> FramesPerTick {
    match rate {
        SampleRate::Hz44100 | SampleRate::Hz48000 => FramesPerTick::Six,
        SampleRate::Hz88200 | SampleRate::Hz96000 => FramesPerTick::Twelve,
        SampleRate::Hz176400 | SampleRate::Hz192000 => FramesPerTick::TwentyFour,
    }
}

fn run_kernel(set: &CoefficientSet, rate: SampleRate, input: &[f64]) -> Vec<f64> {
    let frames = frames_for(rate);
    let mut peq = Peq::new();
    let mut out = Vec::with_capacity(input.len());
    for chunk in input.chunks(frames.count()) {
        let mut block: DspBlock = [[DspSample::from_q2_29(0); 2]; MAX_FRAMES];
        for (slot, v) in block.iter_mut().zip(chunk) {
            let word = (v * Q2_29_FULL_SCALE) as i32;
            *slot = [word, word].map(DspSample::from_q2_29);
        }
        peq.filter(set, frames, &mut block);
        for frame in &block[..chunk.len()] {
            assert_eq!(frame[0], frame[1]);
            out.push(f64::from(frame[0].to_q2_29()) / Q2_29_FULL_SCALE);
        }
    }
    out
}

fn dequantised(biquad: &QuantisedBiquad) -> ([f64; 3], [f64; 2]) {
    let scale = (1u64 << biquad.format().fraction_bits()) as f64;
    (
        biquad.feedforward().map(|w| f64::from(w) / scale),
        biquad.feedback().map(|w| f64::from(w) / scale),
    )
}

fn saturating_reference(set: &CoefficientSet, input: &[f64]) -> Vec<f64> {
    let mut signal = input.to_vec();
    for coefficients in set.bands() {
        let BandCoefficients::Filter(biquad) = coefficients else {
            continue;
        };
        let ([b0, b1, b2], [f1, f2]) = dequantised(biquad);
        let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
        for v in signal.iter_mut() {
            let y = (b0 * *v + b1 * x1 + b2 * x2 + f1 * y1 + f2 * y2).clamp(SMALLEST, LARGEST);
            x2 = x1;
            x1 = *v;
            y2 = y1;
            y1 = y;
            *v = y;
        }
    }
    signal
}

fn assert_tracks_the_saturating_reference(
    set: &CoefficientSet,
    rate: SampleRate,
    input: &[f64],
    case: &str,
) {
    let produced = run_kernel(set, rate, input);
    let expected = saturating_reference(set, input);
    let mut sign_flips = 0usize;
    let mut worst = 0.0f64;
    for (got, want) in produced.iter().zip(&expected) {
        assert!(
            (SMALLEST..=LARGEST).contains(got),
            "{case}: {got} outside Q2.29"
        );
        if want.abs() > SIGN_THRESHOLD && got.signum() != want.signum() {
            sign_flips += 1;
        }
        worst = worst.max((got - want).abs());
    }
    assert_eq!(
        sign_flips, 0,
        "{case}: output changed sign against the saturating reference"
    );
    assert!(
        worst <= TRACKING_TOLERANCE,
        "{case}: strayed {worst} from the saturating reference"
    );
}

#[test]
fn two_twelve_decibel_peaks_on_a_minus_six_dbfs_sine_never_flip_sign() {
    let rate = SampleRate::Hz48000;
    let set = CoefficientSet::design(
        &preset(FilterKind::Peaking, BandQ::new(1.0).unwrap(), 2),
        rate,
        Generation::FIRST,
    );
    let input = signal(Wave::Sine, rate, 0.5, 0.25);
    assert_tracks_the_saturating_reference(&set, rate, &input, "two peaks, -6 dBFS sine");
}

#[test]
fn every_kind_at_maximum_gain_saturates_instead_of_wrapping() {
    for rate in [SampleRate::Hz48000, SampleRate::Hz96000] {
        for kind in KINDS {
            for q in [BandQ::new(1.0).unwrap(), BandQ::HIGHEST] {
                for count in 1..=PEQ_BANDS.min(MOST_FULL_BOOST_BANDS) {
                    let set =
                        CoefficientSet::design(&preset(kind, q, count), rate, Generation::FIRST);
                    for wave in [Wave::Sine, Wave::Square] {
                        let input = signal(wave, rate, 1.0, 0.05);
                        let case = format!("{rate:?} {kind:?} q {} x{count} {wave:?}", q.value());
                        assert_tracks_the_saturating_reference(&set, rate, &input, &case);
                    }
                }
            }
        }
    }
}
