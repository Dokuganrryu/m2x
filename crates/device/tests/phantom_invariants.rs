use device::analog_word::{AnalogWord, PreampCode};
use device::input::Input;
use device::phantom::{
    Event, MUTE_AFTER_PHANTOM_CHANGE, Milliseconds, Output, PhantomGrant, PhantomPower, PinLevel,
    State, transition,
};
use dsp::FramesPerTick;

const SEQUENCES: u64 = 2000;
const EVENTS_PER_SEQUENCE: usize = 600;

struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    fn input(&mut self) -> Input {
        Input::BOTH[self.below(2) as usize]
    }

    fn event(&mut self, with_buttons: bool) -> Event {
        if self.below(200) == 0 {
            return Event::FaultRaised;
        }
        match self.below(100) {
            0 => Event::Boot,
            1..=4 => Event::BootReachedRunning,
            5..=6 => Event::Suspend,
            7..=9 => Event::Resume,
            10..=19 => Event::JackInserted(self.input()),
            20..=34 => Event::JackRemoved(self.input()),
            35..=59 => Event::Elapsed(Milliseconds(match self.below(4) {
                0 => 5,
                1 => self.below(40) as u32,
                2 => self.below(1200) as u32,
                _ => 1000,
            })),
            60..=79 if with_buttons => Event::ButtonPressed(self.input()),
            80..=99 if with_buttons => Event::ButtonReleased(self.input()),
            _ => Event::Elapsed(Milliseconds(5)),
        }
    }
}

fn word_bits(grant: PhantomGrant) -> u16 {
    AnalogWord::new(
        PreampCode::MINIMUM,
        PreampCode::MINIMUM,
        FramesPerTick::Six,
        grant,
    )
    .bits()
}

fn powered(grant: PhantomGrant, input: Input) -> bool {
    grant.power(input) == PhantomPower::On
}

#[derive(Clone, Copy, Default)]
struct InputHistory {
    jack_read_zero_since_boot_and_last_insert: bool,
    millis_since_mute: Option<u64>,
    jack_inserted: bool,
    press_began_since_reset: bool,
    released_a_fresh_press: bool,
}

struct Observer {
    booted: bool,
    running_since_boot: bool,
    faulted_since_boot: bool,
    grant_on_the_wire: Option<PhantomGrant>,
    inputs: [InputHistory; 2],
}

fn index(input: Input) -> usize {
    match input {
        Input::One => 0,
        Input::Two => 1,
    }
}

impl Observer {
    fn new() -> Self {
        Self {
            booted: false,
            running_since_boot: false,
            faulted_since_boot: false,
            grant_on_the_wire: None,
            inputs: [InputHistory::default(); 2],
        }
    }

