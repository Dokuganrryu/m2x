use dsp::{Frame, FramesPerTick, MAX_FRAMES, StreamSample};
use stream::{AlternateSetting, FillRange, Frames, Playback, TxDmaPosition};

const SILENCE: Frame<StreamSample> = [StreamSample::from_bits(0); 2];

fn word(n: i32) -> Frame<StreamSample> {
    [
        StreamSample::from_bits(n << 8),
        StreamSample::from_bits(-(n << 8)),
    ]
}

struct HostPackets {
    next: i32,
}

impl HostPackets {
    fn new() -> Self {
        Self { next: 1 }
    }

    fn packet(&mut self, len: usize) -> [Frame<StreamSample>; 64] {
        let mut frames = [SILENCE; 64];
        for frame in frames.iter_mut().take(len) {
            *frame = word(self.next);
            self.next += 1;
        }
        frames
    }
}

fn fresh_output() -> [Frame<StreamSample>; MAX_FRAMES] {
    [[StreamSample::from_bits(0x7fff_ff00); 2]; MAX_FRAMES]
}

#[test]
fn after_alternate_setting_one_the_tick_outputs_silence_until_the_ring_holds_four_blocks() {
    let mut playback = Playback::new();
    let (mut usb, mut tick, _, gate) = playback.split();
    gate.open();
    let mut host = HostPackets::new();
    let six = FramesPerTick::Six;
    usb.select_alternate(AlternateSetting::One);

    let mut out = fresh_output();
    tick.render(six, &mut out);
    assert_eq!(out[..6], [SILENCE; 6]);

    for _ in 0..3 {
        usb.push_packet(&host.packet(6)[..6]);
        let mut out = fresh_output();
        tick.render(six, &mut out);
        assert_eq!(out[..6], [SILENCE; 6]);
    }
    usb.push_packet(&host.packet(6)[..6]);
    let mut out = fresh_output();
    tick.render(six, &mut out);
    assert_eq!(
        out[..6],
        [word(1), word(2), word(3), word(4), word(5), word(6)]
    );
    assert_eq!(out[6], fresh_output()[6]);
}

#[test]
fn priming_drops_the_oldest_frames_above_the_target_so_every_stream_starts_at_exactly_four_blocks()
{
    let mut playback = Playback::new();
    let (mut usb, mut tick, _, gate) = playback.split();
    gate.open();
    let mut host = HostPackets::new();
    usb.select_alternate(AlternateSetting::One);
    tick.render(FramesPerTick::Six, &mut fresh_output());
    usb.push_packet(&host.packet(29)[..29]);
    let out = render(&mut tick, FramesPerTick::Six);
    assert_eq!(
        out[..6],
        [word(6), word(7), word(8), word(9), word(10), word(11)]
    );
    assert_eq!(usb.observe_sof(dma(0)), Frames::new(18));
}

fn running_at_six(
    usb: &mut stream::PlaybackUsb<'_>,
    tick: &mut stream::PlaybackTick<'_>,
    host: &mut HostPackets,
) {
    usb.select_alternate(AlternateSetting::One);
    tick.render(FramesPerTick::Six, &mut fresh_output());
    usb.push_packet(&host.packet(24)[..24]);
}

fn render(
    tick: &mut stream::PlaybackTick<'_>,
    frames: FramesPerTick,
) -> [Frame<StreamSample>; MAX_FRAMES] {
    let mut out = fresh_output();
    tick.render(frames, &mut out);
    out
}

#[test]
fn on_underrun_the_tick_outputs_what_it_has_then_zeros_counts_one_underrun_and_primes_again() {
    let mut playback = Playback::new();
    let (mut usb, mut tick, counters, gate) = playback.split();
    gate.open();
    let mut host = HostPackets::new();
    running_at_six(&mut usb, &mut tick, &mut host);
    for _ in 0..4 {
        render(&mut tick, FramesPerTick::Six);
    }
    usb.push_packet(&host.packet(4)[..4]);
    let out = render(&mut tick, FramesPerTick::Six);
    assert_eq!(
        out[..6],
        [word(25), word(26), word(27), word(28), SILENCE, SILENCE]
    );
    assert_eq!(counters.take().underruns, 1);

    usb.push_packet(&host.packet(23)[..23]);
    assert_eq!(render(&mut tick, FramesPerTick::Six)[..6], [SILENCE; 6]);
    usb.push_packet(&host.packet(1)[..1]);
    assert_eq!(render(&mut tick, FramesPerTick::Six)[0], word(29));
    assert_eq!(counters.take().underruns, 0);
}

