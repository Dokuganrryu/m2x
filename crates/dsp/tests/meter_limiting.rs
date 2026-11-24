#![cfg(feature = "meter-limiting")]

use dsp::{
    CaptureBlock, CoefficientSet, Generation, InputMute, Meters, MonitorMix, PeqPreset, Pipeline,
    PlaybackBlock, SampleRate, StreamSample, TickControls, TickMix,
};

const FULL_SCALE: i32 = 0x7FFF_FF00;
const QUIET: i32 = 0x0100_0000;
const TICKS: usize = 40;

fn limited_ticks(mix: TickMix, level: i32) -> u32 {
    let rate = SampleRate::Hz48000;
    let coefficients = CoefficientSet::design(&PeqPreset::FLAT, rate, Generation::FIRST);
    let controls = TickControls {
        frames: rate.frames_per_tick(),
        mix,
        coefficients: &coefficients,
        monitor: MonitorMix::OFF,
        inputs: [InputMute::Live; 2],
        capture_gains: [dsp::Gain::UNITY; 2],
    };
    let mut pipeline = Pipeline::default();
    let mut playback = PlaybackBlock::new();
    let mut capture = CaptureBlock::new();
    let mut meters = Meters::new();
    let (mut tick, mut reader) = meters.split();
    for _ in 0..TICKS {
        for (index, slot) in playback.input.iter_mut().enumerate() {
            let sign = if index % 2 == 0 { 1 } else { -1 };
            *slot = [StreamSample::from_bits(sign * level); 2];
        }
        pipeline.tick(&controls, &mut playback, &mut capture, &mut tick);
    }
    reader.take_limited_ticks()
}

#[test]
fn a_full_scale_square_through_the_processed_path_counts_limited_ticks() {
    assert!(limited_ticks(TickMix::Processed, FULL_SCALE) > 0);
}

#[test]
fn a_quiet_signal_never_counts_a_limited_tick() {
    assert_eq!(limited_ticks(TickMix::Processed, QUIET), 0);
}

#[test]
fn the_bypass_path_has_no_limiter_and_never_counts_a_limited_tick() {
    assert_eq!(limited_ticks(TickMix::Bypass, FULL_SCALE), 0);
}

#[test]
fn reading_the_count_clears_it() {
    let mut meters = Meters::new();
    let (_, mut reader) = meters.split();
    assert_eq!(reader.take_limited_ticks(), 0);
}