    fn check(&mut self, event: Event, actions: &[Output], after: State, context: &str) {
        if !self.booted {
            assert!(
                matches!(event, Event::Boot) || actions.is_empty(),
                "{context}: nothing is emitted before boot"
            );
        }
        if self.faulted_since_boot {
            assert!(
                matches!(event, Event::Boot) || actions.is_empty(),
                "{context}: nothing is emitted after a fault until a new boot"
            );
        }
        match event {
            Event::Boot => {
                assert_eq!(
                    actions,
                    [
                        Output::SendAnalogWord(PhantomGrant::NONE),
                        Output::DrivePhantomActivePin(PinLevel::Low)
                    ],
                    "{context}: boot emits exactly the all-off word then pin low"
                );
                assert_eq!(word_bits(PhantomGrant::NONE), 0x0000);
                self.booted = true;
                self.running_since_boot = false;
                self.faulted_since_boot = false;
                self.inputs = [InputHistory {
                    jack_inserted: true,
                    ..InputHistory::default()
                }; 2];
            }
            Event::FaultRaised | Event::Suspend => {
                for history in &mut self.inputs {
                    history.press_began_since_reset = false;
                }
            }
            Event::BootReachedRunning => self.running_since_boot = true,
            Event::JackRemoved(input) => {
                let history = &mut self.inputs[index(input)];
                history.jack_read_zero_since_boot_and_last_insert = true;
                if history.jack_inserted {
                    history.press_began_since_reset = false;
                }
                history.jack_inserted = false;
            }
            Event::JackInserted(input) => {
                let history = &mut self.inputs[index(input)];
                history.jack_read_zero_since_boot_and_last_insert = false;
                if !history.jack_inserted {
                    history.press_began_since_reset = false;
                }
                history.jack_inserted = true;
            }
            Event::ButtonPressed(input) => {
                self.inputs[index(input)].press_began_since_reset = true;
            }
            Event::ButtonReleased(input) => {
                let history = &mut self.inputs[index(input)];
                history.released_a_fresh_press = history.press_began_since_reset;
                history.press_began_since_reset = false;
            }
            Event::Elapsed(Milliseconds(ms)) => {
                for history in &mut self.inputs {
                    if let Some(since) = &mut history.millis_since_mute {
                        *since += u64::from(ms);
                    }
                }
            }
            Event::Resume => {}
        }

        let mut muted_now = [false; 2];
        for (position, action) in actions.iter().enumerate() {
            match *action {
                Output::MuteInput(input) => {
                    muted_now[index(input)] = true;
                    self.inputs[index(input)].millis_since_mute = Some(0);
                }
                Output::UnmuteInput(input) => {
                    let since = self.inputs[index(input)].millis_since_mute.take();
                    assert!(
                        since.is_some_and(|ms| ms >= u64::from(MUTE_AFTER_PHANTOM_CHANGE.0)),
                        "{context}: unmute {input:?} only a full second after its mute"
                    );
                }
                Output::SendAnalogWord(grant) => {
                    let level = match (powered(grant, Input::One), powered(grant, Input::Two)) {
                        (false, false) => PinLevel::Low,
                        _ => PinLevel::High,
                    };
                    assert_eq!(
                        actions.get(position + 1),
                        Some(&Output::DrivePhantomActivePin(level)),
                        "{context}: the word is sent first, then the pin follows the phantom bits"
                    );
                    for input in Input::BOTH {
                        let was = self.grant_on_the_wire.is_some_and(|g| powered(g, input));
                        let now = powered(grant, input);
                        if now && !was {
                            let history = self.inputs[index(input)];
                            assert_eq!(
                                event,
                                Event::ButtonReleased(input),
                                "{context}: phantom {input:?} rises only on its own button"
                            );
                            assert!(
                                self.running_since_boot,
                                "{context}: phantom rises only after boot reached running"
                            );
                            assert!(
                                !self.faulted_since_boot,
                                "{context}: phantom never rises after a fault until a new boot"
                            );
                            assert!(
                                history.jack_read_zero_since_boot_and_last_insert,
                                "{context}: phantom rises only while the jack pin reads zero"
                            );
                            assert!(
                                history.released_a_fresh_press,
                                "{context}: phantom {input:?} rises only on a press begun after the last jack change, fault, suspend or boot"
                            );
                        }
                        if now != was && !matches!(event, Event::Boot) {
                            assert!(
                                muted_now[index(input)],
                                "{context}: a phantom change on {input:?} mutes that input first"
                            );
                        }
                    }
                    self.grant_on_the_wire = Some(grant);
                }
                Output::DrivePhantomActivePin(_) => {
                    assert!(
                        position > 0 && matches!(actions[position - 1], Output::SendAnalogWord(_)),
                        "{context}: the pin never moves without a word just before it"
                    );
                }
            }
        }

        if self.booted {
            assert_eq!(
                self.grant_on_the_wire,
                Some(after.grant()),
                "{context}: the last word sent matches the machine's grant"
            );
        }
        let wire = self.grant_on_the_wire.unwrap_or(PhantomGrant::NONE);
        match event {
            Event::JackInserted(input) => assert!(
                !powered(wire, input),
                "{context}: the jack pin always clears {input:?}"
            ),
            Event::Suspend => assert_eq!(
                wire,
                PhantomGrant::NONE,
                "{context}: suspend clears both requests"
            ),
            Event::FaultRaised => {
                assert_eq!(
                    wire,
                    PhantomGrant::NONE,
                    "{context}: a fault clears both requests"
                );
                self.faulted_since_boot = self.booted;
            }
            Event::Boot
            | Event::BootReachedRunning
            | Event::ButtonPressed(_)
            | Event::ButtonReleased(_)
            | Event::JackRemoved(_)
            | Event::Elapsed(_)
            | Event::Resume => {}
        }
    }
}

fn walk(seed: u64, with_buttons: bool, start_with_boot: bool) -> Vec<PhantomGrant> {
    let mut random = XorShift(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut state = State::Disarmed;
    let mut observer = Observer::new();
    let mut words = Vec::new();
    for position in 0..EVENTS_PER_SEQUENCE {
        let event = if start_with_boot && position == 0 {
            Event::Boot
        } else {
            random.event(with_buttons)
        };
        let (next, actions) = transition(state, event);
        let context = format!("seed {seed} event {position} {event:?}");
        observer.check(event, actions.as_slice(), next, &context);
        words.extend(actions.as_slice().iter().filter_map(|a| match a {
            Output::SendAnalogWord(grant) => Some(*grant),
            Output::MuteInput(_) | Output::UnmuteInput(_) | Output::DrivePhantomActivePin(_) => {
                None
            }
        }));
        state = next;
    }
    words
}

#[test]
fn every_safety_invariant_holds_over_random_event_sequences() {
    for seed in 0..SEQUENCES {
        walk(seed, true, seed % 2 == 0);
    }
}

#[test]
fn the_first_word_after_boot_is_the_all_off_word() {
    for seed in 0..SEQUENCES {
        let words = walk(seed, true, true);
        assert_eq!(words.first().map(|g| word_bits(*g)), Some(0x0000));
    }
}

#[test]
fn no_sequence_without_a_button_event_ever_sets_a_phantom_bit() {
    for seed in 0..SEQUENCES {
        for grant in walk(seed, false, seed % 2 == 0) {
            assert_eq!(word_bits(grant) & 0x4040, 0, "seed {seed}");
        }
    }
}

#[test]
fn random_sequences_do_reach_phantom_on_for_both_inputs() {
    let mut seen = [false; 2];
    for seed in 0..SEQUENCES {
        for grant in walk(seed, true, true) {
            for input in Input::BOTH {
                seen[index(input)] |= powered(grant, input);
            }
        }
    }
    assert_eq!(seen, [true, true]);
}
