use dsp::SampleRate;
use stream::{
    AlternateSetting, CaptureCommand, CaptureEvent, CaptureState, DMA_BUFFER_FRAMES, DmaCursor,
    DmaFrame, Frames, TickBlocks, XferCount,
};

const MICROFRAMES_PER_SECOND: u64 = 8000;
const ONE_HOUR_OF_MICROFRAMES: u64 = 3600 * MICROFRAMES_PER_SECOND;
const PPM: u64 = 1_000_000;

fn allowed_sizes(rate: SampleRate) -> &'static [u64] {
    match rate {
        SampleRate::Hz44100 => &[5, 6],
        SampleRate::Hz48000 => &[5, 6, 7],
        SampleRate::Hz88200 => &[11, 12],
        SampleRate::Hz96000 => &[11, 12, 13],
        SampleRate::Hz176400 => &[22, 23],
        SampleRate::Hz192000 => &[23, 24, 25],
    }
}

fn block_frames(rate: SampleRate) -> u64 {
    match rate {
        SampleRate::Hz44100 | SampleRate::Hz48000 => 6,
        SampleRate::Hz88200 | SampleRate::Hz96000 => 12,
        SampleRate::Hz176400 | SampleRate::Hz192000 => 24,
    }
}

struct AdcClock {
    frames_per_second_in_ppm: u64,
    remainder: u64,
    frames_produced: u64,
}

const FACTORY_LAST_TRANSFER: u64 = 191;
const XFERCOUNT_SHIFT: u32 = 16;

fn receive_position(frames_produced: u64) -> DmaFrame {
    let frame = frames_produced % DMA_BUFFER_FRAMES as u64;
    XferCount::from_xfercfg(((FACTORY_LAST_TRANSFER - 2 * frame) as u32) << XFERCOUNT_SHIFT)
        .frame_in_progress()
}

struct ReceiveTransport {
    block: u64,
    blocks_moved: u64,
    cursor: DmaCursor,
}

impl ReceiveTransport {
    fn new(rate: SampleRate) -> Self {
        Self {
            block: block_frames(rate),
            blocks_moved: 0,
            cursor: DmaCursor::new(),
        }
    }

    fn tick_for_every_completed_block(&mut self, rate: SampleRate, frames_produced: u64) -> u64 {
        let mut moved = 0;
        while (self.blocks_moved + 1) * self.block <= frames_produced {
            self.blocks_moved += 1;
            let boundary = self.blocks_moved * self.block;
            self.cursor.served(TickBlocks::around(
                receive_position(boundary),
                rate.frames_per_tick(),
            ));
            moved += self.block;
        }
        moved
    }

    fn frames_received_but_not_moved(&self, frames_produced: u64) -> u64 {
        u64::from(
            self.cursor
                .fill_at(receive_position(frames_produced))
                .capture
                .count(),
        )
    }
}

impl AdcClock {
    fn new(rate: SampleRate, offset_ppm: i64) -> Self {
        Self {
            frames_per_second_in_ppm: rate.hz() as u64 * (PPM as i64 + offset_ppm) as u64,
            remainder: 0,
            frames_produced: 0,
        }
    }

    fn run_one_microframe(&mut self) {
        self.remainder += self.frames_per_second_in_ppm;
        let whole = self.remainder / (MICROFRAMES_PER_SECOND * PPM);
        self.remainder -= whole * MICROFRAMES_PER_SECOND * PPM;
        self.frames_produced += whole;
    }
}

struct HourOfCapture {
    frames_in_all_packets: u64,
    lowest_fill_after_priming: u64,
    highest_fill_after_priming: u64,
    short_packets_after_priming: u64,
}

fn one_hour_of_capture(rate: SampleRate, offset_ppm: i64) -> HourOfCapture {
    let mut adc = AdcClock::new(rate, offset_ppm);
    let mut state = CaptureState::Idle(rate)
        .step(CaptureEvent::AltSettingSelected(AlternateSetting::One))
        .0;
    let mut transport = ReceiveTransport::new(rate);
    let mut ring = 0;
    let mut primed = false;
    let mut hour = HourOfCapture {
        frames_in_all_packets: 0,
        lowest_fill_after_priming: u64::MAX,
        highest_fill_after_priming: 0,
        short_packets_after_priming: 0,
    };
    for _ in 0..ONE_HOUR_OF_MICROFRAMES {
        adc.run_one_microframe();
        ring += transport.tick_for_every_completed_block(rate, adc.frames_produced);
        let fill = ring + transport.frames_received_but_not_moved(adc.frames_produced);
        if primed {
            hour.lowest_fill_after_priming = hour.lowest_fill_after_priming.min(fill);
            hour.highest_fill_after_priming = hour.highest_fill_after_priming.max(fill);
        }
        let (next, command) = state.step(CaptureEvent::Microframe {
            fill: Frames::new(fill as u32),
        });
        state = next;
        let size = match command {
            CaptureCommand::SendSilence(size) => size.count() as u64,
            CaptureCommand::SendRingFrames(size) => {
                let size = size.count() as u64;
                if primed && size > ring {
                    hour.short_packets_after_priming += 1;
                }
                ring -= size.min(ring);
                primed = true;
                size
            }
            CaptureCommand::SendNothing | CaptureCommand::DiscardRing => {
                panic!("a started stream sends a packet every microframe, got {command:?}")
            }
        };
        assert!(
            allowed_sizes(rate).contains(&size),
            "{rate:?} at {offset_ppm} ppm sent {size}"
        );
        assert!(size * 8 <= 200, "{rate:?} sent {size} frames");
        hour.frames_in_all_packets += size;
    }
    hour
}

fn assert_fill_stays_within_two_blocks_of_target(rate: SampleRate, offset_ppm: i64) {
    let hour = one_hour_of_capture(rate, offset_ppm);
    let block = block_frames(rate);
    let target = 4 * block;
    assert!(
        hour.lowest_fill_after_priming >= target - 2 * block,
        "{rate:?} at {offset_ppm} ppm fell to {}",
        hour.lowest_fill_after_priming
    );
    assert!(
        hour.highest_fill_after_priming <= target + 2 * block,
        "{rate:?} at {offset_ppm} ppm rose to {}",
        hour.highest_fill_after_priming
    );
    assert_eq!(
        hour.short_packets_after_priming, 0,
        "{rate:?} at {offset_ppm} ppm"
    );
}

#[test]
fn over_one_hour_at_the_nominal_clock_every_rate_averages_exactly_its_sample_rate() {
    for rate in SampleRate::ALL {
        let hour = one_hour_of_capture(rate, 0);
        assert_eq!(
            hour.frames_in_all_packets,
            rate.hz() as u64 * 3600,
            "{rate:?}"
        );
    }
}

#[test]
fn over_one_hour_at_the_nominal_clock_capture_fill_stays_within_two_blocks_of_target() {
    for rate in SampleRate::ALL {
        assert_fill_stays_within_two_blocks_of_target(rate, 0);
    }
}

#[test]
fn over_one_hour_with_the_adc_1000_ppm_fast_capture_fill_stays_within_two_blocks_of_target() {
    for rate in SampleRate::ALL {
        assert_fill_stays_within_two_blocks_of_target(rate, 1000);
    }
}

#[test]
fn over_one_hour_with_the_adc_1000_ppm_slow_capture_fill_stays_within_two_blocks_of_target() {
    for rate in SampleRate::ALL {
        assert_fill_stays_within_two_blocks_of_target(rate, -1000);
    }
}
