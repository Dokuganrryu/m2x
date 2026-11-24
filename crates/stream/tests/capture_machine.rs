mod common;

use common::XorShift;
use dsp::SampleRate;
use stream::{AlternateSetting, CaptureCommand, CaptureEvent, CaptureState, Frames, PacketSizer};

fn started(rate: SampleRate) -> CaptureState {
    CaptureState::Idle(rate)
        .step(CaptureEvent::AltSettingSelected(AlternateSetting::One))
        .0
}

fn microframe(fill: u32) -> CaptureEvent {
    CaptureEvent::Microframe {
        fill: Frames::new(fill),
    }
}

fn silent_sizes_from_an_empty_ring(mut state: CaptureState) -> [u32; 160] {
    let mut sizes = [0; 160];
    for size in sizes.iter_mut() {
        let (next, command) = state.step(microframe(0));
        let CaptureCommand::SendSilence(frames) = command else {
            panic!("filling sends silence, got {command:?}");
        };
        *size = frames.count();
        state = next;
    }
    sizes
}

fn allowed_sizes(rate: SampleRate) -> &'static [u32] {
    match rate {
        SampleRate::Hz44100 => &[5, 6],
        SampleRate::Hz48000 => &[5, 6, 7],
        SampleRate::Hz88200 => &[11, 12],
        SampleRate::Hz96000 => &[11, 12, 13],
        SampleRate::Hz176400 => &[22, 23],
        SampleRate::Hz192000 => &[23, 24, 25],
    }
}

fn reference_size(rate: SampleRate, microframes_since_reset: u64) -> u32 {
    let frames_by = |microframes: u64| microframes * rate.hz() as u64 / 8000;
    (frames_by(microframes_since_reset + 1) - frames_by(microframes_since_reset)) as u32
}

fn packet_size(command: CaptureCommand) -> Option<Frames> {
    match command {
        CaptureCommand::SendSilence(size) | CaptureCommand::SendRingFrames(size) => Some(size),
        CaptureCommand::SendNothing | CaptureCommand::DiscardRing => None,
    }
}

#[test]
fn at_44_1_khz_packets_carry_five_or_six_frames_and_every_80_microframes_carry_441() {
    let sizes = silent_sizes_from_an_empty_ring(started(SampleRate::Hz44100));
    assert!(sizes.iter().all(|&size| size == 5 || size == 6));
    assert_eq!(sizes[..80].iter().sum::<u32>(), 441);
    assert_eq!(sizes[80..].iter().sum::<u32>(), 441);
}

#[test]
fn at_every_rate_sizes_follow_the_exact_rate_pattern_from_phase_zero() {
    for rate in SampleRate::ALL {
        let sizes = silent_sizes_from_an_empty_ring(started(rate));
        for (index, &size) in sizes.iter().enumerate() {
            assert_eq!(
                size,
                reference_size(rate, index as u64),
                "{rate:?} at {index}"
            );
        }
    }
}

#[test]
fn at_a_fractional_rate_the_correction_never_leaves_int_nav_and_int_nav_plus_one() {
    let mut state = started(SampleRate::Hz44100).step(microframe(24)).0;
    let mut seen = [false; 2];
    for fill in [100, 0, 100, 0, 100, 100, 3, 3, 3, 3, 100, 100, 100, 100] {
        let (next, command) = state.step(microframe(fill));
        let size = packet_size(command).expect("a running stream sends a packet");
        assert!(size == Frames::new(5) || size == Frames::new(6), "{size:?}");
        seen[size.count() as usize - 5] = true;
        state = next;
    }
    assert_eq!(seen, [true, true]);
}

fn state_is_listed(state: CaptureState) {
    match state {
        CaptureState::Idle(_) | CaptureState::Filling(_) | CaptureState::Streaming(_) => {}
    }
}

fn event_is_listed(event: CaptureEvent) {
    match event {
        CaptureEvent::AltSettingSelected(AlternateSetting::Zero | AlternateSetting::One)
        | CaptureEvent::Microframe { .. }
        | CaptureEvent::ResetStream(_) => {}
    }
}

#[test]
fn every_state_and_event_pair_has_the_transition_d18_m5_names() {
    use CaptureCommand::*;
    use CaptureState::*;
    let at_48 = PacketSizer::starting(SampleRate::Hz48000);
    let at_96 = PacketSizer::starting(SampleRate::Hz96000);
    let idle = Idle(SampleRate::Hz48000);
    let filling = Filling(at_48);
    let streaming = Streaming(at_48);
    let alt_zero = CaptureEvent::AltSettingSelected(AlternateSetting::Zero);
    let alt_one = CaptureEvent::AltSettingSelected(AlternateSetting::One);
    let reset_to_96 = CaptureEvent::ResetStream(SampleRate::Hz96000);
    let empty = microframe(0);
    let at_target = microframe(24);
    let one_block_over = microframe(30);
    let past_one_block_over = microframe(31);
    let one_block_under = microframe(18);
    let past_one_block_under = microframe(17);
    let less_than_a_packet = microframe(4);
    let six = Frames::new(6);
    let table = [
        (idle, alt_zero, idle, SendNothing),
        (idle, alt_one, filling, DiscardRing),
        (idle, reset_to_96, Idle(SampleRate::Hz96000), SendNothing),
        (idle, empty, idle, SendNothing),
        (idle, at_target, idle, SendNothing),
        (filling, alt_zero, idle, SendNothing),
        (filling, alt_one, filling, DiscardRing),
        (filling, reset_to_96, Filling(at_96), DiscardRing),
        (filling, empty, filling, SendSilence(six)),
        (filling, one_block_under, filling, SendSilence(six)),
        (filling, at_target, streaming, SendRingFrames(six)),
        (filling, past_one_block_over, streaming, SendRingFrames(six)),
        (streaming, alt_zero, idle, SendNothing),
        (streaming, alt_one, filling, DiscardRing),
        (streaming, reset_to_96, Filling(at_96), DiscardRing),
        (streaming, at_target, streaming, SendRingFrames(six)),
        (streaming, one_block_over, streaming, SendRingFrames(six)),
        (
            streaming,
            past_one_block_over,
            streaming,
            SendRingFrames(Frames::new(7)),
        ),
        (streaming, one_block_under, streaming, SendRingFrames(six)),
        (
            streaming,
            past_one_block_under,
            streaming,
            SendRingFrames(Frames::new(5)),
        ),
        (
            streaming,
            less_than_a_packet,
            filling,
            SendRingFrames(Frames::new(5)),
        ),
    ];
    for (state, event, next, command) in table {
        state_is_listed(state);
        event_is_listed(event);
        assert_eq!(state.step(event), (next, command), "{state:?} on {event:?}");
    }
}

