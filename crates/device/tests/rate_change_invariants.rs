mod rate_change_support;

use device::adc::AdcKind;
use device::command::Command;
use device::dac::{Attenuation, DacWrite, VolumeRegister};
use device::fault::{FaultCode, I2cPeer};
use device::output_pin::OutputPin;
use device::phantom::{Milliseconds, PinLevel};
use device::pll::audio_pll_stabilisation;
use device::rate_change::{ClockValidity, Event, Output, OutputLevel, State, transition};
use device::steps::{
    MOST_ATTEMPTS_PER_TRANSFER, PLACEHOLDER_I2C_ANSWER_TIMEOUT_UNTIL_T23,
    PLACEHOLDER_STEP_CONFIRMATION_TIMEOUT_UNTIL_T23,
};
use dsp::{FramesPerTick, RateFamily, SampleRate};
use rate_change_support::{ADC_KINDS, completion_for};

const SEQUENCES: u64 = 10_000;
const EVENTS_PER_SEQUENCE: usize = 1_000;
const SETTLE_MS: u32 = 250;
const T15_SPEED_BIT_HOLD_MS: u32 = 5;

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

    fn rate(&mut self) -> SampleRate {
        SampleRate::ALL[self.below(6) as usize]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BoardHealth {
    Healthy,
    NacksSometimes,
}

fn event(random: &mut XorShift, health: BoardHealth) -> Event {
    let drawn = match random.below(100) {
        0..=2 => Event::SetCur(random.rate()),
        3..=32 => Event::Ticked,
        33..=57 => Event::Elapsed(Milliseconds(match random.below(4) {
            0 => 1,
            1 => random.below(30) as u32,
            2 => random.below(300) as u32,
            _ => 10,
        })),
        58..=77 => Event::I2cAcked,
        78..=85 => Event::I2cNacked,
        86..=87 => Event::PllLockTimedOut,
        _ => Event::StepDone,
    };
    match (health, drawn) {
        (BoardHealth::Healthy, Event::I2cNacked) => Event::I2cAcked,
        (BoardHealth::Healthy, Event::PllLockTimedOut) => Event::StepDone,
        _ => drawn,
    }
}

fn speed_in_the_d8_table(rate: SampleRate) -> FramesPerTick {
    match rate.hz() {
        44_100 | 48_000 => FramesPerTick::Six,
        88_200 | 96_000 => FramesPerTick::Twelve,
        _ => FramesPerTick::TwentyFour,
    }
}

fn family_in_the_t15_table(rate: SampleRate) -> RateFamily {
    match rate.hz() {
        44_100 | 88_200 | 176_400 => RateFamily::FortyFourPointOneKilohertz,
        _ => RateFamily::FortyEightKilohertz,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PllWait {
    Stable,
    Unconfirmed {
        needed: Milliseconds,
    },
    Counting {
        needed: Milliseconds,
        counted: u32,
        splits: u32,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SpeedBitHold {
    AdcEnabled,
    Unconfirmed,
    Counting { counted: u32, splits: u32 },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DacLevel {
    Knob,
    Full,
}

struct Board {
    adc: AdcKind,
    tick_timer_running: bool,
    dma_running: bool,
    i2s_running: bool,
    pll: RateFamily,
    pll_wait: PllWait,
    pll_waits_split: u32,
    speed_bit_hold: SpeedBitHold,
    speed_bit_holds_split: u32,
    i2s_speed: FramesPerTick,
    tick_speed: FramesPerTick,
    analog_speed: FramesPerTick,
    dac: [DacLevel; 4],
    pending_write: Option<VolumeRegister>,
    volume_held: bool,
    validity: ClockValidity,
    since_restart: Option<u32>,
}

fn register_index(register: VolumeRegister) -> usize {
    match register {
        VolumeRegister::MainA => 0,
        VolumeRegister::MainB => 1,
        VolumeRegister::HeadphoneA => 2,
        VolumeRegister::HeadphoneB => 3,
        other => panic!("a rate change never writes {other:?}"),
    }
}

impl Board {
    fn running_at(rate: SampleRate, adc: AdcKind) -> Self {
        Self {
            adc,
            tick_timer_running: true,
            dma_running: true,
            i2s_running: true,
            pll: family_in_the_t15_table(rate),
            pll_wait: PllWait::Stable,
            pll_waits_split: 0,
            speed_bit_hold: SpeedBitHold::AdcEnabled,
            speed_bit_holds_split: 0,
            i2s_speed: speed_in_the_d8_table(rate),
            tick_speed: speed_in_the_d8_table(rate),
            analog_speed: speed_in_the_d8_table(rate),
            dac: [DacLevel::Knob; 4],
            pending_write: None,
            volume_held: false,
            validity: ClockValidity::Valid,
            since_restart: Some(SETTLE_MS),
        }
    }

    fn transport_stopped(&self) -> bool {
        !self.tick_timer_running && !self.dma_running && !self.i2s_running
    }

    fn observe(&mut self, event: Event) {
        self.pll_wait = match (self.pll_wait, event) {
            (PllWait::Unconfirmed { needed }, Event::StepDone) => PllWait::Counting {
                needed,
                counted: 0,
                splits: 0,
            },
            (
                PllWait::Counting {
                    needed,
                    counted,
                    splits,
                },
                Event::Elapsed(more),
            ) => PllWait::Counting {
                needed,
                counted: counted.saturating_add(more.0),
                splits: splits + 1,
            },
            (wait, _) => wait,
        };
        self.speed_bit_hold = match (self.speed_bit_hold, event) {
            (SpeedBitHold::Unconfirmed, Event::StepDone) => SpeedBitHold::Counting {
                counted: 0,
                splits: 0,
            },
            (SpeedBitHold::Counting { counted, splits }, Event::Elapsed(more)) => {
                SpeedBitHold::Counting {
                    counted: counted.saturating_add(more.0),
                    splits: splits + 1,
                }
            }
            (hold, _) => hold,
        };
        match event {
            Event::Elapsed(more) => {
                if let Some(elapsed) = self.since_restart.as_mut() {
                    *elapsed = elapsed.saturating_add(more.0);
                }
            }
            Event::I2cAcked => {
                if let Some(register) = self.pending_write.take() {
                    self.dac[register_index(register)] = DacLevel::Full;
                }
            }
            Event::I2cNacked => self.pending_write = None,
            Event::SetCur(_) | Event::Ticked | Event::StepDone | Event::PllLockTimedOut => {}
        }
    }

    fn assert_silent_and_attenuated(&self, before: State, after: State, what: &str) {
        assert_eq!(
            before.output_level(),
            OutputLevel::Silent,
            "{what} while sounding"
        );
        assert_eq!(
            after.output_level(),
            OutputLevel::Silent,
            "{what} while sounding"
        );
        assert!(self.volume_held, "{what} with the volume slew free");
        assert_eq!(
            self.dac,
            [DacLevel::Full; 4],
            "{what} before the DAC is attenuated"
        );
    }

    fn apply(&mut self, output: Output, before: State, after: State) {
        match output {
            Output::Board(command) => self.execute(command, before, after),
            Output::SendAnalogWordAtSpeed(speed) => {
                assert_eq!(
                    self.adc,
                    AdcKind::Unanswered,
                    "speed bits only for ADC type 0"
                );
                self.assert_silent_and_attenuated(before, after, "speed bits");
                assert!(
                    self.transport_stopped(),
                    "speed bits with the clocks running"
                );
                self.analog_speed = speed;
            }
            Output::HoldVolumeAtFull => self.volume_held = true,
            Output::RampVolumeFromFull => {
                assert!(
                    self.since_restart.is_some_and(|ms| ms >= SETTLE_MS),
                    "volume ramps before the settle time"
                );
                assert!(matches!(after, State::FadingIn { .. }));
                self.volume_held = false;
                self.dac = [DacLevel::Knob; 4];
            }
            Output::ResetStreams(_) => {
                assert!(self.transport_stopped(), "logic reset while the tick runs");
                assert_eq!(
                    self.speed_bit_hold,
                    SpeedBitHold::AdcEnabled,
                    "logic reset with ADC enable still low"
                );
                match self.pll_wait {
                    PllWait::Stable => {}
                    PllWait::Unconfirmed { .. } => {
                        panic!("reset before the PLL switch was confirmed")
                    }
                    PllWait::Counting {
                        needed,
                        counted,
                        splits,
                    } => {
                        assert!(
                            counted >= needed.0,
                            "reset after {counted} ms of a {} ms PLL wait",
                            needed.0
                        );
                        if splits > 1 {
                            self.pll_waits_split += 1;
                        }
                        self.pll_wait = PllWait::Stable;
                    }
                }
            }
            Output::SwapCoefficients(_) | Output::ClearDspState => {
                assert!(self.transport_stopped(), "logic reset while the tick runs");
            }
            Output::ClockValidity(validity) => self.validity = validity,
            Output::RaiseFault(fault) => {
                assert!(matches!(after, State::Failed { .. }));
                match before {
                    State::Attenuating { .. } => {
                        assert_eq!(fault, FaultCode::Unanswered(I2cPeer::Dac));
                    }
                    State::SwitchingFamily { .. }
                    | State::StabilisingPll { .. }
                    | State::Reconfiguring { .. }
                    | State::Restarting { .. }
                    | State::Settling { .. } => {
                        assert!(
                            matches!(fault, FaultCode::AudioPllUnlocked | FaultCode::BoardStalled),
                            "{before:?} raised {fault:?}"
                        );
                    }
                    other => {
                        assert!(waits_for_the_board(other), "{other:?} raised {fault:?}");
                        assert_eq!(fault, FaultCode::BoardStalled);
                    }
                }
                assert!(self.volume_held, "a fault with the volume slew free");
            }
        }
    }

    fn execute(&mut self, command: Command, before: State, after: State) {
        match command {
            Command::WriteDac(DacWrite::Volume(register, level)) => {
                assert_eq!(level, Attenuation::FULL, "a rate change only attenuates");
                assert!(self.volume_held, "a volume write races the slew");
                self.pending_write = Some(register);
            }
            Command::StopTickTimer => {
                self.assert_silent_and_attenuated(before, after, "tick stop");
                self.tick_timer_running = false;
            }
            Command::StopDma => {
                self.assert_silent_and_attenuated(before, after, "DMA stop");
                self.dma_running = false;
            }
            Command::StopI2s => {
                self.assert_silent_and_attenuated(before, after, "I2S stop");
                self.i2s_running = false;
            }
            Command::SwitchPll0Family(family) => {
                self.assert_silent_and_attenuated(before, after, "PLL0 switch");
                assert!(
                    self.transport_stopped(),
                    "PLL0 switched with the clocks running"
                );
                assert_ne!(family, self.pll, "PLL0 switched within one family");
                assert_eq!(
                    self.speed_bit_hold,
                    SpeedBitHold::AdcEnabled,
                    "PLL0 switched with ADC enable still low"
                );
                self.pll = family;
                self.pll_wait = PllWait::Unconfirmed {
                    needed: audio_pll_stabilisation(family),
                };
            }
            Command::StartDma => {
                self.assert_silent_and_attenuated(before, after, "DMA start");
                self.dma_running = true;
            }
            Command::StartI2s(speed) => {
                self.assert_silent_and_attenuated(before, after, "I2S start");
                self.i2s_running = true;
                self.i2s_speed = speed;
            }
            Command::StartTickTimer(speed) => {
                self.assert_silent_and_attenuated(before, after, "tick start");
                self.tick_timer_running = true;
                self.tick_speed = speed;
                self.since_restart = Some(0);
            }
            Command::DrivePin(OutputPin::AdcEnable, PinLevel::Low) => {
                self.assert_silent_and_attenuated(before, after, "ADC enable low");
                assert_eq!(self.speed_bit_hold, SpeedBitHold::AdcEnabled);
                self.speed_bit_hold = SpeedBitHold::Unconfirmed;
            }
            Command::DrivePin(OutputPin::AdcEnable, PinLevel::High) => {
                self.assert_silent_and_attenuated(before, after, "ADC enable high");
                match self.speed_bit_hold {
                    SpeedBitHold::Counting { counted, splits } => {
                        assert!(
                            counted >= T15_SPEED_BIT_HOLD_MS,
                            "ADC enable high after {counted} ms of the {T15_SPEED_BIT_HOLD_MS} ms speed bit hold"
                        );
                        if splits > 1 {
                            self.speed_bit_holds_split += 1;
                        }
                    }
                    SpeedBitHold::AdcEnabled | SpeedBitHold::Unconfirmed => {
                        panic!("ADC enable high before the speed word was confirmed")
                    }
                }
                self.speed_bit_hold = SpeedBitHold::AdcEnabled;
            }
            Command::DrivePin(OutputPin::TransportActive, _) => {
                self.assert_silent_and_attenuated(before, after, "a transport pin");
            }
            other => panic!("a rate change never emits {other:?}"),
        }
    }

    fn assert_matches(&self, state: State, seed: u64) {
        assert_eq!(self.validity, state.clock_validity(), "seed {seed}");
        match state.output_level() {
            OutputLevel::Open | OutputLevel::FadingOut { .. } => {
                assert!(
                    !self.transport_stopped(),
                    "seed {seed} sounds with no clock"
                );
            }
            OutputLevel::FadingIn { .. } => {
                assert!(
                    !self.transport_stopped(),
                    "seed {seed} sounds with no clock"
                );
                assert!(
                    self.since_restart.is_some_and(|ms| ms >= SETTLE_MS),
                    "seed {seed}"
                );
            }
            OutputLevel::Silent => {}
        }
        if let State::Steady { rate, adc } = state {
            assert_eq!(self.pll, family_in_the_t15_table(rate), "seed {seed}");
            assert_eq!(self.i2s_speed, speed_in_the_d8_table(rate), "seed {seed}");
            assert_eq!(self.tick_speed, speed_in_the_d8_table(rate), "seed {seed}");
            if adc == AdcKind::Unanswered {
                assert_eq!(
                    self.analog_speed,
                    speed_in_the_d8_table(rate),
                    "seed {seed}"
                );
            }
            assert!(!self.volume_held, "seed {seed} steady with the volume held");
            assert_eq!(state.clock_validity(), ClockValidity::Valid);
        }
    }
}

#[derive(Default)]
struct Coverage {
    changes_restarted: u32,
    speed_words: u32,
    pll_switches: u32,
    dac_faults: u32,
    pll_waits_split: u32,
    speed_bit_holds_split: u32,
    set_cur_mid_sequence: u32,
}

impl Coverage {
    fn count(&mut self, output: Output) {
        match output {
            Output::ResetStreams(_) => self.changes_restarted += 1,
            Output::SendAnalogWordAtSpeed(_) => self.speed_words += 1,
            Output::Board(Command::SwitchPll0Family(_)) => self.pll_switches += 1,
            Output::RaiseFault(FaultCode::Unanswered(_)) => self.dac_faults += 1,
            _ => {}
        }
    }
}

fn run(seed: u64, coverage: &mut Coverage) {
    let mut random = XorShift(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let health = match seed % 2 {
        0 => BoardHealth::Healthy,
        _ => BoardHealth::NacksSometimes,
    };
    let start = random.rate();
    let adc = ADC_KINDS[random.below(3) as usize];
    let mut state = State::Steady { rate: start, adc };
    let mut board = Board::running_at(start, adc);
    let mut last_requested = start;
    let mut failed = false;
    let mut feed = |state: State, event: Event, board: &mut Board| -> State {
        board.observe(event);
        let (after, outputs) = transition(state, event);
        let in_sequence = !matches!(
            state,
            State::Steady { .. } | State::Coalescing { .. } | State::Failed { .. }
        );
        if in_sequence && matches!(event, Event::SetCur(_)) {
            coverage.set_cur_mid_sequence += 1;
        }
        for output in outputs.as_slice() {
            coverage.count(*output);
            board.apply(*output, state, after);
        }
        board.assert_matches(after, seed);
        after
    };
    for _ in 0..EVENTS_PER_SEQUENCE {
        let next_event = match (health, event(&mut random, health)) {
            (BoardHealth::Healthy, Event::Elapsed(_)) if waits_for_the_board(state) => {
                completion_for(state).expect("a change in progress")
            }
            (_, drawn) => drawn,
        };
        if let Event::SetCur(rate) = next_event {
            last_requested = rate;
        }
        state = feed(state, next_event, &mut board);
        if failed {
            assert!(
                matches!(state, State::Failed { .. }),
                "seed {seed} left Failed"
            );
        }
        failed = matches!(state, State::Failed { .. });
        assert_eq!(state.reported_rate(), last_requested, "seed {seed}");
    }
    for _ in 0..10_000 {
        let Some(completion) = completion_for(state) else {
            break;
        };
        state = feed(state, completion, &mut board);
    }
    coverage.pll_waits_split += board.pll_waits_split;
    coverage.speed_bit_holds_split += board.speed_bit_holds_split;
    match state {
        State::Steady { rate, .. } => assert_eq!(rate, last_requested, "seed {seed}"),
        State::Failed { reported, .. } => {
            assert_eq!(reported, last_requested, "seed {seed}");
            assert_eq!(state.output_level(), OutputLevel::Silent, "seed {seed}");
            assert_ne!(
                health,
                BoardHealth::Healthy,
                "seed {seed} failed on a healthy board"
            );
        }
        other => panic!("seed {seed} never settled: {other:?}"),
    }
}

fn waits_for_the_board(state: State) -> bool {
    matches!(
        state,
        State::Attenuating { .. }
            | State::Stopping { .. }
            | State::SettingSpeedBits { .. }
            | State::EnablingAdc { .. }
            | State::SwitchingFamily { .. }
            | State::Reconfiguring { .. }
            | State::Restarting { .. }
    )
}

fn releases_the_outputs(output: Output) -> bool {
    matches!(
        output,
        Output::RampVolumeFromFull
            | Output::Board(Command::DrivePin(
                OutputPin::TransportActive,
                PinLevel::High
            ))
            | Output::ClockValidity(ClockValidity::Valid)
    )
}

const MOST_ELAPSED_PER_EVENT_MS: u32 = 3;

fn withheld_fault_bound_ms() -> u32 {
    let attempts = u32::from(MOST_ATTEMPTS_PER_TRANSFER.count());
    let per_attempt = PLACEHOLDER_I2C_ANSWER_TIMEOUT_UNTIL_T23.0 + MOST_ELAPSED_PER_EVENT_MS;
    let per_step = PLACEHOLDER_STEP_CONFIRMATION_TIMEOUT_UNTIL_T23.0 + MOST_ELAPSED_PER_EVENT_MS;
    (attempts * per_attempt).max(per_step)
}

fn withheld_from(seed: u64) -> (u32, State) {
    let mut random = XorShift(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let from = random.rate();
    let adc = ADC_KINDS[random.below(3) as usize];
    let mut to = random.rate();
    while to == from {
        to = random.rate();
    }
    let mut state = transition(State::Steady { rate: from, adc }, Event::SetCur(to)).0;
    let mut waits_passed = random.below(8);
    loop {
        if waits_for_the_board(state) {
            if waits_passed == 0 {
                break;
            }
            waits_passed -= 1;
        }
        let completion = completion_for(state).expect("a change in progress");
        state = transition(state, completion).0;
        if matches!(state, State::Steady { .. }) {
            state = transition(state, Event::SetCur(from)).0;
        }
    }
    let waiting = state;
    let mut waited = 0;
    for _ in 0..10_000 {
        if matches!(state, State::Failed { .. }) {
            return (waited, waiting);
        }
        let event = match random.below(10) {
            0 => Event::SetCur(random.rate()),
            1..=3 => Event::Ticked,
            4 => Event::I2cNacked,
            _ => Event::Elapsed(Milliseconds(
                1 + random.below(u64::from(MOST_ELAPSED_PER_EVENT_MS)) as u32,
            )),
        };
        if let Event::Elapsed(more) = event {
            waited += more.0;
        }
        let (after, outputs) = transition(state, event);
        for output in outputs.as_slice() {
            assert!(
                !releases_the_outputs(*output),
                "seed {seed} released the outputs from {state:?} with {output:?}"
            );
        }
        assert_eq!(
            after.output_level(),
            OutputLevel::Silent,
            "seed {seed} sounds while the board is silent"
        );
        state = after;
    }
    panic!("seed {seed} never faulted from {waiting:?}");
}

#[test]
fn withheld_answers_and_confirmations_fault_in_bounded_time_with_the_outputs_held() {
    let bound = withheld_fault_bound_ms();
    let mut waits_seen = std::collections::BTreeSet::new();
    for seed in 0..SEQUENCES {
        let (waited, waiting) = withheld_from(seed);
        assert!(
            waited <= bound,
            "seed {seed} faulted after {waited} ms from {waiting:?}, more than {bound} ms"
        );
        waits_seen.insert(
            format!("{waiting:?}")
                .split_whitespace()
                .next()
                .map(str::to_owned),
        );
    }
    assert_eq!(waits_seen.len(), 7, "{waits_seen:?}");
}

#[test]
fn every_safety_invariant_holds_over_seeded_random_event_sequences_with_faults() {
    let mut coverage = Coverage::default();
    for seed in 0..SEQUENCES {
        run(seed, &mut coverage);
    }
    assert!(
        coverage.changes_restarted > 10_000,
        "{}",
        coverage.changes_restarted
    );
    assert!(coverage.speed_words > 1_000, "{}", coverage.speed_words);
    assert!(coverage.pll_switches > 1_000, "{}", coverage.pll_switches);
    assert!(coverage.dac_faults > 100, "{}", coverage.dac_faults);
    assert!(
        coverage.pll_waits_split > 1_000,
        "{}",
        coverage.pll_waits_split
    );
    assert!(
        coverage.speed_bit_holds_split > 1_000,
        "{}",
        coverage.speed_bit_holds_split
    );
    assert!(
        coverage.set_cur_mid_sequence > 1_000,
        "{}",
        coverage.set_cur_mid_sequence
    );
}
