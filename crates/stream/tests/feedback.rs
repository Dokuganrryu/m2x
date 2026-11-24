use dsp::{FramesPerTick, SampleRate};
use stream::{FeedbackEvent, FeedbackState, FeedbackValue, Frames, Integrator};

mod common;

use common::XorShift;

const SEQUENCES: u64 = 10_000;
const EVENTS_PER_SEQUENCE: usize = 1_000;

const RATE: SampleRate = SampleRate::Hz48000;
const OTHER_RATE: SampleRate = SampleRate::Hz96000;
const NOMINAL_48K: u32 = 0x0006_0000;
const NOMINAL_96K: u32 = 0x000C_0000;

fn ff(value: FeedbackValue) -> u32 {
    value.samples_per_microframe_16_16()
}

fn set_point(rate: SampleRate) -> Frames {
    Frames::feedback_set_point(rate.frames_per_tick())
}

fn sof_at(fill: u32) -> FeedbackEvent {
    FeedbackEvent::Sof {
        effective_fill: Frames::new(fill),
    }
}

fn sof_at_set_point(rate: SampleRate) -> FeedbackEvent {
    FeedbackEvent::Sof {
        effective_fill: set_point(rate),
    }
}

fn sof_frames_low(rate: SampleRate, frames: u32) -> FeedbackEvent {
    sof_at(set_point(rate).count() - frames)
}

fn tracking_after_sofs_one_frame_low(count: usize) -> FeedbackState {
    let mut state = FeedbackState::Tracking {
        rate: RATE,
        integrator: Integrator::ZERO,
    };
    for _ in 0..count {
        state = state.step(sof_frames_low(RATE, 1)).0;
    }
    state
}

fn integrator_of(state: FeedbackState) -> Integrator {
    match state {
        FeedbackState::Tracking { integrator, .. }
        | FeedbackState::Recovering { integrator, .. } => integrator,
        FeedbackState::Stopped { .. } | FeedbackState::HoldingNominal { .. } => {
            panic!("{state:?} carries no integrator")
        }
    }
}

#[test]
fn the_controller_set_point_is_the_four_block_ring_target_plus_the_two_blocks_the_tick_writes_ahead_of_the_dma()
 {
    assert_eq!(
        Frames::feedback_set_point(FramesPerTick::Six),
        Frames::new(36)
    );
    assert_eq!(
        Frames::feedback_set_point(FramesPerTick::Twelve),
        Frames::new(72)
    );
    assert_eq!(
        Frames::feedback_set_point(FramesPerTick::TwentyFour),
        Frames::new(144)
    );
}

#[test]
fn each_rate_ticks_at_8000_hz_in_the_48_khz_family_and_7350_hz_in_the_44_1_khz_family() {
    for rate in SampleRate::ALL {
        let ticks_per_second = rate.hz() / rate.frames_per_tick().count() as u32;
        let expected = if rate.hz() % 48_000 == 0 {
            8_000
        } else {
            7_350
        };
        assert_eq!(ticks_per_second, expected, "{rate:?}");
    }
}

#[test]
fn the_nominal_feedback_value_of_each_rate_is_the_research_table_entry_sent_least_significant_byte_first()
 {
    let table = [
        (SampleRate::Hz44100, 0x0005_8333, [0x33, 0x83, 0x05, 0x00]),
        (SampleRate::Hz48000, 0x0006_0000, [0x00, 0x00, 0x06, 0x00]),
        (SampleRate::Hz88200, 0x000B_0666, [0x66, 0x06, 0x0B, 0x00]),
        (SampleRate::Hz96000, 0x000C_0000, [0x00, 0x00, 0x0C, 0x00]),
        (SampleRate::Hz176400, 0x0016_0CCD, [0xCD, 0x0C, 0x16, 0x00]),
        (SampleRate::Hz192000, 0x0018_0000, [0x00, 0x00, 0x18, 0x00]),
    ];
    for (rate, samples_per_microframe, wire) in table {
        let nominal = FeedbackValue::nominal(rate);
        assert_eq!(ff(nominal), samples_per_microframe, "{rate:?}");
        assert_eq!(nominal.to_wire(), wire, "{rate:?}");
    }
}

