#![cfg(not(feature = "bands14"))]

use dsp::{
    Band, BandCoefficients, BandFrequency, BandGain, BandQ, CoefficientFormat, CoefficientSet,
    DspBlock, DspSample, FilterKind, FramesPerTick, Generation, MAX_FRAMES, PEQ_BANDS, Peq,
    PeqPreset, PresetBand, QuantisedBiquad, SampleRate,
};

const Q2_29_FULL_SCALE: f64 = (1u64 << 29) as f64;
const LSB_24_BIT: f64 = 1.0 / (1u64 << 23) as f64;
const NOISE_FLOOR_24_BIT_DB: f64 = -149.3;
const CASCADE_FLOOR_DB: f64 = -143.0;
const MULTITONE_PEAK: f64 = 0.5;
const SECONDS: f64 = 0.5;
const RAIL_NEIGHBOURHOOD: u32 = i32::MAX as u32 - 16;

fn band(kind: FilterKind, hz: f64, db: f64, q: f64) -> Band {
    Band {
        kind,
        frequency: BandFrequency::from_hz(hz).unwrap(),
        gain: BandGain::from_db(db).unwrap(),
        q: BandQ::new(q).unwrap(),
    }
}

fn preset(bands: &[Band]) -> PeqPreset {
    let mut preset = PeqPreset::FLAT;
    for (slot, band) in preset.bands.iter_mut().zip(bands) {
        *slot = PresetBand {
            band: *band,
            enabled: true,
        };
    }
    preset
}

fn frames_for(rate: SampleRate) -> FramesPerTick {
    match rate {
        SampleRate::Hz44100 | SampleRate::Hz48000 => FramesPerTick::Six,
        SampleRate::Hz88200 | SampleRate::Hz96000 => FramesPerTick::Twelve,
        SampleRate::Hz176400 | SampleRate::Hz192000 => FramesPerTick::TwentyFour,
    }
}

fn multitone_24_bit(rate: SampleRate, band_hz: f64) -> Vec<f64> {
    let fs = f64::from(rate.hz());
    let count = (SECONDS * fs) as usize;
    let tones = [20.0, 200.0, 2_000.0, 15_000.0, band_hz * 1.01];
    let raw: Vec<f64> = (0..count)
        .map(|n| {
            tones
                .iter()
                .enumerate()
                .map(|(phase, hz)| {
                    (std::f64::consts::TAU * hz * n as f64 / fs + phase as f64).sin()
                })
                .sum()
        })
        .collect();
    let peak = raw.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    raw.iter()
        .map(|v| (MULTITONE_PEAK * v / peak / LSB_24_BIT).round() * LSB_24_BIT)
        .collect()
}

fn run_kernel(set: &CoefficientSet, frames: FramesPerTick, stereo: &[[i32; 2]]) -> Vec<[i32; 2]> {
    let mut peq = Peq::new();
    let mut out = Vec::with_capacity(stereo.len());
    for chunk in stereo.chunks(frames.count()) {
        let mut block: DspBlock = [[DspSample::from_q2_29(0); 2]; MAX_FRAMES];
        for (slot, frame) in block.iter_mut().zip(chunk) {
            *slot = frame.map(DspSample::from_q2_29);
        }
        peq.filter(set, frames, &mut block);
        out.extend(
            block[..chunk.len()]
                .iter()
                .map(|f| f.map(DspSample::to_q2_29)),
        );
    }
    out
}

fn dequantised(biquad: &QuantisedBiquad) -> ([f64; 3], [f64; 2]) {
    let scale = (1u64 << biquad.format().fraction_bits()) as f64;
    (
        biquad.feedforward().map(|w| w as f64 / scale),
        biquad.feedback().map(|w| w as f64 / scale),
    )
}

fn f64_reference(set: &CoefficientSet, input: &[f64]) -> Vec<f64> {
    let mut signal = input.to_vec();
    for coefficients in set.bands() {
        let BandCoefficients::Filter(biquad) = coefficients else {
            continue;
        };
        let ([b0, b1, b2], [f1, f2]) = dequantised(biquad);
        let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
        for v in signal.iter_mut() {
            let y = b0 * *v + b1 * x1 + b2 * x2 + f1 * y1 + f2 * y2;
            x2 = x1;
            x1 = *v;
            y2 = y1;
            y1 = y;
            *v = y;
        }
    }
    signal
}

