use dsp::{
    CaptureBlock, CoefficientSet, FramesPerTick, Gain, Generation, I2sWord, InputMute, Meters,
    MonitorMix, PeqPreset, Pipeline, PlaybackBlock, SampleRate, StreamSample, TickControls,
    TickMix,
};

fn capture_tick(
    pipeline: &mut Pipeline,
    frames: FramesPerTick,
    capture_gains: [Gain; 2],
    capture: &mut CaptureBlock,
) {
    let flat = CoefficientSet::design(&PeqPreset::FLAT, SampleRate::Hz48000, Generation::FIRST);
    let controls = TickControls {
        frames,
        mix: TickMix::Bypass,
        coefficients: &flat,
        monitor: MonitorMix::OFF,
        inputs: [InputMute::Live; 2],
        capture_gains,
    };
    let mut meters = Meters::new();
    let (mut meter_tick, _) = meters.split();
    pipeline.tick(
        &controls,
        &mut PlaybackBlock::new(),
        capture,
        &mut meter_tick,
    );
}

fn capture_one(gain: Gain, adc_words: [i32; 2]) -> [i32; 2] {
    let mut pipeline = Pipeline::default();
    let mut block = CaptureBlock::new();
    block.input[..6].copy_from_slice(&[adc_words.map(I2sWord::from_bits); 6]);
    capture_tick(&mut pipeline, FramesPerTick::Six, [gain; 2], &mut block);
    capture_tick(&mut pipeline, FramesPerTick::Six, [gain; 2], &mut block);
    assert!(block.output[..6].iter().all(|f| *f == block.output[0]));
    block.output[0].map(StreamSample::to_bits)
}

const DOUBLE: Gain = Gain::from_q5_27(0x1000_0000);
const HALF: Gain = Gain::from_q5_27(0x0400_0000);
const MINUS_ONE: Gain = Gain::from_q5_27(-0x0800_0000);

#[test]
fn unity_rounds_a_32_bit_adc_word_to_nearest_24_bits() {
    assert_eq!(
        capture_one(Gain::UNITY, [0x1234_5680, 0x1234_567F]),
        [0x1234_5700, 0x1234_5600]
    );
    assert_eq!(
        capture_one(Gain::UNITY, [-0x1234_5680, -0x1234_5681]),
        [-0x1234_5600, -0x1234_5700]
    );
}

#[test]
fn unity_saturates_once_at_full_scale() {
    assert_eq!(
        capture_one(Gain::UNITY, [i32::MAX, i32::MIN]),
        [0x7FFF_FF00, i32::MIN]
    );
}

#[test]
fn double_gain_doubles_and_saturates() {
    assert_eq!(
        capture_one(DOUBLE, [0x1234_5600, -0x1234_5600]),
        [0x2468_AC00, -0x2468_AC00]
    );
    assert_eq!(
        capture_one(DOUBLE, [0x4000_0000, -0x4000_0100]),
        [0x7FFF_FF00, i32::MIN]
    );
}

#[test]
fn half_gain_rounds_the_dropped_bit_to_nearest() {
    assert_eq!(
        capture_one(HALF, [0x0000_0300, 0x0000_0100]),
        [0x0000_0200, 0x0000_0100]
    );
}

#[test]
fn minus_one_inverts_and_saturates_the_minimum_code() {
    assert_eq!(
        capture_one(MINUS_ONE, [0x1234_5600, i32::MIN]),
        [-0x1234_5600, 0x7FFF_FF00]
    );
}