#[test]
fn the_clamp_is_at_least_one_thousand_ppm_each_side_of_the_exact_rate_and_never_a_whole_lsb_more() {
    for rate in SampleRate::ALL {
        let exact = f64::from(rate.hz()) * 65_536.0 / 8_000.0;
        let lowest = ff(FeedbackValue::lowest(rate));
        let highest = ff(FeedbackValue::highest(rate));
        assert!(f64::from(highest) >= exact * 1.001, "{rate:?}");
        assert!(f64::from(highest) < exact * 1.001 + 1.0, "{rate:?}");
        assert!(f64::from(lowest) <= exact * 0.999, "{rate:?}");
        assert!(f64::from(lowest) > exact * 0.999 - 1.0, "{rate:?}");
    }
    assert_eq!(ff(FeedbackValue::highest(RATE)), NOMINAL_48K + 394);
    assert_eq!(ff(FeedbackValue::lowest(RATE)), NOMINAL_48K - 394);
}

#[test]
fn the_gains_are_pinned_at_128_lsb_per_frame_now_and_one_lsb_per_frame_every_16_microframes() {
    let tracking = FeedbackState::Tracking {
        rate: RATE,
        integrator: Integrator::ZERO,
    };
    let (_, first) = tracking.step(sof_frames_low(RATE, 1));
    assert_eq!(ff(first), NOMINAL_48K + 128);
    let (_, after_32) = (0..32).fold((tracking, first), |(state, _), _| {
        state.step(sof_frames_low(RATE, 1))
    });
    assert_eq!(ff(after_32), NOMINAL_48K + 128 + 2);
    let (_, high_fill) = tracking.step(sof_at(set_point(RATE).count() + 2));
    assert_eq!(ff(high_fill), NOMINAL_48K - 256);
}

#[test]
fn a_large_fill_error_pins_the_value_to_the_clamp_and_the_integrator_stops_at_the_clamp() {
    let mut state = FeedbackState::Tracking {
        rate: RATE,
        integrator: Integrator::ZERO,
    };
    let mut value = FeedbackValue::nominal(RATE);
    for _ in 0..1_000_000 {
        (state, value) = state.step(sof_at(0));
    }
    assert_eq!(value, FeedbackValue::highest(RATE));
    let (_, at_set_point) = state.step(sof_at_set_point(RATE));
    assert_eq!(at_set_point, FeedbackValue::highest(RATE));
    let (_, one_frame_high) = state.step(sof_at(set_point(RATE).count() + 1));
    assert_eq!(ff(one_frame_high), NOMINAL_48K + 394 - 128);
    for _ in 0..1_000_000 {
        (state, value) = state.step(sof_at(4_000_000_000));
    }
    assert_eq!(value, FeedbackValue::lowest(RATE));
}

fn every_state() -> Vec<FeedbackState> {
    let states = vec![
        FeedbackState::Stopped { rate: RATE },
        FeedbackState::HoldingNominal { rate: RATE },
        tracking_after_sofs_one_frame_low(0),
        tracking_after_sofs_one_frame_low(16),
        FeedbackState::Recovering {
            rate: RATE,
            integrator: integrator_of(tracking_after_sofs_one_frame_low(16)),
        },
    ];
    for state in &states {
        match state {
            FeedbackState::Stopped { .. }
            | FeedbackState::HoldingNominal { .. }
            | FeedbackState::Tracking { .. }
            | FeedbackState::Recovering { .. } => {}
        }
    }
    states
}

fn every_event() -> Vec<FeedbackEvent> {
    let events = vec![
        FeedbackEvent::Started,
        FeedbackEvent::PrimingDone,
        FeedbackEvent::Underran,
        FeedbackEvent::Stopped,
        FeedbackEvent::ResetStream { rate: OTHER_RATE },
        sof_at_set_point(RATE),
        sof_frames_low(RATE, 1),
    ];
    for event in &events {
        match event {
            FeedbackEvent::Started
            | FeedbackEvent::PrimingDone
            | FeedbackEvent::Underran
            | FeedbackEvent::Stopped
            | FeedbackEvent::ResetStream { .. }
            | FeedbackEvent::Sof { .. } => {}
        }
    }
    events
}

