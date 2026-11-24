use dsp::{
    Band, BandFrequency, BandGain, BandQ, CROSSFADE_TICKS, CaptureBlock, CoefficientSet, Crossfade,
    DspMode, DspModeSwitch, FilterKind, Frame, FramesPerTick, Gain, Generation, I2sWord, InputMute,
    MeterTick, Meters, MonitorMix, PeqPreset, Pipeline, PlaybackBlock, PresetBand, PresetTrim,
    SampleRate, StreamSample, TickControls, TickMix,
};

const CODES: i32 = 1 << 24;
const STREAM_SHIFT: u32 = 8;

fn tick(
    pipeline: &mut Pipeline,
    frames: FramesPerTick,
    mix: TickMix,
    set: &CoefficientSet,
    block: &mut PlaybackBlock,
    meters: &mut MeterTick,
) {
    let mut capture = CaptureBlock::new();
    let controls = TickControls {
        frames,
        mix,
        coefficients: set,
        monitor: MonitorMix::OFF,
        inputs: [InputMute::Live; 2],
        capture_gains: [Gain::UNITY; 2],
    };
    pipeline.tick(&controls, block, &mut capture, meters);
}

fn band(kind: FilterKind, hz: f64, db: f64, q: f64) -> PresetBand {
    PresetBand {
        band: Band {
            kind,
            frequency: BandFrequency::from_hz(hz).unwrap(),
            gain: BandGain::from_db(db).unwrap(),
            q: BandQ::new(q).unwrap(),
        },
        enabled: true,
    }
}

fn loud_preset() -> PeqPreset {
    let mut preset = PeqPreset::FLAT;
    preset.bands[0] = band(FilterKind::LowShelf, 40.0, 6.0, 0.8);
    preset.bands[1] = band(FilterKind::Peaking, 7_500.0, 8.0, 1.6);
    preset
}

fn code(frame: i32, channel: i32) -> i32 {
    let mixed = (frame.wrapping_mul(2) + channel).wrapping_mul(0x9E37_79B1_u32 as i32);
    (mixed >> 8) << STREAM_SHIFT
}

#[test]
fn flat_after_a_trimmed_eq_is_a_bit_exact_copy_of_every_24_bit_code() {
    let mut pipeline = Pipeline::default();
    let mut meters = Meters::new();
    let (mut meter_tick, _) = meters.split();
    let mut block = PlaybackBlock::new();
    let switch = DspModeSwitch::new();
    let mut crossfade = Crossfade::new();
    let trim = PresetTrim::from_db(-6.6).unwrap();
    let eq = CoefficientSet::design_trimmed(
        &loud_preset(),
        trim,
        SampleRate::Hz48000,
        Generation::FIRST,
    );
    switch.request(DspMode::On);
    for frame in 0..(CROSSFADE_TICKS as i32 * 4) {
        block.input[..6].fill([StreamSample::from_bits(code(frame, 0)); 2]);
        let mix = crossfade.tick(&switch);
        tick(
            &mut pipeline,
            FramesPerTick::Six,
            mix,
            &eq,
            &mut block,
            &mut meter_tick,
        );
    }
    switch.request(DspMode::Bypass);
    for _ in 0..CROSSFADE_TICKS + 2 {
        let mix = crossfade.tick(&switch);
        tick(
            &mut pipeline,
            FramesPerTick::Six,
            mix,
            &eq,
            &mut block,
            &mut meter_tick,
        );
    }
    assert_eq!(crossfade.tick(&switch), TickMix::Bypass);

    let frames = FramesPerTick::TwentyFour.count() as i32;
    let mut previous: Option<[Frame<StreamSample>; 24]> = None;
    let mut checked = 0u64;
    let mut start = -CODES / 2;
    while start < CODES / 2 {
        let mut input = [[StreamSample::from_bits(0); 2]; 24];
        for (offset, frame) in input.iter_mut().enumerate() {
            let left = start + 2 * offset as i32;
            *frame = [
                StreamSample::from_bits(left << STREAM_SHIFT),
                StreamSample::from_bits((left + 1).min(CODES / 2 - 1) << STREAM_SHIFT),
            ];
        }
        block.input[..24].copy_from_slice(&input);
        let mix = crossfade.tick(&switch);
        tick(
            &mut pipeline,
            FramesPerTick::TwentyFour,
            mix,
            &eq,
            &mut block,
            &mut meter_tick,
        );
        if let Some(sent) = previous {
            for (out, sent) in block.output[..24].iter().zip(sent) {
                assert_eq!(
                    out.map(I2sWord::to_bits),
                    sent.map(|s| s.to_bits()),
                    "flat must copy the 24-bit code unchanged"
                );
                checked += 2;
            }
        }
        previous = Some(input);
        start += 2 * frames;
    }
    assert!(checked >= CODES as u64 - 2 * 24, "{checked} codes checked");
}

#[cfg(feature = "bands14")]
use dsp::BandCoefficients;

#[cfg(feature = "bands14")]
#[test]
fn a_trim_scales_only_the_first_running_band_and_no_trim_designs_the_same_set() {
    let preset = loud_preset();
    let plain = CoefficientSet::design(&preset, SampleRate::Hz48000, Generation::FIRST);
    let untrimmed = CoefficientSet::design_trimmed(
        &preset,
        PresetTrim::NONE,
        SampleRate::Hz48000,
        Generation::FIRST,
    );
    assert_eq!(plain, untrimmed);
    let trimmed = CoefficientSet::design_trimmed(
        &preset,
        PresetTrim::from_db(-6.0).unwrap(),
        SampleRate::Hz48000,
        Generation::FIRST,
    );
    let (BandCoefficients::Filter(a), BandCoefficients::Filter(b)) =
        (plain.bands()[0], trimmed.bands()[0])
    else {
        panic!("band 0 runs");
    };
    assert_eq!(a.feedback(), b.feedback());
    let ratio = f64::from(b.feedforward()[0]) / f64::from(a.feedforward()[0])
        * f64::from(1i32 << a.format().fraction_bits())
        / f64::from(1i32 << b.format().fraction_bits());
    assert!((20.0 * ratio.log10() + 6.0).abs() < 0.001, "ratio {ratio}");
    assert_eq!(plain.bands()[1..], trimmed.bands()[1..]);
}

#[cfg(feature = "bands14")]
#[test]
fn a_trim_with_every_band_off_runs_as_one_pure_gain_band() {
    let set = CoefficientSet::design_trimmed(
        &PeqPreset::FLAT,
        PresetTrim::from_db(-6.0).unwrap(),
        SampleRate::Hz48000,
        Generation::FIRST,
    );
    assert_eq!(set.enabled_mask(), 1);
    let BandCoefficients::Filter(gain) = set.bands()[0] else {
        panic!("band 0 carries the trim");
    };
    assert_eq!(gain.feedforward()[1..], [0, 0]);
    assert_eq!(gain.feedback(), [0, 0]);
}

#[test]
fn the_trim_accepts_minus_18_to_0_db_only() {
    assert!(PresetTrim::from_db(0.0).is_ok());
    assert!(PresetTrim::from_db(-18.0).is_ok());
    assert!(PresetTrim::from_db(0.1).is_err());
    assert!(PresetTrim::from_db(-18.1).is_err());
}
