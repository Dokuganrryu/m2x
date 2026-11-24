use dsp::FramesPerTick;
use stream::{Frames, PlaybackCommand, PlaybackEvent, PlaybackState};

const BLOCK: FramesPerTick = FramesPerTick::Twelve;

fn tick(buffered: u32) -> PlaybackEvent {
    PlaybackEvent::Tick {
        buffered: Frames::new(buffered),
        block: BLOCK,
    }
}

fn at_target() -> PlaybackEvent {
    tick(48)
}

fn one_block_short_of_target() -> PlaybackEvent {
    tick(47)
}

fn less_than_one_block() -> PlaybackEvent {
    tick(11)
}

#[test]
fn the_target_fill_is_four_tick_blocks_at_every_frame_count() {
    assert_eq!(Frames::playback_target(FramesPerTick::Six), Frames::new(24));
    assert_eq!(
        Frames::playback_target(FramesPerTick::Twelve),
        Frames::new(48)
    );
    assert_eq!(
        Frames::playback_target(FramesPerTick::TwentyFour),
        Frames::new(96)
    );
}

#[test]
fn every_state_and_event_pair_has_the_transition_the_ring_policy_names() {
    use PlaybackCommand::*;
    use PlaybackState::*;
    let table = [
        (Idle, PlaybackEvent::Started, Priming, DiscardRing),
        (Idle, PlaybackEvent::Stopped, Idle, DiscardRing),
        (Idle, at_target(), Idle, OutputSilence),
        (Idle, less_than_one_block(), Idle, OutputSilence),
        (Priming, PlaybackEvent::Started, Priming, DiscardRing),
        (Priming, PlaybackEvent::Stopped, Idle, DiscardRing),
        (
            Priming,
            at_target(),
            Running,
            DropOldestThenOutputBlock {
                excess: Frames::ZERO,
            },
        ),
        (
            Priming,
            tick(53),
            Running,
            DropOldestThenOutputBlock {
                excess: Frames::new(5),
            },
        ),
        (Priming, one_block_short_of_target(), Priming, OutputSilence),
        (Priming, less_than_one_block(), Priming, OutputSilence),
        (Running, PlaybackEvent::Started, Priming, DiscardRing),
        (Running, PlaybackEvent::Stopped, Fading, Hold),
        (Running, at_target(), Running, OutputBlock),
        (Running, one_block_short_of_target(), Running, OutputBlock),
        (Running, tick(12), Running, OutputBlock),
        (
            Running,
            less_than_one_block(),
            Priming,
            OutputWhatIsLeftThenSilenceAfterUnderrun,
        ),
        (
            Running,
            tick(0),
            Priming,
            OutputWhatIsLeftThenSilenceAfterUnderrun,
        ),
        (Fading, PlaybackEvent::Started, Priming, DiscardRing),
        (Fading, PlaybackEvent::Stopped, Fading, Hold),
        (Fading, at_target(), Idle, FadeOutOneBlockThenDiscardRing),
        (
            Fading,
            less_than_one_block(),
            Idle,
            FadeOutOneBlockThenDiscardRing,
        ),
    ];
    for (state, event, next, command) in table {
        assert_eq!(state.step(event), (next, command), "{state:?} on {event:?}");
    }
}

mod common;

use common::XorShift;

fn random_event(rng: &mut XorShift) -> PlaybackEvent {
    match rng.below(8) {
        0 => PlaybackEvent::Started,
        1 => PlaybackEvent::Stopped,
        _ => PlaybackEvent::Tick {
            buffered: Frames::new(rng.below(130)),
            block: [
                FramesPerTick::Six,
                FramesPerTick::Twelve,
                FramesPerTick::TwentyFour,
            ][rng.below(3) as usize],
        },
    }
}

fn is_audible(command: PlaybackCommand) -> bool {
    matches!(
        command,
        PlaybackCommand::OutputBlock
            | PlaybackCommand::DropOldestThenOutputBlock { .. }
            | PlaybackCommand::OutputWhatIsLeftThenSilenceAfterUnderrun
            | PlaybackCommand::FadeOutOneBlockThenDiscardRing
    )
}

#[test]
fn audio_starts_only_on_a_tick_that_finds_the_target_fill_after_a_start_or_an_underrun() {
    for seed in 1..200 {
        let mut rng = XorShift::seeded(seed);
        let mut state = PlaybackState::Idle;
        let mut primed = false;
        for _ in 0..2000 {
            let event = random_event(&mut rng);
            let (next, command) = state.step(event);
            if is_audible(command) && !primed {
                let PlaybackEvent::Tick { buffered, block } = event else {
                    panic!("audible command {command:?} on {event:?}");
                };
                assert!(
                    buffered >= Frames::playback_target(block),
                    "seed {seed}: {event:?}"
                );
            }
            primed = match command {
                PlaybackCommand::DiscardRing
                | PlaybackCommand::OutputWhatIsLeftThenSilenceAfterUnderrun => false,
                PlaybackCommand::OutputBlock
                | PlaybackCommand::DropOldestThenOutputBlock { .. } => true,
                _ => primed,
            };
            state = next;
        }
    }
}

#[test]
fn after_a_stop_at_most_one_faded_block_is_audible_until_the_next_start() {
    for seed in 1..200 {
        let mut rng = XorShift::seeded(seed);
        let mut state = PlaybackState::Idle;
        let mut audible_since_stop = None;
        for _ in 0..2000 {
            let event = random_event(&mut rng);
            let (next, command) = state.step(event);
            match event {
                PlaybackEvent::Started => audible_since_stop = None,
                PlaybackEvent::Stopped if audible_since_stop.is_none() => {
                    audible_since_stop = Some(0)
                }
                _ => {}
            }
            if let Some(count) = audible_since_stop.as_mut() {
                if is_audible(command) {
                    assert_eq!(
                        command,
                        PlaybackCommand::FadeOutOneBlockThenDiscardRing,
                        "seed {seed}"
                    );
                    *count += 1;
                }
                assert!(*count <= 1, "seed {seed}");
            }
            state = next;
        }
    }
}
