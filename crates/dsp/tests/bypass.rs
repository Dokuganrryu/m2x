use dsp::{
    CaptureBlock, CoefficientSet, Frame, FramesPerTick, Gain, Generation, I2sWord, InputMute,
    MeterTick, Meters, MonitorMix, PeqPreset, Pipeline, PlaybackBlock, SampleRate, StreamSample,
    TickControls, TickMix,
};

fn tick_in_bypass(
    pipeline: &mut Pipeline,
    frames: FramesPerTick,
    playback: &mut PlaybackBlock,
    capture: &mut CaptureBlock,
    meters: &mut MeterTick,
) {
    let flat = CoefficientSet::design(&PeqPreset::FLAT, SampleRate::Hz48000, Generation::FIRST);
    let controls = TickControls {
        frames,
        mix: TickMix::Bypass,
        coefficients: &flat,
        monitor: MonitorMix::OFF,
        inputs: [InputMute::Live; 2],
        capture_gains: [Gain::UNITY; 2],
    };
    pipeline.tick(&controls, playback, capture, meters);
}

fn stream_frames<const N: usize>(words: [[i32; 2]; N]) -> [Frame<StreamSample>; N] {
    words.map(|frame| frame.map(StreamSample::from_bits))
}

fn i2s_bits(frames: &[Frame<I2sWord>]) -> Vec<[i32; 2]> {
    frames.iter().map(|f| f.map(I2sWord::to_bits)).collect()
}

fn stream_bits(frames: &[Frame<StreamSample>]) -> Vec<[i32; 2]> {
    frames
        .iter()
        .map(|f| f.map(StreamSample::to_bits))
        .collect()
}

#[test]
fn playback_is_the_input_one_tick_later() {
    let mut pipeline = Pipeline::default();
    let mut block = PlaybackBlock::new();
    let mut capture = CaptureBlock::new();
    let mut meters = Meters::new();
    let (mut meter_tick, _) = meters.split();
    let first = [
        [0x0000_0100, -0x0000_0100],
        [0x7FFF_FF00, i32::MIN],
        [0x1234_5600, 0x0ABC_DE00],
        [0, 0x0000_0100],
        [-0x1234_5600, 0x7FFF_FF00],
        [0x0080_0000, -0x0080_0000],
    ];
    block.input[..6].copy_from_slice(&stream_frames(first));
    tick_in_bypass(
        &mut pipeline,
        FramesPerTick::Six,
        &mut block,
        &mut capture,
        &mut meter_tick,
    );
    assert_eq!(
        i2s_bits(&block.output[..6]),
        [[0, 0]; 6],
        "first tick plays silence"
    );

    block.input[..6].copy_from_slice(&stream_frames([[0x0000_0200, 0]; 6]));
    tick_in_bypass(
        &mut pipeline,
        FramesPerTick::Six,
        &mut block,
        &mut capture,
        &mut meter_tick,
    );
    assert_eq!(
        i2s_bits(&block.output[..6]),
        first,
        "second tick plays the first tick's input"
    );
}

struct RepeatableWords(u32);

impl RepeatableWords {
    fn next_24_bit(&mut self) -> i32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 & 0xFFFF_FF00) as i32
    }
}

const FULL_SCALE: i32 = 0x7FFF_FF00;
const MINIMUM_CODE: i32 = i32::MIN;

const FRAMES_PER_TICK_AT_EACH_RATE: [(u32, FramesPerTick); 6] = [
    (44_100, FramesPerTick::Six),
    (48_000, FramesPerTick::Six),
    (88_200, FramesPerTick::Twelve),
    (96_000, FramesPerTick::Twelve),
    (176_400, FramesPerTick::TwentyFour),
    (192_000, FramesPerTick::TwentyFour),
];

fn fill_tick<S>(
    words: &mut RepeatableWords,
    tick: usize,
    frames: &mut [Frame<S>],
    sample: fn(i32) -> S,
) {
    for (i, frame) in frames.iter_mut().enumerate() {
        let pair = match (tick + i) % 5 {
            0 => [FULL_SCALE, MINIMUM_CODE],
            1 => [MINIMUM_CODE, FULL_SCALE],
            _ => [words.next_24_bit(), words.next_24_bit()],
        };
        *frame = pair.map(sample);
    }
}

#[test]
fn bypass_is_bit_exact_at_every_rate() {
    for (rate, frames) in FRAMES_PER_TICK_AT_EACH_RATE {
        let n = frames.count();
        let mut pipeline = Pipeline::default();
        let mut meters = Meters::new();
        let (mut meter_tick, _) = meters.split();
        let mut playback = PlaybackBlock::new();
        let mut capture = CaptureBlock::new();
        let mut words = RepeatableWords(rate);
        let mut previous = vec![[0; 2]; n];

        for tick in 0..2_000 {
            fill_tick(
                &mut words,
                tick,
                &mut playback.input[..n],
                StreamSample::from_bits,
            );
            fill_tick(
                &mut words,
                tick + 1,
                &mut capture.input[..n],
                I2sWord::from_bits,
            );

            tick_in_bypass(
                &mut pipeline,
                frames,
                &mut playback,
                &mut capture,
                &mut meter_tick,
            );

            assert_eq!(
                i2s_bits(&playback.output[..n]),
                previous,
                "{rate} Hz tick {tick}: playback is the input one tick later"
            );
            assert_eq!(
                stream_bits(&capture.output[..n]),
                capture.input[..n]
                    .iter()
                    .map(|f| f.map(I2sWord::to_bits))
                    .collect::<Vec<_>>(),
                "{rate} Hz tick {tick}: capture is the ADC's top 24 bits at unity gain"
            );
            previous = stream_bits(&playback.input[..n]);
        }
    }
}
