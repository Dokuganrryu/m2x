#![cfg(all(feature = "intake-meter", feature = "bands14"))]

use dsp::{
    CaptureBlock, CoefficientSet, Gain, Generation, InputMute, MeterChannel, Meters, MonitorMix,
    PeqPreset, Pipeline, PlaybackBlock, PresetTrim, SampleRate, StreamSample, TickControls,
    TickMix,
};

const TICKS: usize = 64;

fn db_of(bits: u32) -> f64 {
    20.0 * (f64::from(bits) / f64::from(0x7FFF_FFFFu32)).log10()
}

fn run(mix: TickMix, trim_db: f64, level: i32) -> (f64, f64) {
    let rate = SampleRate::Hz96000;
    let coefficients = CoefficientSet::design_trimmed(
        &PeqPreset::FLAT,
        PresetTrim::from_db(trim_db).unwrap(),
        rate,
        Generation::FIRST,
    );
    let controls = TickControls {
        frames: rate.frames_per_tick(),
        mix,
        coefficients: &coefficients,
        monitor: MonitorMix::OFF,
        inputs: [InputMute::Live; 2],
        capture_gains: [Gain::UNITY; 2],
    };
    let mut pipeline = Pipeline::default();
    let mut playback = PlaybackBlock::new();
    let mut capture = CaptureBlock::new();
    let mut meters = Meters::new();
    let (mut tick, mut reader) = meters.split();
    let (mut intake, mut dac) = (0u32, 0u32);
    for _ in 0..TICKS {
        for (index, slot) in playback.input.iter_mut().enumerate() {
            let sign = if index % 2 == 0 { 1 } else { -1 };
            *slot = [StreamSample::from_bits(sign * level); 2];
        }
        pipeline.tick(&controls, &mut playback, &mut capture, &mut tick);
        let [left, _] = reader.take_intake();
        intake = intake.max(left.peak.to_bits());
        dac = dac.max(reader.take(MeterChannel::OutputLeft).peak.to_bits());
    }
    (db_of(intake), db_of(dac))
}

#[test]
fn the_intake_tap_reads_the_stream_before_the_trim_and_the_dac_tap_after_it() {
    let level = (f64::from(0x7FFF_FFFFu32) * 10f64.powf(-14.0 / 20.0)) as i32 & !0xFF;
    let (intake, dac) = run(TickMix::Processed, -6.6, level);
    assert!((intake - -14.0).abs() < 0.05, "intake {intake:.2} dB");
    assert!((dac - -20.6).abs() < 0.1, "dac {dac:.2} dB");
}

#[test]
fn in_bypass_both_taps_read_the_same_bit_perfect_stream() {
    let level = (f64::from(0x7FFF_FFFFu32) * 10f64.powf(-3.0 / 20.0)) as i32 & !0xFF;
    let (intake, dac) = run(TickMix::Bypass, -6.6, level);
    assert!(
        (intake - dac).abs() < 0.001,
        "intake {intake:.3} dac {dac:.3}"
    );
}
