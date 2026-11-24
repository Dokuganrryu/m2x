use core::sync::atomic::AtomicU32;

use dsp::{Frame, FramesPerTick, MAX_FRAMES, SampleRate, StreamSample};
use stream::{AlternateSetting, Frames, Playback, PlaybackEndpoint, PlaybackTick};

const SILENCE: Frame<StreamSample> = [StreamSample::from_bits(0); 2];
const SIX: FramesPerTick = FramesPerTick::Six;
const PRIMING_BLOCKS: usize = 4;

fn frame(n: i32) -> Frame<StreamSample> {
    [
        StreamSample::from_bits(n << 8),
        StreamSample::from_bits(-(n << 8)),
    ]
}

fn packet(first: i32) -> [Frame<StreamSample>; 6] {
    core::array::from_fn(|index| frame(first + index as i32))
}

fn wire_packet() -> [AtomicU32; 12] {
    core::array::from_fn(|index| AtomicU32::new((index as u32 + 1) << 8))
}

fn render(tick: &mut PlaybackTick<'_>) -> [Frame<StreamSample>; 6] {
    let mut out = [[StreamSample::from_bits(0x7fff_ff00); 2]; MAX_FRAMES];
    tick.render(SIX, &mut out);
    core::array::from_fn(|index| out[index])
}

#[test]
fn packets_before_the_gate_opens_are_dropped_and_the_tick_renders_silence() {
    let mut playback = Playback::new();
    let (usb, mut tick, counters, gate) = playback.split();
    let mut endpoint = PlaybackEndpoint::new(usb, SampleRate::Hz48000);
    endpoint.select_alternate(AlternateSetting::One);
    assert_eq!(render(&mut tick), [SILENCE; 6]);

    for microframe in 0..(3 * PRIMING_BLOCKS as i32) {
        assert_eq!(
            endpoint.push_packet(&packet(1000 + 6 * microframe)),
            Frames::ZERO
        );
        assert_eq!(endpoint.push_wire_words(&wire_packet()), Frames::ZERO);
        assert_eq!(render(&mut tick), [SILENCE; 6], "microframe {microframe}");
    }
    assert_eq!(counters.take().overruns, 0);

    gate.open();
    for block in 0..PRIMING_BLOCKS as i32 {
        assert_eq!(render(&mut tick), [SILENCE; 6]);
        assert_eq!(endpoint.push_packet(&packet(1 + 6 * block)), Frames::new(6));
    }
    assert_eq!(render(&mut tick), packet(1));
    assert_eq!(endpoint.push_wire_words(&wire_packet()), Frames::new(6));
}

#[test]
fn a_rate_change_does_not_close_the_gate() {
    let mut playback = Playback::new();
    let (usb, mut tick, _, gate) = playback.split();
    let mut endpoint = PlaybackEndpoint::new(usb, SampleRate::Hz48000);
    gate.open();
    endpoint.select_alternate(AlternateSetting::One);
    render(&mut tick);
    assert_eq!(endpoint.push_packet(&packet(1)), Frames::new(6));

    endpoint.reset_for_new_rate(SampleRate::Hz96000);
    assert_eq!(endpoint.push_packet(&packet(7)), Frames::ZERO);
    render(&mut tick);
    assert_eq!(endpoint.push_packet(&packet(13)), Frames::new(6));
    assert_eq!(endpoint.push_wire_words(&wire_packet()), Frames::new(6));
}

#[test]
fn the_gate_does_not_bypass_the_wait_for_the_tick_to_follow_the_request() {
    let mut playback = Playback::new();
    let (usb, mut tick, _, gate) = playback.split();
    let mut endpoint = PlaybackEndpoint::new(usb, SampleRate::Hz48000);
    gate.open();
    assert_eq!(endpoint.push_packet(&packet(1)), Frames::ZERO);
    endpoint.select_alternate(AlternateSetting::One);
    assert_eq!(endpoint.push_packet(&packet(1)), Frames::ZERO);
    render(&mut tick);
    assert_eq!(endpoint.push_packet(&packet(1)), Frames::new(6));
}
