use dsp::{
    CaptureBlock, CaptureGainTargets, CoefficientSet, FramesPerTick, Gain, Generation, I2sWord,
    InputMute, Meters, MonitorMix, PeqPreset, Pipeline, PlaybackBlock, SampleRate, StreamSample,
    TickControls, TickMix,
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

const ALL_FRAME_COUNTS: [FramesPerTick; 3] = [
    FramesPerTick::Six,
    FramesPerTick::Twelve,
    FramesPerTick::TwentyFour,
];
const QUIET_ADC_WORD: i32 = 0x0010_0000;
const QUIET_CODE_AT_UNITY: i64 = 0x1000;
const DOUBLE: Gain = Gain::from_q5_27(0x1000_0000);
const HALF: Gain = Gain::from_q5_27(0x0400_0000);
const LARGEST_GAIN: Gain = Gain::from_q5_27(i32::MAX);
const SMALLEST_GAIN: Gain = Gain::from_q5_27(i32::MIN);
const INPUTS: [usize; 2] = [0, 1];

type InputCodes = [Vec<i64>; 2];

fn codes_of_one_tick(
    pipeline: &mut Pipeline,
    frames: FramesPerTick,
    capture_gains: [Gain; 2],
    adc_words: [i32; 2],
) -> InputCodes {
    let n = frames.count();
    let mut block = CaptureBlock::new();
    block.input[..n].fill(adc_words.map(I2sWord::from_bits));
    capture_tick(pipeline, frames, capture_gains, &mut block);
    INPUTS.map(|input| {
        block.output[..n]
            .iter()
            .map(|frame| {
                assert_eq!(frame[input].to_bits() & 0xFF, 0, "low byte stays zero");
                i64::from(frame[input].to_bits() >> 8)
            })
            .collect()
    })
}

fn settled_code(gain: Gain, frames: FramesPerTick, adc_word: i32) -> i64 {
    let mut pipeline = Pipeline::default();
    codes_of_one_tick(&mut pipeline, frames, [gain; 2], [adc_word; 2]);
    let [codes, _] = codes_of_one_tick(&mut pipeline, frames, [gain; 2], [adc_word; 2]);
    assert!(
        codes.iter().all(|c| *c == codes[0]),
        "a settled gain is constant"
    );
    codes[0]
}

fn assert_ramps_smoothly_onto(start_code: i64, codes: &[i64], target_code: i64) {
    let frames = codes.len() as i64;
    let span = (target_code - start_code).abs();
    let mut previous = start_code;
    for (frame, code) in codes.iter().enumerate() {
        let rise = (code - previous) * (target_code - start_code).signum();
        assert!(
            rise >= 0,
            "frame {frame} moves away from the target: {codes:?}"
        );
        assert!(
            rise * frames <= span + 2 * frames,
            "frame {frame} steps {rise} codes, more than 1/{frames} of the {span}-code span: {codes:?}"
        );
        previous = *code;
    }
    assert_eq!(
        *codes.last().unwrap(),
        target_code,
        "the ramp ends exactly on the target"
    );
}

#[test]
fn each_input_ramps_across_one_tick_and_ends_exactly_on_its_own_target() {
    for frames in ALL_FRAME_COUNTS {
        let mut pipeline = Pipeline::default();
        let [first, second] =
            codes_of_one_tick(&mut pipeline, frames, [DOUBLE, HALF], [QUIET_ADC_WORD; 2]);
        assert_ramps_smoothly_onto(QUIET_CODE_AT_UNITY, &first, 2 * QUIET_CODE_AT_UNITY);
        assert_ramps_smoothly_onto(QUIET_CODE_AT_UNITY, &second, QUIET_CODE_AT_UNITY / 2);
        assert_ne!(
            first[0],
            2 * QUIET_CODE_AT_UNITY,
            "{frames:?}: the first frame is not yet at the target"
        );
        assert_eq!(
            codes_of_one_tick(&mut pipeline, frames, [DOUBLE, HALF], [QUIET_ADC_WORD; 2]),
            [
                vec![2 * QUIET_CODE_AT_UNITY; frames.count()],
                vec![QUIET_CODE_AT_UNITY / 2; frames.count()]
            ],
            "{frames:?}: the tick after the ramp holds both targets"
        );
    }
}

#[test]
fn an_input_whose_gain_holds_stays_bit_exact_while_the_other_input_ramps() {
    let words: [[i32; 2]; 6] = [
        [0x0010_0000, 0x7FFF_FF00],
        [0x0010_0000, i32::MIN],
        [0x0010_0000, 0x1234_5600],
        [0x0010_0000, -0x0000_0100],
        [0x0010_0000, 0x0ABC_DE00],
        [0x0010_0000, -0x0080_0000],
    ];
    let mut pipeline = Pipeline::default();
    let mut block = CaptureBlock::new();
    block.input[..6].copy_from_slice(&words.map(|f| f.map(I2sWord::from_bits)));
    capture_tick(
        &mut pipeline,
        FramesPerTick::Six,
        [DOUBLE, Gain::UNITY],
        &mut block,
    );
    let held: Vec<i32> = block.output[..6].iter().map(|f| f[1].to_bits()).collect();
    assert_eq!(held, words.map(|f| f[1]));
    let ramped: Vec<i32> = block.output[..6].iter().map(|f| f[0].to_bits()).collect();
    assert_ne!(
        ramped[0], ramped[5],
        "the first input ramps in the same tick"
    );
}

#[test]
fn a_loud_ramp_lands_on_the_target_even_when_the_change_does_not_divide_by_the_frames() {
    const LOUD_ADC_WORD: i32 = 0x7FFF_FF00;
    const CHANGE_LEAVING_THE_LARGEST_REMAINDER_AT_EVERY_FRAME_COUNT: i32 = -0x0400_0007;
    let lowered =
        Gain::from_q5_27(0x0800_0000 + CHANGE_LEAVING_THE_LARGEST_REMAINDER_AT_EVERY_FRAME_COUNT);
    let raised =
        Gain::from_q5_27(0x0800_0000 - CHANGE_LEAVING_THE_LARGEST_REMAINDER_AT_EVERY_FRAME_COUNT);
    for frames in ALL_FRAME_COUNTS {
        let mut pipeline = Pipeline::default();
        let [first, second] = codes_of_one_tick(
            &mut pipeline,
            frames,
            [lowered, raised],
            [LOUD_ADC_WORD, LOUD_ADC_WORD / 2],
        );
        assert_ramps_smoothly_onto(
            settled_code(Gain::UNITY, frames, LOUD_ADC_WORD),
            &first,
            settled_code(lowered, frames, LOUD_ADC_WORD),
        );
        assert_ramps_smoothly_onto(
            settled_code(Gain::UNITY, frames, LOUD_ADC_WORD / 2),
            &second,
            settled_code(raised, frames, LOUD_ADC_WORD / 2),
        );
    }
}

#[test]
fn each_input_spans_the_whole_gain_range_in_both_directions_at_once() {
    for frames in ALL_FRAME_COUNTS {
        let mut pipeline = Pipeline::default();
        codes_of_one_tick(
            &mut pipeline,
            frames,
            [SMALLEST_GAIN, LARGEST_GAIN],
            [QUIET_ADC_WORD; 2],
        );
        let [rising, falling] = codes_of_one_tick(
            &mut pipeline,
            frames,
            [LARGEST_GAIN, SMALLEST_GAIN],
            [QUIET_ADC_WORD; 2],
        );
        let smallest = settled_code(SMALLEST_GAIN, frames, QUIET_ADC_WORD);
        let largest = settled_code(LARGEST_GAIN, frames, QUIET_ADC_WORD);
        assert_ramps_smoothly_onto(smallest, &rising, largest);
        assert_ramps_smoothly_onto(largest, &falling, smallest);
    }
}

struct XorShift32(u32);

impl XorShift32 {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
}

#[test]
fn random_publishes_ramp_each_input_from_its_applied_gain_onto_its_latest_target() {
    let targets = CaptureGainTargets::new();
    let mut random = XorShift32(0x0600_0006);
    let mut pipeline = Pipeline::default();
    let mut applied = [Gain::UNITY; 2];
    for _ in 0..3_000 {
        let frames = ALL_FRAME_COUNTS[(random.next() % 3) as usize];
        let mut published = applied;
        for _ in 0..random.next() % 3 {
            let input = (random.next() % 2) as usize;
            published[input] = Gain::from_q5_27(random.next() as i32);
            targets.publish(published);
        }
        let latest = targets.latest();
        assert_eq!(latest, published, "the tick adopts the latest publish");
        let codes = codes_of_one_tick(&mut pipeline, frames, latest, [QUIET_ADC_WORD; 2]);
        for input in INPUTS {
            assert_ramps_smoothly_onto(
                settled_code(applied[input], frames, QUIET_ADC_WORD),
                &codes[input],
                settled_code(latest[input], frames, QUIET_ADC_WORD),
            );
        }
        applied = latest;
    }
}

#[test]
fn targets_published_at_one_frame_count_still_end_on_target_at_another() {
    for set_at in ALL_FRAME_COUNTS {
        for run_at in ALL_FRAME_COUNTS {
            let targets = CaptureGainTargets::new();
            let mut pipeline = Pipeline::default();
            codes_of_one_tick(&mut pipeline, set_at, targets.latest(), [QUIET_ADC_WORD; 2]);
            targets.publish([DOUBLE, HALF]);
            let [first, second] =
                codes_of_one_tick(&mut pipeline, run_at, targets.latest(), [QUIET_ADC_WORD; 2]);
            assert_ramps_smoothly_onto(QUIET_CODE_AT_UNITY, &first, 2 * QUIET_CODE_AT_UNITY);
            assert_ramps_smoothly_onto(QUIET_CODE_AT_UNITY, &second, QUIET_CODE_AT_UNITY / 2);
        }
    }
}

#[test]
fn capture_is_bit_exact_again_once_both_inputs_ramp_back_to_unity() {
    let mut pipeline = Pipeline::default();
    codes_of_one_tick(
        &mut pipeline,
        FramesPerTick::Six,
        [DOUBLE, HALF],
        [QUIET_ADC_WORD; 2],
    );
    codes_of_one_tick(
        &mut pipeline,
        FramesPerTick::Six,
        [Gain::UNITY; 2],
        [QUIET_ADC_WORD; 2],
    );

    let words: [[i32; 2]; 6] = [
        [0x7FFF_FF00, i32::MIN],
        [0x1234_5600, -0x1234_5600],
        [0x0000_0100, -0x0000_0100],
        [0, 0x0ABC_DE00],
        [i32::MIN, 0x7FFF_FF00],
        [0x0080_0000, -0x0080_0000],
    ];
    let mut block = CaptureBlock::new();
    block.input[..6].copy_from_slice(&words.map(|f| f.map(I2sWord::from_bits)));
    capture_tick(
        &mut pipeline,
        FramesPerTick::Six,
        [Gain::UNITY; 2],
        &mut block,
    );
    let out: Vec<[i32; 2]> = block.output[..6]
        .iter()
        .map(|f| f.map(StreamSample::to_bits))
        .collect();
    assert_eq!(out, words);
}

static SHARED_WITH_THE_TICK: CaptureGainTargets = CaptureGainTargets::new();

#[test]
fn targets_start_at_unity_and_live_in_a_static_shared_by_thread_mode_and_the_tick() {
    assert_eq!(SHARED_WITH_THE_TICK.latest(), [Gain::UNITY; 2]);
    SHARED_WITH_THE_TICK.publish([HALF, DOUBLE]);
    SHARED_WITH_THE_TICK.publish([DOUBLE, HALF]);
    assert_eq!(SHARED_WITH_THE_TICK.latest(), [DOUBLE, HALF]);
}