#[test]
fn every_state_and_event_pair_has_the_transition_the_controller_policy_names() {
    use FeedbackState::*;
    let stopped = Stopped { rate: RATE };
    let holding = HoldingNominal { rate: RATE };
    let fresh = tracking_after_sofs_one_frame_low(0);
    let wound = tracking_after_sofs_one_frame_low(16);
    let wound_once_more = tracking_after_sofs_one_frame_low(17);
    let fresh_once = tracking_after_sofs_one_frame_low(1);
    let held = Recovering {
        rate: RATE,
        integrator: integrator_of(wound),
    };
    let nominal = NOMINAL_48K;
    let with_integral = NOMINAL_48K + 1;
    let started = FeedbackEvent::Started;
    let primed = FeedbackEvent::PrimingDone;
    let underran = FeedbackEvent::Underran;
    let stop = FeedbackEvent::Stopped;
    let reset = FeedbackEvent::ResetStream { rate: OTHER_RATE };
    let level = sof_at_set_point(RATE);
    let low = sof_frames_low(RATE, 1);
    let table = [
        (stopped, started, holding, nominal),
        (stopped, primed, stopped, nominal),
        (stopped, underran, stopped, nominal),
        (stopped, stop, stopped, nominal),
        (stopped, reset, Stopped { rate: OTHER_RATE }, NOMINAL_96K),
        (stopped, level, stopped, nominal),
        (stopped, low, stopped, nominal),
        (holding, started, holding, nominal),
        (holding, primed, fresh, nominal),
        (holding, underran, holding, nominal),
        (holding, stop, stopped, nominal),
        (
            holding,
            reset,
            HoldingNominal { rate: OTHER_RATE },
            NOMINAL_96K,
        ),
        (holding, level, holding, nominal),
        (holding, low, holding, nominal),
        (fresh, started, holding, nominal),
        (fresh, primed, fresh, nominal),
        (
            fresh,
            underran,
            Recovering {
                rate: RATE,
                integrator: Integrator::ZERO,
            },
            nominal,
        ),
        (fresh, stop, stopped, nominal),
        (
            fresh,
            reset,
            HoldingNominal { rate: OTHER_RATE },
            NOMINAL_96K,
        ),
        (fresh, level, fresh, nominal),
        (fresh, low, fresh_once, nominal + 128),
        (wound, started, holding, nominal),
        (wound, primed, wound, with_integral),
        (wound, underran, held, with_integral),
        (wound, stop, stopped, nominal),
        (
            wound,
            reset,
            HoldingNominal { rate: OTHER_RATE },
            NOMINAL_96K,
        ),
        (wound, level, wound, with_integral),
        (wound, low, wound_once_more, with_integral + 128),
        (held, started, holding, nominal),
        (held, primed, wound, with_integral),
        (held, underran, held, with_integral),
        (held, stop, stopped, nominal),
        (
            held,
            reset,
            HoldingNominal { rate: OTHER_RATE },
            NOMINAL_96K,
        ),
        (held, level, held, with_integral),
        (held, low, held, with_integral),
    ];
    let states = every_state();
    let events = every_event();
    assert_eq!(table.len(), states.len() * events.len());
    for state in &states {
        for event in &events {
            assert!(
                table.iter().any(|row| row.0 == *state && row.1 == *event),
                "no row for {state:?} on {event:?}"
            );
        }
    }
    for (state, event, next, value) in table {
        let (actual_next, actual_value) = state.step(event);
        assert_eq!(
            (actual_next, ff(actual_value)),
            (next, value),
            "{state:?} on {event:?}"
        );
    }
}

fn random_rate(rng: &mut XorShift) -> SampleRate {
    SampleRate::ALL[rng.below(6) as usize]
}

fn random_event(rng: &mut XorShift) -> FeedbackEvent {
    match rng.below(64) {
        0 => FeedbackEvent::Started,
        1 => FeedbackEvent::PrimingDone,
        2 => FeedbackEvent::Underran,
        3 => FeedbackEvent::Stopped,
        4 => FeedbackEvent::ResetStream {
            rate: random_rate(rng),
        },
        5..=9 => FeedbackEvent::Sof {
            effective_fill: Frames::new(rng.below(u32::MAX)),
        },
        _ => FeedbackEvent::Sof {
            effective_fill: Frames::new(rng.below(300)),
        },
    }
}