#[test]
fn frames_that_do_not_fit_are_dropped_whole_and_each_short_packet_counts_one_overrun() {
    let mut playback = Playback::new();
    let (mut usb, mut tick, counters, gate) = playback.split();
    gate.open();
    let mut host = HostPackets::new();
    usb.select_alternate(AlternateSetting::One);
    render(&mut tick, FramesPerTick::Six);
    for _ in 0..8 {
        assert_eq!(usb.push_packet(&host.packet(60)[..60]), Frames::new(60));
    }
    assert_eq!(usb.push_packet(&host.packet(60)[..60]), Frames::new(32));
    assert_eq!(usb.push_packet(&host.packet(6)[..6]), Frames::ZERO);
    assert_eq!(counters.take().overruns, 2);
    assert_eq!(
        render(&mut tick, FramesPerTick::Six)[..6],
        [
            word(489),
            word(490),
            word(491),
            word(492),
            word(493),
            word(494)
        ]
    );
}

#[test]
fn on_alternate_setting_zero_one_tick_fades_to_silence_and_the_ring_clears() {
    let mut playback = Playback::new();
    let (mut usb, mut tick, _, gate) = playback.split();
    gate.open();
    usb.select_alternate(AlternateSetting::One);
    render(&mut tick, FramesPerTick::Six);
    let loud = [[
        StreamSample::from_bits(0x0600_0000),
        StreamSample::from_bits(-0x0600_0000),
    ]; 30];
    usb.push_packet(&loud);
    assert_eq!(render(&mut tick, FramesPerTick::Six)[..6], loud[..6]);

    usb.select_alternate(AlternateSetting::Zero);
    let faded = render(&mut tick, FramesPerTick::Six);
    for (frame, remaining_sixths) in faded[..6].iter().zip([6, 5, 4, 3, 2, 1]) {
        let exact = remaining_sixths << 24;
        assert!(
            (exact - 256..=exact).contains(&frame[0].to_bits()),
            "{frame:?}"
        );
        assert_eq!(frame[0].to_bits() & 0xff, 0, "{frame:?}");
        assert_eq!(frame[1].to_bits(), -exact);
    }
    assert_eq!(render(&mut tick, FramesPerTick::Six)[..6], [SILENCE; 6]);
    assert_eq!(usb.push_packet(&loud[..6]), Frames::ZERO);

    usb.select_alternate(AlternateSetting::One);
    render(&mut tick, FramesPerTick::Six);
    usb.push_packet(&[word(500); 24]);
    assert_eq!(render(&mut tick, FramesPerTick::Six)[..6], [word(500); 6]);
}

#[test]
fn a_rate_change_while_streaming_discards_the_ring_and_primes_to_the_new_target() {
    let mut playback = Playback::new();
    let (mut usb, mut tick, counters, gate) = playback.split();
    gate.open();
    let mut host = HostPackets::new();
    running_at_six(&mut usb, &mut tick, &mut host);
    render(&mut tick, FramesPerTick::Six);

    usb.reset_for_new_rate();
    assert_eq!(
        render(&mut tick, FramesPerTick::TwentyFour)[..24],
        [SILENCE; 24]
    );
    usb.push_packet(&[word(900); 64][..64]);
    usb.push_packet(&[word(900); 31][..31]);
    assert_eq!(
        render(&mut tick, FramesPerTick::TwentyFour)[..24],
        [SILENCE; 24]
    );
    usb.push_packet(&[word(900); 1][..1]);
    assert_eq!(
        render(&mut tick, FramesPerTick::TwentyFour)[..24],
        [word(900); 24]
    );
    assert_eq!(counters.take().underruns, 0);
}

#[test]
fn packets_that_arrive_before_the_tick_has_seen_alternate_setting_one_are_refused() {
    let mut playback = Playback::new();
    let (mut usb, mut tick, counters, gate) = playback.split();
    gate.open();
    assert_eq!(usb.push_packet(&[word(1); 6]), Frames::ZERO);
    usb.select_alternate(AlternateSetting::One);
    assert_eq!(usb.push_packet(&[word(1); 6]), Frames::ZERO);
    render(&mut tick, FramesPerTick::Six);
    assert_eq!(usb.push_packet(&[word(2); 6]), Frames::new(6));
    assert_eq!(counters.take().overruns, 0);
}

fn dma(queued: u32) -> TxDmaPosition {
    TxDmaPosition {
        frames_queued: Frames::new(queued),
    }
}

