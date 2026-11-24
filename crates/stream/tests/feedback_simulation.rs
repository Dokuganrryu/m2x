use std::collections::VecDeque;

use dsp::{Frame, MAX_FRAMES, SampleRate, StreamSample};
use stream::{
    AlternateSetting, DMA_BUFFER_FRAMES, DmaCursor, DmaFrame, FeedbackValue, Frames, Playback,
    PlaybackCounterReader, PlaybackEndpoint, PlaybackTick, TickBlocks, TxDmaPosition, XferCount,
};

mod common;

use common::XorShift;

const TIME_UNITS_PER_MICROFRAME: u64 = 1 << 32;
const MICROFRAMES_PER_SECOND: u64 = 8_000;
const ONE_SAMPLE_16_16: i64 = 1 << 16;
const FEEDBACK_POLL_PERIOD_MICROFRAMES: u64 = 8;
const HOST_REPORTS_AVERAGED: usize = 4;
const HOST_SCHEDULING_DELAY_MICROFRAMES: usize = 16;
const LONGEST_SOF_SERVICE_DELAY: u64 = TIME_UNITS_PER_MICROFRAME / 16;
const EARLIEST_PACKET_ARRIVAL: u64 = TIME_UNITS_PER_MICROFRAME / 8;
const PACKET_ARRIVAL_SPREAD: u64 = TIME_UNITS_PER_MICROFRAME * 3 / 4;
const STREAM_START_MICROFRAME: u64 = 16;
const CONVERGENCE_DEADLINE_MICROFRAMES: u64 = 10 * MICROFRAMES_PER_SECOND;
const STALL_MICROFRAMES: u64 = MICROFRAMES_PER_SECOND / 10;
const LARGEST_PACKET_FRAMES: usize = 25;
const SHORT_RUN_MICROFRAMES: u64 = 20 * MICROFRAMES_PER_SECOND;
const ONE_HOUR_MICROFRAMES: u64 = 3_600 * MICROFRAMES_PER_SECOND;
const TEST_FRAME: Frame<StreamSample> = [StreamSample::from_bits(0x1234_5600); 2];

#[derive(Clone, Copy, Debug)]
enum Drift {
    Constant { ppm: i32 },
    Step { before_ppm: i32, after_ppm: i32 },
}

#[derive(Clone, Copy, Debug)]
enum Disturbance {
    None,
    DriftStepHalfway,
    HostStallsHalfway,
}

#[derive(Clone, Copy, Debug)]
struct Scenario {
    rate: SampleRate,
    drift: Drift,
    disturbance: Disturbance,
    microframes: u64,
    seed: u64,
}

impl Scenario {
    fn halfway(&self) -> u64 {
        self.microframes / 2
    }

    fn ppm_at(&self, microframe: u64) -> i32 {
        match self.drift {
            Drift::Constant { ppm } => ppm,
            Drift::Step {
                before_ppm,
                after_ppm,
            } => {
                if microframe < self.halfway() {
                    before_ppm
                } else {
                    after_ppm
                }
            }
        }
    }

    fn host_is_stalled(&self, microframe: u64) -> bool {
        match self.disturbance {
            Disturbance::HostStallsHalfway => {
                (self.halfway()..self.halfway() + STALL_MICROFRAMES).contains(&microframe)
            }
            Disturbance::None | Disturbance::DriftStepHalfway => false,
        }
    }

    fn underrun_is_expected(&self, microframe: u64) -> bool {
        match self.disturbance {
            Disturbance::HostStallsHalfway => {
                (self.halfway()..self.halfway() + 2 * STALL_MICROFRAMES).contains(&microframe)
            }
            Disturbance::None | Disturbance::DriftStepHalfway => false,
        }
    }
}

struct DeviceClock {
    frames_per_tick: u64,
    tick_period: u64,
    blocks_played: u64,
    last_tick_at: u64,
    next_tick_at: u64,
    cursor: DmaCursor,
    block: [Frame<StreamSample>; MAX_FRAMES],
}

const FACTORY_LAST_TRANSFER: u64 = 191;
const DMA_BUFFER_TRANSFERS: u64 = 2 * DMA_BUFFER_FRAMES as u64;
const XFERCOUNT_SHIFT: u32 = 16;

impl DeviceClock {
    fn tick_period(rate: SampleRate, ppm: i32) -> u64 {
        let frames = rate.frames_per_tick().count() as f64;
        let device_rate = f64::from(rate.hz()) * (1.0 + f64::from(ppm) * 1e-6);
        (TIME_UNITS_PER_MICROFRAME as f64 * MICROFRAMES_PER_SECOND as f64 * frames / device_rate)
            .round() as u64
    }

