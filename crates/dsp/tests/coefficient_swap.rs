mod seeded_sequences;

use dsp::{CoefficientSlot, Generation, NextFill, SwapCommand, SwapEvent, SwapState};
use seeded_sequences::{EVENTS_PER_SEQUENCE, SEQUENCES, XorShift};

const G0: Generation = Generation::FIRST;
const G1: Generation = Generation::FIRST.next();
const G2: Generation = G1.next();

fn every_state() -> [SwapState; 4] {
    let states = [
        SwapState::Settled(G0),
        SwapState::Filling(G1),
        SwapState::AwaitingTick {
            published: G1,
            next_fill: NextFill::NotNeeded,
        },
        SwapState::AwaitingTick {
            published: G1,
            next_fill: NextFill::Needed,
        },
    ];
    for state in states {
        match state {
            SwapState::Settled(_) | SwapState::Filling(_) | SwapState::AwaitingTick { .. } => {}
        }
    }
    states
}

fn every_event() -> [SwapEvent; 4] {
    let events = [
        SwapEvent::ParametersChanged,
        SwapEvent::DesignDone,
        SwapEvent::TickUsedGeneration(G0),
        SwapEvent::TickUsedGeneration(G1),
    ];
    for event in events {
        match event {
            SwapEvent::ParametersChanged
            | SwapEvent::DesignDone
            | SwapEvent::TickUsedGeneration(_) => {}
        }
    }
    events
}

#[test]
fn every_state_and_event_pair_has_the_transition_the_two_slot_swap_names() {
    use NextFill::{Needed, NotNeeded};
    use SwapCommand::{FillIdleSlot, Hold, Publish};
    use SwapEvent::{DesignDone, ParametersChanged, TickUsedGeneration};
    use SwapState::{AwaitingTick, Filling, Settled};
    let awaiting = |next_fill| AwaitingTick {
        published: G1,
        next_fill,
    };
    let table = [
        (
            Settled(G0),
            ParametersChanged,
            Filling(G1),
            FillIdleSlot(G1),
        ),
        (Settled(G0), DesignDone, Settled(G0), Hold),
        (Settled(G0), TickUsedGeneration(G0), Settled(G0), Hold),
        (Settled(G0), TickUsedGeneration(G1), Settled(G0), Hold),
        (
            Filling(G1),
            ParametersChanged,
            Filling(G1),
            FillIdleSlot(G1),
        ),
        (Filling(G1), DesignDone, awaiting(NotNeeded), Publish(G1)),
        (Filling(G1), TickUsedGeneration(G0), Filling(G1), Hold),
        (Filling(G1), TickUsedGeneration(G1), Filling(G1), Hold),
        (
            awaiting(NotNeeded),
            ParametersChanged,
            awaiting(Needed),
            Hold,
        ),
        (awaiting(NotNeeded), DesignDone, awaiting(NotNeeded), Hold),
        (
            awaiting(NotNeeded),
            TickUsedGeneration(G0),
            awaiting(NotNeeded),
            Hold,
        ),
        (
            awaiting(NotNeeded),
            TickUsedGeneration(G1),
            Settled(G1),
            Hold,
        ),
        (awaiting(Needed), ParametersChanged, awaiting(Needed), Hold),
        (awaiting(Needed), DesignDone, awaiting(Needed), Hold),
        (
            awaiting(Needed),
            TickUsedGeneration(G0),
            awaiting(Needed),
            Hold,
        ),
        (
            awaiting(Needed),
            TickUsedGeneration(G1),
            Filling(G2),
            FillIdleSlot(G2),
        ),
    ];
    let mut covered = 0;
    for state in every_state() {
        for event in every_event() {
            let row = table
                .iter()
                .find(|row| row.0 == state && row.1 == event)
                .unwrap_or_else(|| panic!("no row for {state:?} on {event:?}"));
            assert_eq!(state.step(event), (row.2, row.3), "{state:?} on {event:?}");
            covered += 1;
        }
    }
    assert_eq!(covered, table.len());
}

#[test]
fn consecutive_generations_alternate_between_the_two_slots() {
    assert_eq!(CoefficientSlot::of(G0), CoefficientSlot::First);
    assert_eq!(CoefficientSlot::of(G1), CoefficientSlot::Second);
    assert_eq!(CoefficientSlot::of(G2), CoefficientSlot::First);
}

struct TickModel {
    published: Generation,
    adopted: Generation,
    filling: Option<Generation>,
}

impl TickModel {
    fn new() -> Self {
        Self {
            published: G0,
            adopted: G0,
            filling: None,
        }
    }

    fn slots_the_tick_may_read(&self) -> [CoefficientSlot; 2] {
        [
            CoefficientSlot::of(self.published),
            CoefficientSlot::of(self.adopted),
        ]
    }