trait SequenceModel {
    fn starting(rate: SampleRate) -> Self;
    fn observe(&mut self, seed: u64, event: CaptureEvent, command: CaptureCommand);
}

fn random_event(rng: &mut XorShift) -> CaptureEvent {
    match rng.below(40) {
        0 => CaptureEvent::AltSettingSelected(AlternateSetting::Zero),
        1 | 2 => CaptureEvent::AltSettingSelected(AlternateSetting::One),
        3 => CaptureEvent::ResetStream(SampleRate::ALL[rng.below(6) as usize]),
        _ => microframe(rng.below(160)),
    }
}

fn holds_over_random_sequences<M: SequenceModel>() {
    for seed in 1..=10_000 {
        let mut rng = XorShift::seeded(seed);
        let rate = SampleRate::ALL[rng.below(6) as usize];
        let mut model = M::starting(rate);
        let mut state = CaptureState::Idle(rate);
        for _ in 0..1_000 {
            let event = random_event(&mut rng);
            let (next, command) = state.step(event);
            model.observe(seed, event, command);
            state = next;
        }
    }
}

struct SizesAllowedByFmt2 {
    rate: SampleRate,
}

impl SequenceModel for SizesAllowedByFmt2 {
    fn starting(rate: SampleRate) -> Self {
        Self { rate }
    }

    fn observe(&mut self, seed: u64, event: CaptureEvent, command: CaptureCommand) {
        if let CaptureEvent::ResetStream(rate) = event {
            self.rate = rate;
        }
        if let Some(size) = packet_size(command) {
            assert!(
                allowed_sizes(self.rate).contains(&size.count()),
                "seed {seed}: {size:?} at {:?}",
                self.rate
            );
            assert!(size.count() * 8 <= 200, "seed {seed}: {size:?}");
        }
    }
}

#[test]
fn under_random_events_every_packet_size_is_allowed_by_fmt_2_and_fits_200_bytes() {
    holds_over_random_sequences::<SizesAllowedByFmt2>();
}

struct SilentAtAlternateZero {
    selected: AlternateSetting,
}

impl SequenceModel for SilentAtAlternateZero {
    fn starting(_: SampleRate) -> Self {
        Self {
            selected: AlternateSetting::Zero,
        }
    }

    fn observe(&mut self, seed: u64, event: CaptureEvent, command: CaptureCommand) {
        if let CaptureEvent::AltSettingSelected(alternate) = event {
            self.selected = alternate;
        }
        if self.selected == AlternateSetting::Zero {
            assert_eq!(packet_size(command), None, "seed {seed}: {event:?}");
        }
    }
}

#[test]
fn under_random_events_no_packet_is_sent_while_alternate_setting_zero_is_selected() {
    holds_over_random_sequences::<SilentAtAlternateZero>();
}

struct PatternRestartsAfterReset {
    rate: SampleRate,
    microframes_since_reset: Option<u64>,
}

impl SequenceModel for PatternRestartsAfterReset {
    fn starting(rate: SampleRate) -> Self {
        Self {
            rate,
            microframes_since_reset: None,
        }
    }

    fn observe(&mut self, seed: u64, event: CaptureEvent, command: CaptureCommand) {
        match (event, command) {
            (CaptureEvent::ResetStream(rate), _) => {
                self.rate = rate;
                self.microframes_since_reset = Some(0);
            }
            (CaptureEvent::AltSettingSelected(AlternateSetting::One), _) => {
                self.microframes_since_reset = Some(0)
            }
            (_, CaptureCommand::SendSilence(size)) => {
                if let Some(count) = self.microframes_since_reset.as_mut() {
                    assert_eq!(
                        size.count(),
                        reference_size(self.rate, *count),
                        "seed {seed}"
                    );
                    *count += 1;
                }
            }
            (_, CaptureCommand::SendRingFrames(_)) => self.microframes_since_reset = None,
            (_, CaptureCommand::SendNothing | CaptureCommand::DiscardRing) => {}
        }
    }
}

#[test]
fn under_random_events_the_size_pattern_restarts_from_phase_zero_after_every_reset() {
    holds_over_random_sequences::<PatternRestartsAfterReset>();
}