    fn new(rate: SampleRate, ppm: i32, first_tick_at: u64) -> Self {
        Self {
            frames_per_tick: rate.frames_per_tick().count() as u64,
            tick_period: Self::tick_period(rate, ppm),
            blocks_played: 0,
            last_tick_at: 0,
            next_tick_at: first_tick_at,
            cursor: DmaCursor::new(),
            block: [TEST_FRAME; MAX_FRAMES],
        }
    }

    fn run_ticks_until(&mut self, now: u64, rate: SampleRate, tick: &mut PlaybackTick) {
        while self.next_tick_at <= now {
            self.blocks_played += 1;
            self.last_tick_at = self.next_tick_at;
            self.next_tick_at += self.tick_period;
            let frames = rate.frames_per_tick();
            let blocks = TickBlocks::around(self.dma_position(self.last_tick_at), frames);
            tick.render(frames, &mut self.block);
            self.cursor.served(blocks);
        }
    }

    fn dma_position(&self, now: u64) -> DmaFrame {
        let transfers_per_tick = 2 * self.frames_per_tick;
        let into_block = u128::from(now - self.last_tick_at) * u128::from(transfers_per_tick)
            / u128::from(self.tick_period);
        let moved = (self.blocks_played * transfers_per_tick
            + (into_block as u64).min(transfers_per_tick - 1))
            % DMA_BUFFER_TRANSFERS;
        XferCount::from_xfercfg(((FACTORY_LAST_TRANSFER - moved) as u32) << XFERCOUNT_SHIFT)
            .frame_in_progress()
    }

    fn tx_position(&self, now: u64) -> TxDmaPosition {
        self.cursor.fill_at(self.dma_position(now)).playback
    }
}

struct Host {
    nominal: i64,
    reports: [i64; HOST_REPORTS_AVERAGED],
    next_report: usize,
    scheduled: VecDeque<i64>,
    phase: i64,
}

impl Host {
    fn new(rate: SampleRate) -> Self {
        let nominal = i64::from(FeedbackValue::nominal(rate).samples_per_microframe_16_16());
        Self {
            nominal,
            reports: [nominal; HOST_REPORTS_AVERAGED],
            next_report: 0,
            scheduled: std::iter::repeat_n(nominal, HOST_SCHEDULING_DELAY_MICROFRAMES).collect(),
            phase: 0,
        }
    }

    fn receive(&mut self, value: FeedbackValue) {
        self.reports[self.next_report] = i64::from(value.samples_per_microframe_16_16());
        self.next_report = (self.next_report + 1) % HOST_REPORTS_AVERAGED;
    }

    fn next_packet_frames(&mut self) -> usize {
        let averaged = self.reports.iter().sum::<i64>() / HOST_REPORTS_AVERAGED as i64;
        self.scheduled.push_back(averaged);
        let rate = self.scheduled.pop_front().unwrap_or(self.nominal);
        self.phase += rate;
        let frames = self.phase >> 16;
        self.phase &= ONE_SAMPLE_16_16 - 1;
        frames as usize
    }

    fn is_within_one_slot_of_nominal(&self, frames: usize) -> bool {
        (frames as i64 * ONE_SAMPLE_16_16 - self.nominal).abs() <= ONE_SAMPLE_16_16
    }
}

#[derive(Debug, Default)]
struct Outcome {
    playing_since: Option<u64>,
    converged_since: Option<u64>,
    worst_error_after_convergence_frames: i64,
    slowest_convergence_microframes: u64,
    underruns: u32,
    lowest_feedback: u32,
    highest_feedback: u32,
    feedback_held_through_stall: Option<u32>,
}

fn random_share_of(rng: &mut XorShift, span: u64) -> u64 {
    const SHARE_BITS: u32 = 28;
    (u64::from(rng.below(1 << SHARE_BITS)) * span) >> SHARE_BITS
}

