use dsp::{Frame, FramesPerTick, I2sWord, MAX_FRAMES};
use stream::{
    DMA_BUFFER_FRAMES, DmaBuffer, DmaCursor, DmaTransport, Frames, TRANSMIT_LEAD_BLOCKS,
    TickBlocks, XferCount, tick_counter_start,
};

mod common;

use common::XorShift;

const FRAME_COUNTS: [FramesPerTick; 3] = [
    FramesPerTick::Six,
    FramesPerTick::Twelve,
    FramesPerTick::TwentyFour,
];
const FACTORY_LAST_TRANSFER: u32 = 191;
const XFERCOUNT_SHIFT: u32 = 16;
const FACTORY_RECEIVE_XFERCFG_WITHOUT_COUNT: u32 = 0x0000_4203;

fn reading(xfercount: u32) -> XferCount {
    XferCount::from_xfercfg((xfercount << XFERCOUNT_SHIFT) | FACTORY_RECEIVE_XFERCFG_WITHOUT_COUNT)
}

fn reading_at(frame: u32, second_slot_moving: bool) -> XferCount {
    let moved = 2 * frame + u32::from(second_slot_moving);
    reading(FACTORY_LAST_TRANSFER - moved)
}

fn numbered_buffer() -> DmaBuffer {
    let buffer = DmaBuffer::silent();
    for word in 0..2 * DMA_BUFFER_FRAMES as u32 {
        buffer.store_word(word as usize, 0x1000 + word);
    }
    buffer
}

fn words_of(buffer: &DmaBuffer) -> Vec<u32> {
    (0..2 * DMA_BUFFER_FRAMES)
        .map(|word| buffer.load_word(word))
        .collect()
}

fn numbered_block(first: i32) -> [Frame<I2sWord>; MAX_FRAMES] {
    core::array::from_fn(|frame| {
        let left = first + 2 * frame as i32;
        [I2sWord::from_bits(left), I2sWord::from_bits(left + 1)]
    })
}

#[test]
fn the_factory_reading_gives_the_frame_in_progress_and_reads_0x3ff_as_the_last_frame() {
    let cases = [
        (191, 0),
        (190, 0),
        (189, 1),
        (188, 1),
        (101, 45),
        (2, 94),
        (1, 95),
        (0, 95),
        (0x3FF, 95),
    ];
    for (xfercount, frame) in cases {
        assert_eq!(
            reading(xfercount).frame_in_progress().index(),
            frame,
            "XFERCOUNT {xfercount:#x}"
        );
    }
}

#[test]
fn a_frame_with_one_slot_moved_still_counts_as_in_progress() {
    for frame in 0..DMA_BUFFER_FRAMES as u32 {
        for second_slot_moving in [false, true] {
            assert_eq!(
                reading_at(frame, second_slot_moving)
                    .frame_in_progress()
                    .index(),
                frame
            );
        }
    }
}

fn transport_with(received: DmaBuffer, played: DmaBuffer, mirrored: DmaBuffer) -> DmaTransport {
    DmaTransport {
        received,
        played,
        mirrored,
        cursor: DmaCursor::new(),
    }
}

#[test]
fn the_tick_moves_the_received_block_that_ended_where_the_block_in_progress_began() {
    let transport = transport_with(numbered_buffer(), DmaBuffer::silent(), DmaBuffer::silent());
    for frames in FRAME_COUNTS {
        let n = frames.count() as u32;
        let blocks = DMA_BUFFER_FRAMES as u32 / n;
        for position in 0..DMA_BUFFER_FRAMES as u32 {
            let in_progress = position / n;
            let completed = (in_progress + blocks - 1) % blocks;
            let mut into = [[I2sWord::from_bits(-1); 2]; MAX_FRAMES];
            TickBlocks::receive(reading_at(position, false), frames, &transport, &mut into);
            for (frame, words) in into.iter().enumerate() {
                let expected = if (frame as u32) < n {
                    let word = 2 * (completed * n + frame as u32);
                    [0x1000 + word as i32, 0x1001 + word as i32]
                } else {
                    [-1, -1]
                };
                assert_eq!(
                    words.map(I2sWord::to_bits),
                    expected,
                    "{n} frames per tick at position {position}, frame {frame}"
                );
            }
        }
    }
}