fn error_db_re_peak(set: &CoefficientSet, rate: SampleRate, input: &[f64]) -> f64 {
    let stereo: Vec<[i32; 2]> = input
        .iter()
        .map(|v| {
            let word = (v * Q2_29_FULL_SCALE) as i32;
            [word, -word]
        })
        .collect();
    let produced = run_kernel(set, frames_for(rate), &stereo);
    let expected = f64_reference(set, input);
    let settled = input.len() / 10;
    let mut worst = f64::NEG_INFINITY;
    for channel in 0..2 {
        let sign = if channel == 0 { 1.0 } else { -1.0 };
        let squares: f64 = produced[settled..]
            .iter()
            .zip(&expected[settled..])
            .map(|(got, want)| (got[channel] as f64 / Q2_29_FULL_SCALE - sign * want).powi(2))
            .sum();
        let rms = (squares / (input.len() - settled) as f64).sqrt();
        worst = worst.max(20.0 * rms.log10());
    }
    worst
}

#[test]
fn one_20_hz_band_stays_under_the_24_bit_noise_floor_at_48_and_192_khz() {
    let gate_band = band(FilterKind::Peaking, 20.0, 6.0, 1.0);
    for rate in [SampleRate::Hz48000, SampleRate::Hz192000] {
        let set = CoefficientSet::design(&preset(&[gate_band]), rate, Generation::FIRST);
        let error = error_db_re_peak(&set, rate, &multitone_24_bit(rate, 20.0));
        assert!(
            error <= NOISE_FLOOR_24_BIT_DB,
            "{rate:?} error {error:.1} dB re peak"
        );
    }
}

fn worst_case_low_cascade() -> [Band; PEQ_BANDS] {
    [
        band(FilterKind::HighShelf, 20.0, 12.0, 0.707),
        band(FilterKind::Peaking, 20.0, 6.0, 1.0),
        band(FilterKind::HighShelf, 25.0, -6.0, 0.707),
        band(FilterKind::HighShelf, 31.5, 6.0, 0.707),
        band(FilterKind::HighPass, 20.0, 0.0, 0.707),
    ]
}

#[test]
fn the_worst_case_low_cascade_uses_every_coefficient_format() {
    let set = CoefficientSet::design(
        &preset(&worst_case_low_cascade()),
        SampleRate::Hz192000,
        Generation::FIRST,
    );
    let formats: Vec<CoefficientFormat> = set
        .bands()
        .iter()
        .filter_map(|b| match b {
            BandCoefficients::Filter(q) => Some(q.format()),
            BandCoefficients::Skip => None,
        })
        .collect();
    assert_eq!(formats.len(), PEQ_BANDS);
    for format in [
        CoefficientFormat::Q2_30,
        CoefficientFormat::Q3_29,
        CoefficientFormat::Q4_28,
    ] {
        assert!(formats.contains(&format), "{format:?} missing");
    }
}

#[test]
fn five_worst_case_low_bands_at_192_khz_stay_under_the_cascade_floor() {
    let rate = SampleRate::Hz192000;
    let set = CoefficientSet::design(&preset(&worst_case_low_cascade()), rate, Generation::FIRST);
    let input = multitone_24_bit(rate, 20.0);
    let peak = f64_reference(&set, &input)
        .iter()
        .fold(0.0f64, |m, v| m.max(v.abs()));
    assert!(peak < 4.0, "cascade leaves Q2.29 headroom, peak {peak}");
    let error = error_db_re_peak(&set, rate, &input);
    assert!(error <= CASCADE_FLOOR_DB, "error {error:.1} dB re peak");
}

#[test]
fn a_band_in_each_coefficient_format_alone_stays_under_the_24_bit_noise_floor() {
    let rate = SampleRate::Hz192000;
    for single in [
        band(FilterKind::Peaking, 20.0, 6.0, 1.0),
        band(FilterKind::HighShelf, 20.0, 6.0, 0.707),
        band(FilterKind::HighShelf, 20.0, 12.0, 0.707),
    ] {
        let set = CoefficientSet::design(&preset(&[single]), rate, Generation::FIRST);
        let error = error_db_re_peak(&set, rate, &multitone_24_bit(rate, 20.0));
        assert!(
            error <= NOISE_FLOOR_24_BIT_DB,
            "{single:?} error {error:.1} dB re peak"
        );
    }
}

struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn sample_24_bit_in_q2_29(&mut self) -> i32 {
        ((self.next() as i32) >> 8) << 6
    }

    fn preset(&mut self) -> PeqPreset {
        let kinds = [
            FilterKind::Peaking,
            FilterKind::LowShelf,
            FilterKind::HighShelf,
            FilterKind::HighPass,
            FilterKind::LowPass,
        ];
        let mut preset = PeqPreset::FLAT;
        for slot in preset.bands.iter_mut() {
            *slot = PresetBand {
                band: band(
                    kinds[(self.next() % 5) as usize],
                    20.0 * 1000f64.powf(self.unit()),
                    -12.0 + 24.0 * self.unit(),
                    0.3 + 9.7 * self.unit(),
                ),
                enabled: self.next() % 4 != 0,
            };
        }
        preset
    }
}

#[derive(Clone, Copy, Default)]
struct ModelState {
    x1: i64,
    x2: i64,
    y1: i64,
    y2: i64,
    error: i64,
}

fn per_sample_definition(
    set: &CoefficientSet,
    states: &mut [[ModelState; 2]],
    frame: [i32; 2],
) -> [i32; 2] {
    let mut signal = frame;
    for (coefficients, channel_states) in set.bands().iter().zip(states.iter_mut()) {
        let BandCoefficients::Filter(biquad) = coefficients else {
            continue;
        };
        let [b0, b1, b2] = biquad.feedforward().map(i64::from);
        let [f1, f2] = biquad.feedback().map(i64::from);
        let shift = biquad.format().fraction_bits();
        for (value, s) in signal.iter_mut().zip(channel_states.iter_mut()) {
            let x = i64::from(*value);
            let exact = [b0 * x, b1 * s.x1, b2 * s.x2, f1 * s.y1, f2 * s.y2, s.error]
                .into_iter()
                .fold(0i64, i64::wrapping_add);
            let rounded = exact.wrapping_add(1 << (shift - 1)) >> shift;
            let kept = 32 - shift;
            let high = (rounded >> kept).clamp(-(1 << (shift - 1)), (1 << (shift - 1)) - 1);
            let y = ((high << kept) | (rounded & ((1 << kept) - 1))) as i32;
            s.error = exact.wrapping_sub(rounded << shift);
            s.x2 = s.x1;
            s.x1 = x;
            s.y2 = s.y1;
            s.y1 = i64::from(y);
            *value = y;
        }
    }
    signal
}

#[test]
fn the_kernel_matches_the_per_sample_definition_bit_for_bit() {
    let mut rng = Xorshift(0x9E37_79B9_7F4A_7C15);
    for trial in 0..60 {
        let rate = SampleRate::ALL[trial % 6];
        let frames = [
            FramesPerTick::Six,
            FramesPerTick::Twelve,
            FramesPerTick::TwentyFour,
        ][trial % 3];
        let set = CoefficientSet::design(&rng.preset(), rate, Generation::FIRST);
        let mut peq = Peq::new();
        let mut model = [[ModelState::default(); 2]; PEQ_BANDS];
        for _ in 0..40 {
            let mut block: DspBlock = [[DspSample::from_q2_29(0); 2]; MAX_FRAMES];
            for frame in block.iter_mut() {
                *frame = [rng.sample_24_bit_in_q2_29(), rng.sample_24_bit_in_q2_29()]
                    .map(DspSample::from_q2_29);
            }
            let before = block;
            peq.filter(&set, frames, &mut block);
            for (index, (got, given)) in block.iter().zip(&before).enumerate() {
                if index < frames.count() {
                    let want =
                        per_sample_definition(&set, &mut model, given.map(DspSample::to_q2_29));
                    assert_eq!(
                        got.map(DspSample::to_q2_29),
                        want,
                        "trial {trial} frame {index}"
                    );
                } else {
                    assert_eq!(got, given, "frame {index} past the tick is untouched");
                }
            }
        }
    }
}

#[test]
fn output_does_not_depend_on_how_the_stream_is_cut_into_ticks() {
    let mut rng = Xorshift(0x2545_F491_4F6C_DD1D);
    let set = CoefficientSet::design(&rng.preset(), SampleRate::Hz96000, Generation::FIRST);
    let stream: Vec<[i32; 2]> = (0..24 * 50)
        .map(|_| [rng.sample_24_bit_in_q2_29(), rng.sample_24_bit_in_q2_29()])
        .collect();
    let six = run_kernel(&set, FramesPerTick::Six, &stream);
    assert_eq!(six, run_kernel(&set, FramesPerTick::Twelve, &stream));
    assert_eq!(six, run_kernel(&set, FramesPerTick::TwentyFour, &stream));
}