    fn random_event(&self, rng: &mut XorShift) -> SwapEvent {
        match rng.below(6) {
            0 | 1 => SwapEvent::ParametersChanged,
            2 => SwapEvent::DesignDone,
            3 => SwapEvent::TickUsedGeneration(self.adopted),
            _ => SwapEvent::TickUsedGeneration(self.published),
        }
    }

    fn observe_event(&mut self, event: SwapEvent) {
        if let SwapEvent::TickUsedGeneration(generation) = event {
            self.adopted = generation;
        }
    }

    fn observe_command(&mut self, command: SwapCommand) {
        match command {
            SwapCommand::Hold => {}
            SwapCommand::FillIdleSlot(generation) => self.filling = Some(generation),
            SwapCommand::Publish(generation) => {
                self.published = generation;
                self.filling = None;
            }
        }
    }
}

fn run_seeded_sequences(mut check: impl FnMut(u64, &TickModel, SwapEvent, SwapCommand)) {
    for seed in 0..SEQUENCES {
        let mut rng = XorShift::seeded(seed);
        let mut state = SwapState::Settled(G0);
        let mut model = TickModel::new();
        for _ in 0..EVENTS_PER_SEQUENCE {
            let event = model.random_event(&mut rng);
            let (next, command) = state.step(event);
            model.observe_event(event);
            check(seed, &model, event, command);
            model.observe_command(command);
            state = next;
        }
    }
}

#[test]
fn thread_mode_never_fills_a_slot_the_tick_may_still_read() {
    let mut fills = 0;
    run_seeded_sequences(|seed, model, _, command| {
        if let SwapCommand::FillIdleSlot(generation) = command {
            fills += 1;
            assert!(
                !model
                    .slots_the_tick_may_read()
                    .contains(&CoefficientSlot::of(generation)),
                "seed {seed}: fill {generation:?} with tick on {:?} and {:?}",
                model.published,
                model.adopted
            );
        }
    });
    assert!(fills > SEQUENCES);
}

#[test]
fn every_publish_is_the_whole_set_just_filled_and_the_generation_rises_by_one() {
    run_seeded_sequences(|seed, model, _, command| {
        if let SwapCommand::Publish(generation) = command {
            assert_eq!(model.filling, Some(generation), "seed {seed}");
            assert_eq!(generation, model.published.next(), "seed {seed}");
        }
    });
}

#[test]
fn no_parameter_change_is_lost_once_the_tick_keeps_adopting() {
    for seed in 0..SEQUENCES {
        let mut rng = XorShift::seeded(seed);
        let mut state = SwapState::Settled(G0);
        let mut model = TickModel::new();
        let mut change_not_yet_filled = false;
        let feed = |event, state: &mut SwapState, model: &mut TickModel, pending: &mut bool| {
            let (next, command) = state.step(event);
            if event == SwapEvent::ParametersChanged {
                *pending = true;
            }
            if let SwapCommand::FillIdleSlot(_) = command {
                *pending = false;
            }
            model.observe_event(event);
            model.observe_command(command);
            *state = next;
        };
        for _ in 0..EVENTS_PER_SEQUENCE {
            let event = model.random_event(&mut rng);
            feed(event, &mut state, &mut model, &mut change_not_yet_filled);
        }
        for _ in 0..3 {
            feed(
                SwapEvent::DesignDone,
                &mut state,
                &mut model,
                &mut change_not_yet_filled,
            );
            let published = model.published;
            feed(
                SwapEvent::TickUsedGeneration(published),
                &mut state,
                &mut model,
                &mut change_not_yet_filled,
            );
        }
        assert!(!change_not_yet_filled, "seed {seed}");
        assert_eq!(state, SwapState::Settled(model.published), "seed {seed}");
    }
}

use dsp::{
    Band, BandFrequency, BandGain, BandQ, CoefficientExchange, CoefficientSet, FilterKind,
    PeqPreset, PresetBand, SampleRate,
};

const BAND_CENTRES_HZ: [f64; 5] = [100.0, 300.0, 1_000.0, 3_000.0, 10_000.0];
const PRESET_FAMILY: u32 = 8;

fn preset(member: u32) -> PeqPreset {
    let mut preset = PeqPreset::FLAT;
    for (slot, centre) in preset.bands.iter_mut().zip(BAND_CENTRES_HZ) {
        *slot = PresetBand {
            band: Band {
                kind: FilterKind::Peaking,
                frequency: BandFrequency::from_hz(centre).expect("audible"),
                gain: BandGain::from_db(f64::from(member) * 1.5 - 5.75).expect("in range"),
                q: BandQ::new(1.0).expect("in range"),
            },
            enabled: true,
        };
    }
    preset
}

fn reference(member: u32, rate: SampleRate, generation: Generation) -> CoefficientSet {
    CoefficientSet::design(&preset(member), rate, generation)
}

