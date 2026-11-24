use std::sync::atomic::{AtomicBool, Ordering};

use dsp::{
    CaptureBlock, ClippedTicks, FramesPerTick, I2sWord, MAX_FRAMES, MeterChannel, MeterReading,
    Meters, PeakLevel, PlaybackBlock, StreamSample,
};

const ALL_FRAME_COUNTS: [FramesPerTick; 3] = [
    FramesPerTick::Six,
    FramesPerTick::Twelve,
    FramesPerTick::TwentyFour,
];

const ALL_CHANNELS: [MeterChannel; 4] = [
    MeterChannel::Input1,
    MeterChannel::Input2,
    MeterChannel::OutputLeft,
    MeterChannel::OutputRight,
];

const FACTORY_CLIP_THRESHOLD: u32 = 0x7FFF_FF00;

fn capture_block_with(frame_index: usize, frame: [i32; 2]) -> CaptureBlock {
    let mut block = CaptureBlock::new();
    block.output[frame_index] = frame.map(StreamSample::from_bits);
    block
}

fn playback_block_with(frame_index: usize, frame: [i32; 2]) -> PlaybackBlock {
    let mut block = PlaybackBlock::new();
    block.output[frame_index] = frame.map(I2sWord::from_bits);
    block
}

#[test]
fn a_capture_block_raises_each_input_peak_and_a_read_clears_it() {
    let mut meters = Meters::new();
    let (mut tick, mut reader) = meters.split();
    tick.record_capture(
        FramesPerTick::Six,
        &capture_block_with(3, [0x1234_5600, -0x0040_0000]),
    );

    assert_eq!(
        reader.take(MeterChannel::Input1),
        MeterReading {
            peak: PeakLevel::from_bits(0x1234_5600),
            clipped_ticks: ClippedTicks::from_count(0),
        }
    );
    assert_eq!(
        reader.take(MeterChannel::Input2).peak,
        PeakLevel::from_bits(0x003F_FFFF)
    );
    assert_eq!(reader.take(MeterChannel::Input1).peak, PeakLevel::SILENCE);
    assert_eq!(
        reader.take(MeterChannel::OutputLeft).peak,
        PeakLevel::SILENCE
    );
}

#[test]
fn a_capture_word_clips_from_0x7fffff00_and_the_most_negative_word_clips_too() {
    let mut meters = Meters::new();
    let (mut tick, mut reader) = meters.split();
    let cases = [
        (0x7FFF_FF00, true),
        (0x7FFF_FE00, false),
        (i32::MIN, true),
        (-0x7FFF_FF00, false),
    ];
    for (word, clips) in cases {
        tick.record_capture(FramesPerTick::Six, &capture_block_with(0, [word, 0]));
        let reading = reader.take(MeterChannel::Input1);
        assert_eq!(reading.peak.is_clip(), clips, "word {word:#x}");
        assert_eq!(
            reading.clipped_ticks.count(),
            clips as u32,
            "word {word:#x}"
        );
        assert_eq!(reader.take(MeterChannel::Input2).clipped_ticks.count(), 0);
    }
    assert_eq!(
        PeakLevel::CLIP,
        PeakLevel::from_bits(FACTORY_CLIP_THRESHOLD)
    );
}

#[test]
fn a_dac_bound_word_at_the_threshold_counts_one_clipped_tick_per_tick_on_its_own_output() {
    let mut meters = Meters::new();
    let (mut tick, mut reader) = meters.split();
    for _ in 0..3 {
        tick.record_playback(
            FramesPerTick::Twelve,
            &playback_block_with(11, [0x1000_0000, 0x7FFF_FFFC]),
        );
    }
    let right = reader.take(MeterChannel::OutputRight);
    assert_eq!(right.peak, PeakLevel::from_bits(0x7FFF_FFFC));
    assert_eq!(right.clipped_ticks, ClippedTicks::from_count(3));
    let left = reader.take(MeterChannel::OutputLeft);
    assert_eq!(left.peak, PeakLevel::from_bits(0x1000_0000));
    assert_eq!(left.clipped_ticks, ClippedTicks::from_count(0));
    assert_eq!(reader.take(MeterChannel::Input1), MeterReading::SILENCE);
}

#[test]
fn only_the_frames_of_the_current_tick_reach_the_meter() {
    for frames in ALL_FRAME_COUNTS {
        let mut meters = Meters::new();
        let (mut tick, mut reader) = meters.split();
        let last = frames.count() - 1;
        tick.record_playback(frames, &playback_block_with(last, [7 << 8, 0]));
        assert_eq!(
            reader.take(MeterChannel::OutputLeft).peak,
            PeakLevel::from_bits(7 << 8)
        );
        if frames.count() < MAX_FRAMES {
            let beyond = playback_block_with(frames.count(), [i32::MIN, 0]);
            tick.record_playback(frames, &beyond);
            assert_eq!(reader.take(MeterChannel::OutputLeft), MeterReading::SILENCE);
        }
    }
}