fn simulate(scenario: Scenario) -> Outcome {
    let rate = scenario.rate;
    let block = i64::from(Frames::in_one_tick(rate.frames_per_tick()).count());
    let set_point = i64::from(Frames::feedback_set_point(rate.frames_per_tick()).count());
    let lowest = FeedbackValue::lowest(rate);
    let highest = FeedbackValue::highest(rate);
    let label = format!("{scenario:?}");

    let mut playback = Playback::new();
    let (usb, mut tick, counters, gate): (_, _, PlaybackCounterReader, _) = playback.split();
    gate.open();
    let mut endpoint = PlaybackEndpoint::new(usb, rate);
    let mut rng = XorShift::seeded(scenario.seed);
    let mut device = DeviceClock::new(rate, scenario.ppm_at(0), u64::from(rng.below(1 << 30)) << 2);
    let mut host = Host::new(rate);
    let packet = [TEST_FRAME; LARGEST_PACKET_FRAMES];
    let mut outcome = Outcome {
        lowest_feedback: u32::MAX,
        ..Outcome::default()
    };

    for microframe in 0..scenario.microframes {
        let start = microframe * TIME_UNITS_PER_MICROFRAME;
        if microframe == scenario.halfway() {
            device.tick_period = DeviceClock::tick_period(rate, scenario.ppm_at(microframe));
        }
        if microframe == STREAM_START_MICROFRAME {
            endpoint.select_alternate(AlternateSetting::One);
        }

        let sof_at = start + random_share_of(&mut rng, LONGEST_SOF_SERVICE_DELAY);
        device.run_ticks_until(sof_at, rate, &mut tick);
        let latest = endpoint.on_sof(device.tx_position(sof_at));
        let raw = latest.samples_per_microframe_16_16();
        assert!(
            (lowest.samples_per_microframe_16_16()..=highest.samples_per_microframe_16_16())
                .contains(&raw),
            "{label}: feedback {raw:#x} left the clamp at microframe {microframe}"
        );
        outcome.lowest_feedback = outcome.lowest_feedback.min(raw);
        outcome.highest_feedback = outcome.highest_feedback.max(raw);

        let read = counters.take();
        assert_eq!(
            read.overruns, 0,
            "{label}: overrun at microframe {microframe}"
        );
        if read.underruns > 0 {
            assert!(
                scenario.underrun_is_expected(microframe),
                "{label}: underrun at microframe {microframe}"
            );
            outcome.underruns += read.underruns;
        }
        match read.effective_fill_while_running {
            Some(range) => {
                let fill = i64::from(range.lowest.count());
                let error = fill - set_point;
                let since = *outcome.playing_since.get_or_insert(microframe);
                match outcome.converged_since {
                    None if error.abs() <= block => {
                        let took = microframe - since;
                        assert!(
                            took <= CONVERGENCE_DEADLINE_MICROFRAMES,
                            "{label}: took {took} microframes to reach the set point"
                        );
                        outcome.slowest_convergence_microframes =
                            outcome.slowest_convergence_microframes.max(took);
                        outcome.converged_since = Some(microframe);
                    }
                    None => assert!(
                        microframe - since <= CONVERGENCE_DEADLINE_MICROFRAMES,
                        "{label}: fill {fill} still off the set point {set_point} after 10 s"
                    ),
                    Some(_) if scenario.underrun_is_expected(microframe) => {}
                    Some(_) => {
                        assert!(
                            error.abs() <= 2 * block,
                            "{label}: fill {fill} left set point {set_point} plus or minus two blocks at microframe {microframe}"
                        );
                        outcome.worst_error_after_convergence_frames = outcome
                            .worst_error_after_convergence_frames
                            .max(error.abs());
                    }
                }
            }
            None => {
                if outcome.playing_since.is_some() {
                    assert!(
                        scenario.underrun_is_expected(microframe),
                        "{label}: playback stopped at microframe {microframe}"
                    );
                    let held = *outcome.feedback_held_through_stall.get_or_insert(raw);
                    assert_eq!(
                        held, raw,
                        "{label}: feedback moved while the stream re-primed after an underrun"
                    );
                }
                outcome.playing_since = None;
                outcome.converged_since = None;
            }
        }

        let frames = host.next_packet_frames();
        assert!(
            host.is_within_one_slot_of_nominal(frames),
            "{label}: host packet of {frames} frames at microframe {microframe}"
        );
        if microframe > STREAM_START_MICROFRAME && !scenario.host_is_stalled(microframe) {
            let arrives_at =
                start + EARLIEST_PACKET_ARRIVAL + random_share_of(&mut rng, PACKET_ARRIVAL_SPREAD);
            device.run_ticks_until(arrives_at, rate, &mut tick);
            endpoint.push_packet(&packet[..frames]);
        }
        if microframe % FEEDBACK_POLL_PERIOD_MICROFRAMES == FEEDBACK_POLL_PERIOD_MICROFRAMES - 1 {
            host.receive(latest);
        }
    }
    assert!(
        outcome.converged_since.is_some(),
        "{label}: never converged"
    );
    if let Disturbance::HostStallsHalfway = scenario.disturbance {
        assert!(
            outcome.underruns > 0,
            "{label}: the stall caused no underrun"
        );
    }
    println!(
        "{label}: converged within {} microframes, worst error after convergence {} frames of a {block}-frame block, feedback {:#x} to {:#x}",
        outcome.slowest_convergence_microframes,
        outcome.worst_error_after_convergence_frames,
        outcome.lowest_feedback,
        outcome.highest_feedback
    );
    outcome
}