#[test]
fn the_tick_starts_on_the_flat_set_at_the_first_generation() {
    let mut exchange = CoefficientExchange::flat_at(SampleRate::Hz48000);
    let (publisher, mut tick) = exchange.split();
    assert_eq!(
        *tick.adopt(),
        CoefficientSet::design(&PeqPreset::FLAT, SampleRate::Hz48000, G0)
    );
    assert_eq!(publisher.state(), SwapState::Settled(G0));
}

#[test]
fn a_published_set_reaches_the_next_tick_whole_and_the_tick_reports_its_generation() {
    let mut exchange = CoefficientExchange::flat_at(SampleRate::Hz48000);
    let (mut publisher, mut tick) = exchange.split();
    publisher.change(&preset(3), SampleRate::Hz192000);
    let used = *tick.adopt();
    assert_eq!(used, reference(3, SampleRate::Hz192000, G1));
    assert_eq!(used.generation(), G1);
    publisher.poll();
    assert_eq!(publisher.state(), SwapState::Settled(G1));
}

#[test]
fn changes_made_before_the_tick_adopts_the_last_publish_wait_and_only_the_latest_is_published() {
    let mut exchange = CoefficientExchange::flat_at(SampleRate::Hz48000);
    let (mut publisher, mut tick) = exchange.split();
    publisher.change(&preset(1), SampleRate::Hz48000);
    publisher.change(&preset(2), SampleRate::Hz48000);
    publisher.change(&preset(5), SampleRate::Hz96000);
    publisher.poll();
    assert_eq!(*tick.adopt(), reference(1, SampleRate::Hz48000, G1));
    assert_eq!(*tick.adopt(), reference(1, SampleRate::Hz48000, G1));
    publisher.poll();
    assert_eq!(*tick.adopt(), reference(5, SampleRate::Hz96000, G2));
    publisher.poll();
    assert_eq!(publisher.state(), SwapState::Settled(G2));
}

#[test]
fn a_publish_during_a_tick_leaves_the_set_that_tick_is_using_untouched() {
    let mut exchange = CoefficientExchange::flat_at(SampleRate::Hz48000);
    let (mut publisher, mut tick) = exchange.split();
    publisher.change(&preset(0), SampleRate::Hz48000);
    let in_use = tick.adopt();
    publisher.poll();
    publisher.change(&preset(7), SampleRate::Hz48000);
    publisher.poll();
    assert_eq!(*in_use, reference(0, SampleRate::Hz48000, G1));
    assert_eq!(*tick.adopt(), reference(7, SampleRate::Hz48000, G2));
}

fn latest_generation(state: SwapState) -> Generation {
    match state {
        SwapState::Settled(generation) | SwapState::Filling(generation) => generation,
        SwapState::AwaitingTick { published, .. } => published,
    }
}

#[test]
fn a_tick_racing_a_publishing_thread_always_uses_one_whole_designed_set() {
    const PUBLISHES: u32 = 20_000;
    let rate = SampleRate::Hz48000;
    let mut family: Vec<CoefficientSet> = (0..PRESET_FAMILY)
        .map(|member| reference(member, rate, G0))
        .collect();
    family.push(CoefficientSet::design(&PeqPreset::FLAT, rate, G0));
    let mut exchange = CoefficientExchange::flat_at(rate);
    let (mut publisher, mut tick) = exchange.split();
    let publisher_settled = std::sync::atomic::AtomicBool::new(false);
    let (last_adopted, adoptions, settled) = std::thread::scope(|scope| {
        let ticks = scope.spawn(|| {
            let mut last = G0;
            let mut adoptions = 0u32;
            while !publisher_settled.load(std::sync::atomic::Ordering::Acquire) {
                let used = *tick.adopt();
                let generation = used.generation();
                assert!(
                    family
                        .iter()
                        .any(|set| set.bands() == used.bands() && set.rate() == used.rate()),
                    "torn set at {generation:?}"
                );
                assert!(generation.value() >= last.value());
                adoptions += u32::from(generation != last);
                last = generation;
            }
            (last, adoptions)
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let in_time = || std::time::Instant::now() < deadline;
        let mut member = 0;
        while latest_generation(publisher.state()).value() < PUBLISHES && in_time() {
            member = (member + 1) % PRESET_FAMILY;
            publisher.change(&preset(member), rate);
            publisher.poll();
        }
        while !matches!(publisher.state(), SwapState::Settled(_)) && in_time() {
            publisher.poll();
        }
        let settled = latest_generation(publisher.state());
        publisher_settled.store(true, std::sync::atomic::Ordering::Release);
        let (last, adoptions) = ticks.join().expect("tick thread finished");
        (last, adoptions, settled)
    });
    assert!(
        settled.value() >= PUBLISHES,
        "publishing stalled at {settled:?}"
    );
    assert_eq!(last_adopted, settled);
    assert!(adoptions > 0);
}