#[test]
fn the_tick_writes_the_same_block_into_both_transmit_buffers_two_blocks_ahead_and_nothing_else() {
    for frames in FRAME_COUNTS {
        let n = frames.count() as u32;
        let blocks = DMA_BUFFER_FRAMES as u32 / n;
        for position in 0..DMA_BUFFER_FRAMES as u32 {
            let transport =
                transport_with(DmaBuffer::silent(), DmaBuffer::silent(), numbered_buffer());
            let (first, second) = (&transport.played, &transport.mirrored);
            let before_second = words_of(second);
            TickBlocks::around(reading_at(position, true).frame_in_progress(), frames)
                .transmit(&numbered_block(0x5000), &transport);
            let written = ((position / n + TRANSMIT_LEAD_BLOCKS) % blocks) * n;
            for word in 0..2 * DMA_BUFFER_FRAMES as u32 {
                let frame = word / 2;
                let inside = (written..written + n).contains(&frame);
                let from_block = (0x5000 + word).wrapping_sub(2 * written);
                let expected_first = if inside { from_block } else { 0 };
                let expected_second = if inside {
                    from_block
                } else {
                    before_second[word as usize]
                };
                assert_eq!(
                    first.load_word(word as usize),
                    expected_first,
                    "{n} frames at position {position}, word {word} of the first buffer"
                );
                assert_eq!(
                    second.load_word(word as usize),
                    expected_second,
                    "{n} frames at position {position}, word {word} of the second buffer"
                );
            }
        }
    }
}

#[test]
fn the_transmit_step_publishes_the_blocks_it_served_to_the_cursor() {
    let transport = DmaTransport::silent();
    for served in 1..=3u16 {
        let blocks = TickBlocks::receive(
            reading_at(12, false),
            FramesPerTick::Six,
            &transport,
            &mut [[I2sWord::from_bits(0); 2]; MAX_FRAMES],
        );
        blocks.transmit(&numbered_block(0), &transport);
        assert_eq!(transport.cursor.ticks_served(), served);
    }
    let fill = transport
        .cursor
        .fill_at(reading_at(15, false).frame_in_progress());
    assert_eq!(fill.playback.frames_queued, Frames::new(15));
    assert_eq!(fill.capture, Frames::new(3));
}

#[test]
fn the_count_of_ticks_served_wraps_after_65535_and_never_disturbs_the_positions() {
    let cursor = DmaCursor::new();
    let blocks = TickBlocks::around(
        reading_at(12, false).frame_in_progress(),
        FramesPerTick::Six,
    );
    for _ in 0..65_537u32 {
        cursor.served(blocks);
    }
    assert_eq!(cursor.ticks_served(), 1);
    let fill = cursor.fill_at(reading_at(15, false).frame_in_progress());
    assert_eq!(fill.playback.frames_queued, Frames::new(15));
    assert_eq!(fill.capture, Frames::new(3));
}

#[test]
fn zeroing_the_transmit_buffers_silences_every_word_and_leaves_the_received_block() {
    let transport = transport_with(numbered_buffer(), numbered_buffer(), numbered_buffer());
    transport.zero_transmit_buffers();
    assert!(words_of(&transport.played).iter().all(|word| *word == 0));
    assert!(words_of(&transport.mirrored).iter().all(|word| *word == 0));
    assert_eq!(words_of(&transport.received), words_of(&numbered_buffer()));
}

#[test]
fn the_cursor_reports_frames_written_ahead_of_the_dma_and_frames_received_behind_the_tick() {
    let cursor = DmaCursor::new();
    cursor.served(TickBlocks::around(
        reading_at(12, false).frame_in_progress(),
        FramesPerTick::Six,
    ));
    let fill = cursor.fill_at(reading_at(15, true).frame_in_progress());
    assert_eq!(fill.playback.frames_queued, Frames::new(15));
    assert_eq!(fill.capture, Frames::new(3));

    cursor.served(TickBlocks::around(
        reading_at(72, false).frame_in_progress(),
        FramesPerTick::TwentyFour,
    ));
    let fill = cursor.fill_at(reading_at(80, false).frame_in_progress());
    assert_eq!(fill.playback.frames_queued, Frames::new(64));
    assert_eq!(fill.capture, Frames::new(8));
}