#[test]
fn effective_fill_is_ring_frames_plus_frames_still_queued_in_the_tx_dma_buffer() {
    let mut playback = Playback::new();
    let (mut usb, mut tick, _, gate) = playback.split();
    gate.open();
    usb.select_alternate(AlternateSetting::One);
    render(&mut tick, FramesPerTick::Twelve);
    usb.push_packet(&[word(1); 50]);
    assert_eq!(usb.observe_sof(dma(24)), Frames::new(50 + 12 + 12));
    assert_eq!(usb.observe_sof(dma(12)), Frames::new(50 + 12));
    assert_eq!(usb.observe_sof(dma(1)), Frames::new(50 + 1));
}

#[test]
fn fill_extremes_cover_only_sof_readings_while_running_and_reset_when_read() {
    let mut playback = Playback::new();
    let (mut usb, mut tick, counters, gate) = playback.split();
    gate.open();
    let mut host = HostPackets::new();
    usb.select_alternate(AlternateSetting::One);
    render(&mut tick, FramesPerTick::Six);
    usb.observe_sof(dma(1));
    assert_eq!(counters.take().effective_fill_while_running, None);

    usb.push_packet(&host.packet(30)[..30]);
    render(&mut tick, FramesPerTick::Six);
    usb.observe_sof(dma(12));
    usb.observe_sof(dma(7));
    render(&mut tick, FramesPerTick::Six);
    usb.observe_sof(dma(9));
    assert_eq!(
        counters.take().effective_fill_while_running,
        Some(FillRange {
            lowest: Frames::new(12 + 3 + 6),
            highest: Frames::new(18 + 6 + 6),
        })
    );
    assert_eq!(counters.take().effective_fill_while_running, None);
}

mod common;

use common::XorShift;

const EVERY_FRAME_COUNT: [FramesPerTick; 3] = [
    FramesPerTick::Six,
    FramesPerTick::Twelve,
    FramesPerTick::TwentyFour,
];

#[test]
fn under_random_host_and_tick_traffic_audio_stays_in_order_bounded_and_silent_when_stopped() {
    for seed in 1..60 {
        let mut rng = XorShift::seeded(seed);
        let mut playback = Playback::new();
        let (mut usb, mut tick, counters, gate) = playback.split();
        gate.open();
        let mut host = HostPackets::new();
        let mut frames = FramesPerTick::Six;
        let mut alternate = AlternateSetting::Zero;
        let mut fade_tick_pending = false;
        let mut last_heard = 0;
        let mut short_packets = 0;
        for _ in 0..4000 {
            match rng.below(40) {
                0 => {
                    alternate = AlternateSetting::One;
                    usb.select_alternate(alternate);
                    fade_tick_pending = false;
                }
                1 => {
                    alternate = AlternateSetting::Zero;
                    usb.select_alternate(alternate);
                    fade_tick_pending = true;
                }
                2 => {
                    frames = EVERY_FRAME_COUNT[rng.below(3) as usize];
                    usb.reset_for_new_rate();
                    fade_tick_pending = alternate == AlternateSetting::Zero;
                }
                3..=20 => {
                    let len = rng.below(64) as usize;
                    let stored = usb.push_packet(&host.packet(len)[..len]).count() as usize;
                    if stored > 0 && stored < len {
                        short_packets += 1;
                    }
                    assert!(
                        alternate == AlternateSetting::One || stored == 0,
                        "seed {seed}"
                    );
                }
                _ => {
                    let out = render(&mut tick, frames);
                    let block = &out[..frames.count()];
                    if fade_tick_pending {
                        assert!(
                            block
                                .iter()
                                .all(|f| f[0].to_bits().unsigned_abs() <= (host.next as u32) << 8),
                            "seed {seed}"
                        );
                        fade_tick_pending = false;
                    } else if alternate == AlternateSetting::Zero {
                        assert!(block.iter().all(|f| *f == SILENCE), "seed {seed}");
                    } else {
                        for frame in block.iter().filter(|f| **f != SILENCE) {
                            let heard = frame[0].to_bits() >> 8;
                            assert_eq!(*frame, word(heard), "seed {seed}");
                            assert!(
                                heard > last_heard,
                                "seed {seed}: {heard} after {last_heard}"
                            );
                            last_heard = heard;
                        }
                    }
                }
            }
            assert!(usb.observe_sof(dma(0)) <= Frames::new(512), "seed {seed}");
        }
        assert!(counters.take().overruns >= short_packets, "seed {seed}");
    }
}
