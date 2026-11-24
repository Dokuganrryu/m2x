use dsp::{Frame, StreamSample};
use stream::{FrameRing, Frames, RING_CAPACITY};

fn stereo(left: i32, right: i32) -> Frame<StreamSample> {
    [
        StreamSample::from_bits(left),
        StreamSample::from_bits(right),
    ]
}

fn numbered(start: i32, count: usize) -> [Frame<StreamSample>; 32] {
    let mut frames = [stereo(0, 0); 32];
    for (offset, frame) in frames.iter_mut().take(count).enumerate() {
        let value = (start + offset as i32) << 8;
        *frame = stereo(value, -value);
    }
    frames
}

#[test]
fn frames_come_out_in_the_order_they_went_in() {
    let mut ring = FrameRing::new();
    let (mut writer, mut reader) = ring.split();
    let packet = numbered(1, 6);
    assert_eq!(writer.push(&packet[..6]), Frames::new(6));
    assert_eq!(reader.level(), Frames::new(6));

    let mut block = [stereo(7, 7); 6];
    assert_eq!(reader.pop_block(&mut block), Frames::new(6));
    assert_eq!(
        block,
        [
            stereo(256, -256),
            stereo(512, -512),
            stereo(768, -768),
            stereo(1024, -1024),
            stereo(1280, -1280),
            stereo(1536, -1536)
        ]
    );
    assert_eq!(reader.level(), Frames::ZERO);
}

#[test]
fn a_packet_larger_than_the_free_space_stores_only_the_frames_that_fit() {
    let mut ring = FrameRing::new();
    let (mut writer, reader) = ring.split();
    let packet = numbered(1, 32);
    for _ in 0..15 {
        assert_eq!(writer.push(&packet), Frames::new(32));
    }
    assert_eq!(writer.push(&packet[..20]), Frames::new(20));
    assert_eq!(writer.push(&packet), Frames::new(12));
    assert_eq!(writer.push(&packet[..1]), Frames::ZERO);
    assert_eq!(reader.level(), RING_CAPACITY);
    assert_eq!(RING_CAPACITY, Frames::new(512));
}

#[test]
fn a_block_read_from_a_short_ring_takes_what_is_there_and_leaves_the_rest_of_the_block_alone() {
    let mut ring = FrameRing::new();
    let (mut writer, mut reader) = ring.split();
    writer.push(&numbered(1, 2)[..2]);
    let mut block = [stereo(9, 9); 6];
    assert_eq!(reader.pop_block(&mut block), Frames::new(2));
    assert_eq!(block[..2], [stereo(256, -256), stereo(512, -512)]);
    assert_eq!(block[2..], [stereo(9, 9); 4]);
}

#[test]
fn discarding_empties_the_ring_and_later_frames_still_arrive() {
    let mut ring = FrameRing::new();
    let (mut writer, mut reader) = ring.split();
    writer.push(&numbered(1, 10)[..10]);
    reader.discard_all();
    assert_eq!(reader.level(), Frames::ZERO);
    assert_eq!(writer.level(), Frames::ZERO);
    writer.push(&numbered(100, 1)[..1]);
    let mut block = [stereo(0, 0); 1];
    reader.pop_block(&mut block);
    assert_eq!(block, [stereo(100 << 8, -(100 << 8))]);
}

#[test]
fn order_holds_across_many_trips_around_the_ring() {
    let mut ring = FrameRing::new();
    let (mut writer, mut reader) = ring.split();
    let mut next_in = 0;
    let mut next_out = 0;
    for round in 0..5000 {
        let packet_len = 5 + round % 3;
        writer.push(&numbered(next_in, packet_len)[..packet_len]);
        next_in += packet_len as i32;
        let mut block = [stereo(0, 0); 6];
        let taken = reader.pop_block(&mut block).count() as usize;
        for frame in &block[..taken] {
            assert_eq!(*frame, stereo(next_out << 8, -(next_out << 8)));
            next_out += 1;
        }
    }
    assert!(next_out > 4 * 512);
}