fn scenarios(rate: SampleRate, microframes: u64, seed: u64) -> Vec<Scenario> {
    let mut rng = XorShift::seeded(seed);
    let random_ppm = rng.below(2_001) as i32 - 1_000;
    let mut list: Vec<Scenario> = [-1_000, -500, 0, 500, 1_000, random_ppm]
        .into_iter()
        .map(|ppm| Scenario {
            rate,
            drift: Drift::Constant { ppm },
            disturbance: Disturbance::None,
            microframes,
            seed: seed ^ ppm as u64,
        })
        .collect();
    for (before_ppm, after_ppm) in [(-1_000, 1_000), (1_000, -1_000)] {
        list.push(Scenario {
            rate,
            drift: Drift::Step {
                before_ppm,
                after_ppm,
            },
            disturbance: Disturbance::DriftStepHalfway,
            microframes,
            seed: seed.wrapping_add(before_ppm as u64),
        });
    }
    for ppm in [-1_000, 1_000] {
        list.push(Scenario {
            rate,
            drift: Drift::Constant { ppm },
            disturbance: Disturbance::HostStallsHalfway,
            microframes,
            seed: seed.wrapping_mul(3) ^ ppm as u64,
        });
    }
    list
}

fn run_all(rate: SampleRate, microframes: u64) {
    for scenario in scenarios(rate, microframes, 0x5eed_0000 + u64::from(rate.hz())) {
        simulate(scenario);
    }
}

#[test]
fn a_stall_holds_the_feedback_at_the_learned_offset_instead_of_nominal() {
    let outcome = simulate(Scenario {
        rate: SampleRate::Hz48000,
        drift: Drift::Constant { ppm: 800 },
        disturbance: Disturbance::HostStallsHalfway,
        microframes: SHORT_RUN_MICROFRAMES,
        seed: 7,
    });
    let held = outcome
        .feedback_held_through_stall
        .expect("the stall re-primed the stream");
    let nominal = FeedbackValue::nominal(SampleRate::Hz48000).samples_per_microframe_16_16();
    assert!(
        held > nominal + 250,
        "held {held:#x} against nominal {nominal:#x}"
    );
}

#[test]
fn twenty_simulated_seconds_at_44_1_khz_keep_the_fill_bounded() {
    run_all(SampleRate::Hz44100, SHORT_RUN_MICROFRAMES);
}

#[test]
fn twenty_simulated_seconds_at_48_khz_keep_the_fill_bounded() {
    run_all(SampleRate::Hz48000, SHORT_RUN_MICROFRAMES);
}

#[test]
fn twenty_simulated_seconds_at_88_2_khz_keep_the_fill_bounded() {
    run_all(SampleRate::Hz88200, SHORT_RUN_MICROFRAMES);
}

#[test]
fn twenty_simulated_seconds_at_96_khz_keep_the_fill_bounded() {
    run_all(SampleRate::Hz96000, SHORT_RUN_MICROFRAMES);
}

#[test]
fn twenty_simulated_seconds_at_176_4_khz_keep_the_fill_bounded() {
    run_all(SampleRate::Hz176400, SHORT_RUN_MICROFRAMES);
}

#[test]
fn twenty_simulated_seconds_at_192_khz_keep_the_fill_bounded() {
    run_all(SampleRate::Hz192000, SHORT_RUN_MICROFRAMES);
}

#[test]
#[ignore = "one simulated hour per scenario; run with --release -- --ignored"]
fn one_simulated_hour_at_44_1_khz_keeps_the_fill_bounded() {
    run_all(SampleRate::Hz44100, ONE_HOUR_MICROFRAMES);
}

#[test]
#[ignore = "one simulated hour per scenario; run with --release -- --ignored"]
fn one_simulated_hour_at_48_khz_keeps_the_fill_bounded() {
    run_all(SampleRate::Hz48000, ONE_HOUR_MICROFRAMES);
}

#[test]
#[ignore = "one simulated hour per scenario; run with --release -- --ignored"]
fn one_simulated_hour_at_88_2_khz_keeps_the_fill_bounded() {
    run_all(SampleRate::Hz88200, ONE_HOUR_MICROFRAMES);
}

#[test]
#[ignore = "one simulated hour per scenario; run with --release -- --ignored"]
fn one_simulated_hour_at_96_khz_keeps_the_fill_bounded() {
    run_all(SampleRate::Hz96000, ONE_HOUR_MICROFRAMES);
}

#[test]
#[ignore = "one simulated hour per scenario; run with --release -- --ignored"]
fn one_simulated_hour_at_176_4_khz_keeps_the_fill_bounded() {
    run_all(SampleRate::Hz176400, ONE_HOUR_MICROFRAMES);
}

#[test]
#[ignore = "one simulated hour per scenario; run with --release -- --ignored"]
fn one_simulated_hour_at_192_khz_keeps_the_fill_bounded() {
    run_all(SampleRate::Hz192000, ONE_HOUR_MICROFRAMES);
}