#[test]
fn a_set_with_every_band_skipped_leaves_the_block_unchanged() {
    let mut rng = Xorshift(7);
    let mut preset = preset(&[band(FilterKind::Peaking, 100.0, 0.0, 1.0); PEQ_BANDS]);
    preset.bands[1].enabled = false;
    let set = CoefficientSet::design(&preset, SampleRate::Hz48000, Generation::FIRST);
    assert_eq!(set.enabled_mask(), 0);
    let mut peq = Peq::new();
    for frames in [
        FramesPerTick::Six,
        FramesPerTick::Twelve,
        FramesPerTick::TwentyFour,
    ] {
        let mut block: DspBlock = [[DspSample::from_q2_29(0); 2]; MAX_FRAMES];
        for frame in block.iter_mut() {
            *frame = [rng.sample_24_bit_in_q2_29(), rng.sample_24_bit_in_q2_29()]
                .map(DspSample::from_q2_29);
        }
        let before = block;
        peq.filter(&set, frames, &mut block);
        assert_eq!(block, before);
    }
}

#[test]
fn a_skipped_tick_leaves_the_band_history_as_if_that_tick_never_came() {
    let mut rng = Xorshift(11);
    let active = CoefficientSet::design(
        &preset(&[band(FilterKind::Peaking, 300.0, 6.0, 2.0)]),
        SampleRate::Hz48000,
        Generation::FIRST,
    );
    let skipped = CoefficientSet::design(&PeqPreset::FLAT, SampleRate::Hz48000, Generation::FIRST);
    let mut random_block = || {
        let mut block: DspBlock = [[DspSample::from_q2_29(0); 2]; MAX_FRAMES];
        for frame in block.iter_mut() {
            *frame = [rng.sample_24_bit_in_q2_29(), rng.sample_24_bit_in_q2_29()]
                .map(DspSample::from_q2_29);
        }
        block
    };
    let blocks: Vec<DspBlock> = (0..5).map(|_| random_block()).collect();
    let mut interrupted = Peq::new();
    let mut continuous = Peq::new();
    for (index, block) in blocks.iter().enumerate() {
        let mut a = *block;
        if index == 2 {
            interrupted.filter(&skipped, FramesPerTick::Six, &mut a);
            assert_eq!(a, *block);
            continue;
        }
        let mut b = *block;
        interrupted.filter(&active, FramesPerTick::Six, &mut a);
        continuous.filter(&active, FramesPerTick::Six, &mut b);
        assert_eq!(a, b, "tick {index}");
    }
}

#[test]
fn the_kernel_matches_the_per_sample_definition_when_the_cascade_saturates() {
    let peak = band(FilterKind::Peaking, 1_000.0, 12.0, 1.0);
    let set = CoefficientSet::design(
        &preset(&[peak; PEQ_BANDS]),
        SampleRate::Hz96000,
        Generation::FIRST,
    );
    let mut peq = Peq::new();
    let mut model = [[ModelState::default(); 2]; PEQ_BANDS];
    let mut saturated = 0usize;
    for tick in 0..400 {
        let mut block: DspBlock = [[DspSample::from_q2_29(0); 2]; MAX_FRAMES];
        for (index, frame) in block.iter_mut().take(12).enumerate() {
            let n = (tick * 12 + index) as f64;
            let x = (std::f64::consts::TAU * 1_000.0 * n / 96_000.0).sin() * Q2_29_FULL_SCALE;
            *frame = [x as i32, -(x as i32)].map(DspSample::from_q2_29);
        }
        let before = block;
        peq.filter(&set, FramesPerTick::Twelve, &mut block);
        for (got, given) in block.iter().zip(&before).take(12) {
            let want = per_sample_definition(&set, &mut model, given.map(DspSample::to_q2_29));
            assert_eq!(got.map(DspSample::to_q2_29), want, "tick {tick}");
            saturated += want
                .iter()
                .filter(|y| y.unsigned_abs() > RAIL_NEIGHBOURHOOD)
                .count();
        }
    }
    assert!(saturated > 0, "the case must reach the rails");
}