fn rate_of(state: FeedbackState) -> SampleRate {
    match state {
        FeedbackState::Stopped { rate }
        | FeedbackState::HoldingNominal { rate }
        | FeedbackState::Tracking { rate, .. }
        | FeedbackState::Recovering { rate, .. } => rate,
    }
}

fn value_at_set_point(state: FeedbackState) -> FeedbackValue {
    state.step(sof_at_set_point(rate_of(state))).1
}

struct Transition {
    seed: u64,
    state: FeedbackState,
    event: FeedbackEvent,
    next: FeedbackState,
    value: FeedbackValue,
}

fn for_every_seeded_sequence(mut check: impl FnMut(&Transition)) {
    for seed in 0..SEQUENCES {
        let mut rng = XorShift::seeded(seed);
        let mut state = FeedbackState::Stopped {
            rate: random_rate(&mut rng),
        };
        for _ in 0..EVENTS_PER_SEQUENCE {
            let event = random_event(&mut rng);
            let (next, value) = state.step(event);
            check(&Transition {
                seed,
                state,
                event,
                next,
                value,
            });
            state = next;
        }
    }
}

#[test]
fn every_value_stays_inside_the_clamp_of_the_current_rate_with_the_top_four_bits_zero() {
    for_every_seeded_sequence(|t| {
        let rate = rate_of(t.next);
        let raw = ff(t.value);
        assert!(
            (ff(FeedbackValue::lowest(rate))..=ff(FeedbackValue::highest(rate))).contains(&raw),
            "seed {}: {:?} on {:?} gave {raw:#x}",
            t.seed,
            t.state,
            t.event
        );
        assert_eq!(raw >> 28, 0, "seed {}", t.seed);
        assert_eq!(t.value.to_wire()[3] >> 4, 0, "seed {}", t.seed);
    });
}

#[test]
fn holding_nominal_and_stopped_send_exactly_the_nominal_value() {
    for_every_seeded_sequence(|t| {
        if matches!(
            t.next,
            FeedbackState::HoldingNominal { .. } | FeedbackState::Stopped { .. }
        ) {
            assert_eq!(
                t.value,
                FeedbackValue::nominal(rate_of(t.next)),
                "seed {}",
                t.seed
            );
        }
    });
}

#[test]
fn an_underrun_and_the_recovery_after_it_keep_the_integrator_unchanged() {
    for_every_seeded_sequence(|t| {
        let keeps = matches!(
            (t.state, t.event),
            (FeedbackState::Tracking { .. }, FeedbackEvent::Underran)
                | (FeedbackState::Recovering { .. }, _)
        ) && !matches!(
            t.event,
            FeedbackEvent::Started | FeedbackEvent::Stopped | FeedbackEvent::ResetStream { .. }
        );
        if keeps {
            assert_eq!(
                value_at_set_point(t.next),
                value_at_set_point(t.state),
                "seed {}: {:?} on {:?}",
                t.seed,
                t.state,
                t.event
            );
        }
    });
}

#[test]
fn a_stop_or_a_rate_reset_discards_the_integrator_so_the_next_stream_starts_at_nominal() {
    for_every_seeded_sequence(|t| {
        if matches!(
            t.event,
            FeedbackEvent::Stopped | FeedbackEvent::ResetStream { .. }
        ) {
            let restarted = [FeedbackEvent::Started, FeedbackEvent::PrimingDone]
                .into_iter()
                .fold(t.next, |state, event| state.step(event).0);
            assert_eq!(
                value_at_set_point(restarted),
                FeedbackValue::nominal(rate_of(t.next)),
                "seed {}: {:?} on {:?}",
                t.seed,
                t.state,
                t.event
            );
        }
    });
}

#[test]
fn a_rate_reset_moves_every_state_to_the_new_rate() {
    for_every_seeded_sequence(|t| {
        if let FeedbackEvent::ResetStream { rate } = t.event {
            assert_eq!(rate_of(t.next), rate, "seed {}", t.seed);
        }
    });
}
