use dsp::{Frame, SampleRate, StreamSample};
use stream::{
    AlternateSetting, CapturePacket, CaptureUsb, FrameRing, Frames, MAX_CAPTURE_PACKET_FRAMES,
    RingWriter,
};

const SILENCE: Frame<StreamSample> = [StreamSample::from_bits(0); 2];
const UNTOUCHED: Frame<StreamSample> = [StreamSample::from_bits(0x7fff_ff00); 2];

fn word(n: i32) -> Frame<StreamSample> {
    [
        StreamSample::from_bits(n << 8),
        StreamSample::from_bits(-(n << 8)),
    ]
}

fn number_of(frame: Frame<StreamSample>) -> i32 {
    frame[0].to_bits() >> 8
}

struct Adc {
    next: i32,
}

impl Adc {
    fn tick(&mut self, writer: &mut RingWriter<'_>, frames: usize) {
        let mut block = [SILENCE; 24];
        for frame in block.iter_mut().take(frames) {
            *frame = word(self.next);
            self.next += 1;
        }
        writer.push(&block[..frames]);
    }
}

fn send(usb: &mut CaptureUsb<'_>, dma: u32) -> (Option<Frames>, CapturePacket) {
    let mut packet = [UNTOUCHED; MAX_CAPTURE_PACKET_FRAMES];
    let size = usb.fill_packet(Frames::new(dma), &mut packet);
    (size, packet)
}

#[test]
fn at_alternate_setting_zero_no_packet_is_sent_and_the_buffer_is_untouched() {
    let mut ring = FrameRing::new();
    let (mut writer, reader) = ring.split();
    let mut usb = CaptureUsb::new(reader, SampleRate::Hz48000);
    Adc { next: 1 }.tick(&mut writer, 6);
    let (size, packet) = send(&mut usb, 0);
    assert_eq!(size, None);
    assert_eq!(packet, [UNTOUCHED; MAX_CAPTURE_PACKET_FRAMES]);
}

#[test]
fn after_alternate_setting_one_stale_frames_are_dropped_and_silence_goes_out_until_the_target() {
    let mut ring = FrameRing::new();
    let (mut writer, reader) = ring.split();
    let mut usb = CaptureUsb::new(reader, SampleRate::Hz48000);
    let mut adc = Adc { next: 1 };
    adc.tick(&mut writer, 6);
    adc.tick(&mut writer, 6);
    usb.select_alternate(AlternateSetting::One);
    for _ in 0..3 {
        adc.tick(&mut writer, 6);
        let (size, packet) = send(&mut usb, 0);
        assert_eq!(size, Some(Frames::new(6)));
        assert_eq!(packet[..6], [SILENCE; 6]);
        assert_eq!(packet[6], UNTOUCHED);
    }
    adc.tick(&mut writer, 6);
    let (size, packet) = send(&mut usb, 0);
    assert_eq!(size, Some(Frames::new(6)));
    assert_eq!(
        packet[..6],
        [word(13), word(14), word(15), word(16), word(17), word(18)]
    );
    assert_eq!(packet[6], UNTOUCHED);
}

#[test]
fn frames_still_in_the_rx_dma_buffer_count_toward_the_target() {
    let mut ring = FrameRing::new();
    let (mut writer, reader) = ring.split();
    let mut usb = CaptureUsb::new(reader, SampleRate::Hz48000);
    usb.select_alternate(AlternateSetting::One);
    Adc { next: 1 }.tick(&mut writer, 18);
    let (_, packet) = send(&mut usb, 5);
    assert_eq!(packet[0], SILENCE);
    let (_, packet) = send(&mut usb, 6);
    assert_eq!(packet[0], word(1));
}

#[test]
fn when_the_ring_runs_short_the_packet_keeps_its_size_and_ends_in_silence() {
    let mut ring = FrameRing::new();
    let (mut writer, reader) = ring.split();
    let mut usb = CaptureUsb::new(reader, SampleRate::Hz48000);
    usb.select_alternate(AlternateSetting::One);
    Adc { next: 1 }.tick(&mut writer, 3);
    let (size, packet) = send(&mut usb, 24);
    assert_eq!(size, Some(Frames::new(6)));
    assert_eq!(
        packet[..6],
        [word(1), word(2), word(3), SILENCE, SILENCE, SILENCE]
    );
}

#[test]
fn a_rate_change_while_streaming_drops_the_ring_and_fills_to_the_new_target() {
    let mut ring = FrameRing::new();
    let (mut writer, reader) = ring.split();
    let mut usb = CaptureUsb::new(reader, SampleRate::Hz48000);
    let mut adc = Adc { next: 1 };
    usb.select_alternate(AlternateSetting::One);
    adc.tick(&mut writer, 24);
    assert_eq!(send(&mut usb, 0).1[0], word(1));
    usb.reset_for_new_rate(SampleRate::Hz96000);
    adc.tick(&mut writer, 12);
    adc.tick(&mut writer, 12);
    adc.tick(&mut writer, 12);
    let (size, packet) = send(&mut usb, 0);
    assert_eq!(size, Some(Frames::new(12)));
    assert_eq!(packet[..12], [SILENCE; 12]);
    adc.tick(&mut writer, 12);
    let (_, packet) = send(&mut usb, 0);
    assert_eq!(packet[0], word(25));
}

fn stream_ten_seconds(rate: SampleRate, ticks_per_second: u64, frames_per_tick: usize) {
    let mut ring = FrameRing::new();
    let (mut writer, reader) = ring.split();
    let mut usb = CaptureUsb::new(reader, rate);
    let mut adc = Adc { next: 1 };
    usb.select_alternate(AlternateSetting::One);
    let mut ticks = 0u64;
    let mut next_expected = None;
    for microframe in 1..=80_000u64 {
        while (ticks + 1) * 8000 <= microframe * ticks_per_second {
            adc.tick(&mut writer, frames_per_tick);
            ticks += 1;
        }
        let elapsed_in_tick = microframe * ticks_per_second - ticks * 8000;
        let dma = elapsed_in_tick * frames_per_tick as u64 / 8000;
        let (size, packet) = send(&mut usb, dma as u32);
        let size = size
            .expect("a started stream sends every microframe")
            .count() as usize;
        for &frame in &packet[..size] {
            match (next_expected, number_of(frame)) {
                (None, 0) => {}
                (None, first) => next_expected = Some(first + 1),
                (Some(expected), number) => {
                    assert_eq!(number, expected, "{rate:?} at microframe {microframe}");
                    next_expected = Some(expected + 1);
                }
            }
        }
    }
    let sent = next_expected.expect("audio started") - 1;
    assert!(
        adc.next - 1 - sent <= 6 * frames_per_tick as i32,
        "{rate:?}"
    );
}

#[test]
fn over_ten_seconds_at_each_rate_family_every_captured_frame_goes_out_once_and_in_order() {
    stream_ten_seconds(SampleRate::Hz44100, 7350, 6);
    stream_ten_seconds(SampleRate::Hz48000, 8000, 6);
    stream_ten_seconds(SampleRate::Hz88200, 7350, 12);
    stream_ten_seconds(SampleRate::Hz192000, 8000, 24);
}
