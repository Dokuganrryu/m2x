use dsp::{CROSSFADE_TICKS, CrossfadeEvent, CrossfadeState, FadeGain, TickMix};

fn processed_gains_while_fading_to_on() -> Vec<i32> {
    let (mut state, _) = CrossfadeState::Bypass.step(CrossfadeEvent::RequestOn);
    let mut gains = Vec::new();
    for _ in 0..CROSSFADE_TICKS {
        let (next, mix) = state.step(CrossfadeEvent::Ticked);
        let mix = mix.expect("every tick emits a mix");
        assert_eq!(
            mix.bypass_gain().to_q1_30() + mix.processed_gain().to_q1_30(),
            FadeGain::UNITY.to_q1_30()
        );
        gains.push(mix.processed_gain().to_q1_30());
        state = next;
    }
    assert_eq!(state, CrossfadeState::On);
    gains
}

#[test]
fn the_smoothstep_crossfade_gains_sum_to_unity_at_every_one_of_its_32_ticks() {
    let gains = processed_gains_while_fading_to_on();
    assert_eq!(gains.len(), 32);
    let unity = f64::from(1u32 << 30);
    for (step, gain) in gains.iter().enumerate() {
        let x = (step as f64 + 1.0) / 33.0;
        let smoothstep = x * x * (3.0 - 2.0 * x);
        assert!(
            (f64::from(*gain) - smoothstep * unity).abs() <= 0.5,
            "step {step}: {gain} against {}",
            smoothstep * unity
        );
    }
    assert!(gains.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(gains[0] > 0 && gains[31] < 1 << 30);
}

#[test]
fn bypass_and_on_emit_the_whole_bypass_and_the_whole_processed_block() {
    assert_eq!(
        CrossfadeState::Bypass.step(CrossfadeEvent::Ticked),
        (CrossfadeState::Bypass, Some(TickMix::Bypass))
    );
    assert_eq!(
        CrossfadeState::On.step(CrossfadeEvent::Ticked),
        (CrossfadeState::On, Some(TickMix::Processed))
    );
    assert_eq!(TickMix::Bypass.processed_gain(), FadeGain::ZERO);
    assert_eq!(TickMix::Processed.bypass_gain(), FadeGain::ZERO);
}

mod seeded_sequences;

use dsp::{Crossfade, DspMode, DspModeSwitch};

#[test]
fn a_mode_requested_from_thread_mode_starts_a_fade_on_the_next_tick_and_settles_after_32_ticks() {
    let switch = DspModeSwitch::new();
    let mut crossfade = Crossfade::new();
    assert_eq!(crossfade.tick(&switch), TickMix::Bypass);
    switch.request(DspMode::On);
    let fade: Vec<TickMix> = (0..CROSSFADE_TICKS)
        .map(|_| crossfade.tick(&switch))
        .collect();
    assert!(fade.iter().all(|mix| matches!(mix, TickMix::Blend(_))));
    assert_eq!(crossfade.tick(&switch), TickMix::Processed);
    switch.request(DspMode::Bypass);
    switch.request(DspMode::On);
    assert_eq!(crossfade.tick(&switch), TickMix::Processed);
    switch.request(DspMode::Bypass);
    for _ in 0..CROSSFADE_TICKS {
        assert!(matches!(crossfade.tick(&switch), TickMix::Blend(_)));
    }
    assert_eq!(crossfade.tick(&switch), TickMix::Bypass);
}

use dsp::{FadeStep, FramesPerTick, I2sWord, MAX_FRAMES, OutputWords};
use seeded_sequences::{EVENTS_PER_SEQUENCE, SEQUENCES, XorShift};

const UNITY: i32 = 1 << 30;

fn smoothstep_gain(position: u32) -> i32 {
    let x = f64::from(position) / 33.0;
    (x * x * (3.0 - 2.0 * x) * f64::from(UNITY)).round() as i32
}

fn fading_to_on(index: u8) -> CrossfadeState {
    CrossfadeState::FadingToOn(FadeStep::new(index).expect("step inside the fade"))
}

fn fading_to_bypass(index: u8) -> CrossfadeState {
    CrossfadeState::FadingToBypass(FadeStep::new(index).expect("step inside the fade"))
}

fn every_state() -> Vec<CrossfadeState> {
    let states = vec![
        CrossfadeState::Bypass,
        fading_to_on(0),
        fading_to_on(1),
        fading_to_on(2),
        fading_to_on(31),
        CrossfadeState::On,
        fading_to_bypass(0),
        fading_to_bypass(1),
        fading_to_bypass(2),
        fading_to_bypass(31),
    ];
    for state in &states {
        match state {
            CrossfadeState::Bypass
            | CrossfadeState::FadingToOn(_)
            | CrossfadeState::On
            | CrossfadeState::FadingToBypass(_) => {}
        }
    }
    states
}

fn every_event() -> [CrossfadeEvent; 3] {
    let events = [
        CrossfadeEvent::RequestOn,
        CrossfadeEvent::RequestBypass,
        CrossfadeEvent::Ticked,
    ];
    for event in events {
        match event {
            CrossfadeEvent::RequestOn | CrossfadeEvent::RequestBypass | CrossfadeEvent::Ticked => {}
        }
    }
    events
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Emitted {
    Nothing,
    WholeBypass,
    WholeProcessed,
    BlendAtSmoothstepPosition(u32),
}

fn emitted(mix: Option<TickMix>) -> Emitted {
    match mix {
        None => Emitted::Nothing,
        Some(TickMix::Bypass) => Emitted::WholeBypass,
        Some(TickMix::Processed) => Emitted::WholeProcessed,
        Some(blend @ TickMix::Blend(_)) => {
            let gain = blend.processed_gain().to_q1_30();
            let position = (1..=32)
                .find(|&position| smoothstep_gain(position) == gain)
                .unwrap_or_else(|| panic!("gain {gain} is not on the smoothstep curve"));
            Emitted::BlendAtSmoothstepPosition(position)
        }
    }
}

#[test]
fn every_state_and_event_pair_has_the_transition_the_crossfade_design_names() {
    use CrossfadeEvent::{RequestBypass, RequestOn, Ticked};
    use CrossfadeState::{Bypass, On};
    use Emitted::{BlendAtSmoothstepPosition as Blend, Nothing, WholeBypass, WholeProcessed};
    let table = [
        (Bypass, RequestOn, fading_to_on(0), Nothing),
        (Bypass, RequestBypass, Bypass, Nothing),
        (Bypass, Ticked, Bypass, WholeBypass),
        (fading_to_on(0), RequestOn, fading_to_on(0), Nothing),
        (fading_to_on(0), RequestBypass, Bypass, Nothing),
        (fading_to_on(0), Ticked, fading_to_on(1), Blend(1)),
        (fading_to_on(1), RequestOn, fading_to_on(1), Nothing),
        (fading_to_on(1), RequestBypass, Bypass, Nothing),
        (fading_to_on(1), Ticked, fading_to_on(2), Blend(2)),
        (fading_to_on(2), RequestOn, fading_to_on(2), Nothing),
        (
            fading_to_on(2),
            RequestBypass,
            fading_to_bypass(31),
            Nothing,
        ),
        (fading_to_on(2), Ticked, fading_to_on(3), Blend(3)),
        (fading_to_on(31), RequestOn, fading_to_on(31), Nothing),
        (
            fading_to_on(31),
            RequestBypass,
            fading_to_bypass(2),
            Nothing,
        ),
        (fading_to_on(31), Ticked, On, Blend(32)),
        (On, RequestOn, On, Nothing),
        (On, RequestBypass, fading_to_bypass(0), Nothing),
        (On, Ticked, On, WholeProcessed),
        (fading_to_bypass(0), RequestOn, On, Nothing),
        (
            fading_to_bypass(0),
            RequestBypass,
            fading_to_bypass(0),
            Nothing,
        ),
        (fading_to_bypass(0), Ticked, fading_to_bypass(1), Blend(32)),
        (fading_to_bypass(1), RequestOn, On, Nothing),
        (
            fading_to_bypass(1),
            RequestBypass,
            fading_to_bypass(1),
            Nothing,
        ),
        (fading_to_bypass(1), Ticked, fading_to_bypass(2), Blend(31)),
        (fading_to_bypass(2), RequestOn, fading_to_on(31), Nothing),
        (
            fading_to_bypass(2),
            RequestBypass,
            fading_to_bypass(2),
            Nothing,
        ),
        (fading_to_bypass(2), Ticked, fading_to_bypass(3), Blend(30)),
        (fading_to_bypass(31), RequestOn, fading_to_on(2), Nothing),
        (
            fading_to_bypass(31),
            RequestBypass,
            fading_to_bypass(31),
            Nothing,
        ),
        (fading_to_bypass(31), Ticked, Bypass, Blend(1)),
    ];
    let mut covered = 0;
    for state in every_state() {
        for event in every_event() {
            let row = table
                .iter()
                .find(|row| row.0 == state && row.1 == event)
                .unwrap_or_else(|| panic!("no row for {state:?} on {event:?}"));
            let (next, mix) = state.step(event);
            assert_eq!(
                (next, emitted(mix)),
                (row.2, row.3),
                "{state:?} on {event:?}"
            );
            covered += 1;
        }
    }
    assert_eq!(covered, table.len());
}

#[test]
fn a_fade_step_exists_only_inside_the_32_tick_fade() {
    assert_eq!(FadeStep::new(31), Some(FadeStep::LAST));
    assert_eq!(FadeStep::new(32), None);
}

fn random_event(rng: &mut XorShift) -> CrossfadeEvent {
    match rng.below(10) {
        0 => CrossfadeEvent::RequestOn,
        1 => CrossfadeEvent::RequestBypass,
        _ => CrossfadeEvent::Ticked,
    }
}

struct Transition {
    seed: u64,
    starts_a_sequence: bool,
    state: CrossfadeState,
    event: CrossfadeEvent,
    next: CrossfadeState,
    mix: Option<TickMix>,
}

fn for_every_seeded_sequence(mut check: impl FnMut(&Transition, &mut XorShift)) {
    for seed in 0..SEQUENCES {
        let mut rng = XorShift::seeded(seed);
        let mut state = CrossfadeState::Bypass;
        for index in 0..EVENTS_PER_SEQUENCE {
            let event = random_event(&mut rng);
            let (next, mix) = state.step(event);
            let transition = Transition {
                seed,
                starts_a_sequence: index == 0,
                state,
                event,
                next,
                mix,
            };
            check(&transition, &mut rng);
            state = next;
        }
    }
}

#[test]
fn the_bypass_and_processed_gains_sum_to_unity_on_every_tick_of_random_switching() {
    for_every_seeded_sequence(|t, _| {
        if let Some(mix) = t.mix {
            assert_eq!(
                mix.bypass_gain().to_q1_30() + mix.processed_gain().to_q1_30(),
                UNITY,
                "seed {}",
                t.seed
            );
        }
    });
}

#[test]
fn every_tick_in_bypass_gives_the_processed_block_a_gain_of_zero() {
    for_every_seeded_sequence(|t, _| {
        if t.state == CrossfadeState::Bypass && t.event == CrossfadeEvent::Ticked {
            assert_eq!(t.mix, Some(TickMix::Bypass), "seed {}", t.seed);
        }
    });
}

#[test]
fn a_request_during_a_fade_reverses_it_so_no_gain_moves_more_than_one_curve_step_per_tick() {
    let largest_curve_step = (0..=32)
        .map(|position| smoothstep_gain(position + 1) - smoothstep_gain(position))
        .max()
        .expect("curve has steps")
        + 1;
    let mut reversals = 0;
    let mut previous_gain = 0;
    for_every_seeded_sequence(|t, _| {
        if t.starts_a_sequence {
            previous_gain = 0;
        }
        if matches!(
            (t.state, t.next),
            (
                CrossfadeState::FadingToOn(_),
                CrossfadeState::FadingToBypass(_)
            ) | (
                CrossfadeState::FadingToBypass(_),
                CrossfadeState::FadingToOn(_)
            )
        ) {
            reversals += 1;
        }
        if let Some(mix) = t.mix {
            let gain = mix.processed_gain().to_q1_30();
            assert!(
                (gain - previous_gain).abs() <= largest_curve_step,
                "seed {}: {previous_gain} to {gain}",
                t.seed
            );
            previous_gain = gain;
        }
    });
    assert!(reversals > 0);
}

#[test]
fn the_last_request_is_reached_and_held_within_32_ticks() {
    let mut target = CrossfadeState::Bypass;
    let mut ticks_since_request = 0;
    for_every_seeded_sequence(|t, _| {
        if t.starts_a_sequence {
            target = CrossfadeState::Bypass;
            ticks_since_request = 0;
        }
        match t.event {
            CrossfadeEvent::RequestOn if target != CrossfadeState::On => {
                target = CrossfadeState::On;
                ticks_since_request = 0;
            }
            CrossfadeEvent::RequestBypass if target != CrossfadeState::Bypass => {
                target = CrossfadeState::Bypass;
                ticks_since_request = 0;
            }
            CrossfadeEvent::Ticked => ticks_since_request += 1,
            CrossfadeEvent::RequestOn | CrossfadeEvent::RequestBypass => {}
        }
        if ticks_since_request >= CROSSFADE_TICKS {
            assert_eq!(t.next, target, "seed {}", t.seed);
        }
    });
}

fn random_word(rng: &mut XorShift) -> I2sWord {
    I2sWord::from_bits((rng.next_word() >> 32) as u32 as i32)
}

fn random_words(rng: &mut XorShift) -> OutputWords {
    let mut words = [[I2sWord::from_bits(0); 2]; MAX_FRAMES];
    for frame in &mut words {
        *frame = [random_word(rng), random_word(rng)];
    }
    words
}

#[test]
fn every_crossfade_output_word_lies_between_its_bypass_word_and_its_processed_word() {
    let extremes = [i32::MIN, i32::MIN + 1, -1, 0, 1, i32::MAX - 1, i32::MAX];
    let mut rng = XorShift::seeded(11);
    for mix in every_mix_of_a_whole_fade() {
        for frames in [
            FramesPerTick::Six,
            FramesPerTick::Twelve,
            FramesPerTick::TwentyFour,
        ] {
            for round in 0..2_000 {
                let mut bypass = random_words(&mut rng);
                let processed = random_words(&mut rng);
                if round == 0 {
                    for (frame, words) in bypass.iter_mut().enumerate() {
                        *words = [I2sWord::from_bits(extremes[frame % extremes.len()]); 2];
                    }
                }
                let mut output = processed;
                mix.apply(frames, &bypass, &mut output);
                for frame in 0..frames.count() {
                    for side in 0..2 {
                        let [b, p, out] =
                            [bypass, processed, output].map(|block| block[frame][side].to_bits());
                        assert!(
                            b.min(p) <= out && out <= b.max(p),
                            "{mix:?}: {out} outside {b}..{p}"
                        );
                    }
                }
                assert_eq!(output[frames.count()..], processed[frames.count()..]);
            }
        }
    }
}

#[test]
fn random_switching_emits_only_the_gains_of_one_whole_fade() {
    let fade = every_mix_of_a_whole_fade();
    for_every_seeded_sequence(|t, _| {
        if let Some(mix) = t.mix {
            assert!(fade.contains(&mix), "seed {}: {mix:?}", t.seed);
        }
    });
}

fn every_mix_of_a_whole_fade() -> Vec<TickMix> {
    let mut mixes = vec![TickMix::Bypass, TickMix::Processed];
    let (mut state, _) = CrossfadeState::Bypass.step(CrossfadeEvent::RequestOn);
    while state != CrossfadeState::On {
        let (next, mix) = state.step(CrossfadeEvent::Ticked);
        mixes.extend(mix);
        state = next;
    }
    mixes
}

#[test]
fn bypass_is_bit_exact_and_on_passes_the_processed_words_untouched() {
    let mut rng = XorShift::seeded(7);
    let bypass = random_words(&mut rng);
    let processed = random_words(&mut rng);
    for frames in [
        FramesPerTick::Six,
        FramesPerTick::Twelve,
        FramesPerTick::TwentyFour,
    ] {
        let mut output = processed;
        TickMix::Bypass.apply(frames, &bypass, &mut output);
        assert_eq!(output[..frames.count()], bypass[..frames.count()]);
        let mut output = processed;
        TickMix::Processed.apply(frames, &bypass, &mut output);
        assert_eq!(output, processed);
    }
}

#[test]
fn the_crossfade_adds_no_latency_because_each_output_frame_depends_only_on_its_own_input_frames() {
    let full_scale = [I2sWord::from_bits(i32::MAX); 2];
    let silent_frame = [I2sWord::from_bits(0); 2];
    for mix in every_mix_of_a_whole_fade() {
        for impulse_frame in 0..MAX_FRAMES {
            let mut bypass = [silent_frame; MAX_FRAMES];
            let mut output = [silent_frame; MAX_FRAMES];
            bypass[impulse_frame] = full_scale;
            output[impulse_frame] = full_scale;
            mix.apply(FramesPerTick::TwentyFour, &bypass, &mut output);
            for (frame, words) in output.iter().enumerate() {
                let expected = if frame == impulse_frame {
                    full_scale
                } else {
                    silent_frame
                };
                assert_eq!(*words, expected, "{mix:?} impulse at {impulse_frame}");
            }
        }
    }
}