#[test]
fn the_cursor_counts_a_received_block_the_tick_has_not_moved_yet_when_its_tick_is_late() {
    let cursor = DmaCursor::new();
    cursor.served(TickBlocks::around(
        reading_at(18, false).frame_in_progress(),
        FramesPerTick::Six,
    ));
    let fill = cursor.fill_at(reading_at(25, false).frame_in_progress());
    assert_eq!(fill.capture, Frames::new(7));
    assert_eq!(fill.playback.frames_queued, Frames::new(11));
}

#[test]
fn over_random_tick_latency_the_written_block_stays_ahead_of_the_dma_and_the_moved_block_is_complete()
 {
    let mut rng = XorShift::seeded(0x35);
    for frames in FRAME_COUNTS {
        let n = frames.count() as u32;
        let buffer_frames = DMA_BUFFER_FRAMES as u32;
        let cursor = DmaCursor::new();
        let mut boundary = 0u32;
        for _ in 0..20_000 {
            boundary += n;
            let latency = rng.below(n / 2);
            let position = (boundary + latency) % buffer_frames;
            let blocks =
                TickBlocks::around(reading_at(position, false).frame_in_progress(), frames);
            cursor.served(blocks);
            let fill = cursor.fill_at(reading_at(position, false).frame_in_progress());
            assert!(
                fill.capture.count() < n,
                "{n} frames: {} received frames left behind at position {position}",
                fill.capture.count()
            );
            let lead = fill.playback.frames_queued.count();
            assert!(
                lead > TRANSMIT_LEAD_BLOCKS * n && lead <= (TRANSMIT_LEAD_BLOCKS + 1) * n,
                "{n} frames: {lead} frames queued at position {position}"
            );
            let next_tick_position = (boundary + n + n / 2 - 1) % buffer_frames;
            let later = cursor.fill_at(reading_at(next_tick_position, true).frame_in_progress());
            assert!(
                later.playback.frames_queued.count() > n,
                "{n} frames: the DMA reached the block the tick just wrote"
            );
        }
    }
}

fn reading_after(transfers: u32) -> XferCount {
    reading(FACTORY_LAST_TRANSFER - transfers % (2 * DMA_BUFFER_FRAMES as u32))
}

fn received_block(reading: XferCount, frames: FramesPerTick, transport: &DmaTransport) -> u32 {
    let mut into = [[I2sWord::from_bits(0); 2]; MAX_FRAMES];
    TickBlocks::receive(reading, frames, transport, &mut into);
    (into[0][0].to_bits() as u32 - 0x1000) / (2 * frames.count() as u32)
}

#[test]
fn at_the_tick_phase_a_read_one_word_early_or_a_tick_up_to_n_minus_two_frames_late_never_repeats_or_skips_a_block()
 {
    let transport = transport_with(numbered_buffer(), DmaBuffer::silent(), DmaBuffer::silent());
    for frames in FRAME_COUNTS {
        let n = frames.count() as u32;
        let blocks = DMA_BUFFER_FRAMES as u32 / n;
        let first_match = n - tick_counter_start(frames);
        let mut expected = received_block(reading_after(2 * first_match), frames, &transport);
        for tick in 0..3 * blocks {
            let boundary_and_phase = first_match + tick * n;
            for latency in 0..=n - 2 {
                let frame = boundary_and_phase + latency;
                for (early, transfers) in [(false, 2 * frame), (true, 2 * frame - 1)] {
                    assert_eq!(
                        received_block(reading_after(transfers), frames, &transport),
                        expected,
                        "{n} frames per tick, tick {tick}, {latency} frames late, one transfer early: {early}"
                    );
                }
            }
            expected = (expected + 1) % blocks;
        }
    }
}