#[test]
fn a_quieter_tick_never_lowers_a_peak_the_reader_has_not_taken() {
    let mut meters = Meters::new();
    let (mut tick, mut reader) = meters.split();
    tick.record_capture(FramesPerTick::Six, &capture_block_with(0, [0x4000_0000, 0]));
    tick.record_capture(FramesPerTick::Six, &capture_block_with(0, [0x0100_0000, 0]));
    assert_eq!(
        reader.take(MeterChannel::Input1).peak,
        PeakLevel::from_bits(0x4000_0000)
    );
}

struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: u64) -> usize {
        (self.next() % bound) as usize
    }

    fn word(&mut self) -> i32 {
        let raw = self.next() as i32;
        match self.below(4) {
            0 => raw >> self.below(31),
            1 => 0x7FFF_FF00 | (raw & 0xFF),
            2 => i32::MIN | (raw & 0xFF),
            _ => raw,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct ModelCell {
    peak: u32,
    clipped_ticks: u32,
}

fn ones_complement_magnitude(word: i32) -> u32 {
    if word < 0 { !word as u32 } else { word as u32 }
}

#[test]
fn every_read_returns_the_loudest_word_and_the_clipped_ticks_since_the_previous_read() {
    let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
    let mut meters = Meters::new();
    let (mut tick, mut reader) = meters.split();
    let mut model = [ModelCell::default(); 4];
    for _ in 0..20_000 {
        if rng.below(3) == 2 {
            let index = rng.below(4);
            let expected = core::mem::take(&mut model[index]);
            assert_eq!(
                reader.take(ALL_CHANNELS[index]),
                MeterReading {
                    peak: PeakLevel::from_bits(expected.peak),
                    clipped_ticks: ClippedTicks::from_count(expected.clipped_ticks),
                }
            );
            continue;
        }
        let frames = ALL_FRAME_COUNTS[rng.below(3)];
        let words: Vec<[i32; 2]> = (0..frames.count())
            .map(|_| [rng.word(), rng.word()])
            .collect();
        let playback = rng.below(2) == 0;
        let first_cell = if playback { 2 } else { 0 };
        for ch in 0..2 {
            let peak = words
                .iter()
                .map(|frame| ones_complement_magnitude(frame[ch]))
                .max()
                .unwrap();
            let cell = &mut model[first_cell + ch];
            cell.peak = cell.peak.max(peak);
            cell.clipped_ticks += (peak >= FACTORY_CLIP_THRESHOLD) as u32;
        }
        if playback {
            let mut block = PlaybackBlock::new();
            for (slot, frame) in block.output.iter_mut().zip(&words) {
                *slot = frame.map(I2sWord::from_bits);
            }
            tick.record_playback(frames, &block);
        } else {
            let mut block = CaptureBlock::new();
            for (slot, frame) in block.output.iter_mut().zip(&words) {
                *slot = frame.map(StreamSample::from_bits);
            }
            tick.record_capture(frames, &block);
        }
    }
}

#[test]
fn a_reader_on_another_thread_loses_no_peak_and_no_clipped_tick() {
    const TICKS: u32 = 200_000;
    let loudest = FACTORY_CLIP_THRESHOLD as i32;
    let mut meters = Meters::new();
    let (mut tick, mut reader) = meters.split();
    let ticks_done = AtomicBool::new(false);
    let readings = std::thread::scope(|scope| {
        scope.spawn(|| {
            for index in 0..TICKS {
                let quiet = ((index as i32) << 8) & 0x3FFF_FF00;
                let first = if index == TICKS / 2 { loudest } else { quiet };
                tick.record_capture(FramesPerTick::Six, &capture_block_with(0, [first, loudest]));
            }
            ticks_done.store(true, Ordering::Release);
        });
        let mut readings = Vec::new();
        let mut take_both = || {
            (
                reader.take(MeterChannel::Input1),
                reader.take(MeterChannel::Input2),
            )
        };
        while !ticks_done.load(Ordering::Acquire) {
            readings.push(take_both());
        }
        readings.push(take_both());
        readings
    });
    assert!(readings.len() > 2, "the reader ran beside the tick");
    let first_clips: u32 = readings
        .iter()
        .map(|(first, _)| first.clipped_ticks.count())
        .sum();
    let second_clips: u32 = readings
        .iter()
        .map(|(_, second)| second.clipped_ticks.count())
        .sum();
    assert_eq!(first_clips, 1);
    assert_eq!(second_clips, TICKS);
    let first_peak = readings.iter().map(|(first, _)| first.peak).max();
    assert_eq!(first_peak, Some(PeakLevel::CLIP));
    assert!(
        readings
            .iter()
            .all(|(_, second)| [PeakLevel::CLIP, PeakLevel::SILENCE].contains(&second.peak))
    );
}
