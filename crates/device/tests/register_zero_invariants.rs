#![cfg(feature = "register-zero-protocol")]

mod register_zero_support;

use device::command::Command;
use device::dac::{Attenuation, DacWrite, VolumeRegister};
use device::fault::FaultCode;
use device::input::Input;
use device::phantom::Milliseconds;
use device::register_zero::{
    ARMING_STEP_TIMEOUT, BoardFact, BoardRecord, ConfirmationNonce, Event, Output, State,
    TransmitBuffers, VolumeEvidence, transition,
};
use device::supervisor;
use register_zero_support::{NONCE, clean_record, locked, phantom_requested};

const SEQUENCES: u64 = 10_000;
const EVENTS_PER_SEQUENCE: usize = 1_000;
const WALK_LENGTH: usize = 5;

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
}

struct Records {
    phantom_one: BoardRecord,
    phantom_two: BoardRecord,
}

impl Records {
    fn new() -> Self {
        Self {
            phantom_one: BoardRecord {
                phantom: phantom_requested(Input::One),
                ..clean_record()
            },
            phantom_two: BoardRecord {
                phantom: phantom_requested(Input::Two),
                ..clean_record()
            },
        }
    }

    fn pick(&self, rng: &mut XorShift) -> BoardRecord {
        match rng.below(20) {
            0 => BoardRecord {
                volume: VolumeEvidence::Unconfirmed,
                ..clean_record()
            },
            1 => BoardRecord {
                transmit: TransmitBuffers::Live,
                ..clean_record()
            },
            2 => self.phantom_one,
            3 => self.phantom_two,
            _ => clean_record(),
        }
    }
}

fn nonce(rng: &mut XorShift) -> ConfirmationNonce {
    match rng.below(10) {
        0 => ConfirmationNonce::from_boot_entropy(rng.next() as u32),
        _ => NONCE,
    }
}

fn walk_event(step: usize, rng: &mut XorShift, records: &Records) -> Event {
    match step {
        0 => Event::ProtocolSigned,
        1 => Event::SpareConfirmed,
        2 => Event::OutputsUnplugged,
        3 => Event::ReplayRequested(records.pick(rng)),
        _ => Event::NonceTyped(nonce(rng), records.pick(rng)),
    }
}

fn any_event(rng: &mut XorShift, records: &Records) -> Event {
    match rng.below(14) {
        0..=4 => walk_event(rng.below(WALK_LENGTH as u64) as usize, rng, records),
        5 => Event::Aborted,
        6 => Event::PortClosed,
        7 => Event::PreconditionLost(BoardFact::PhantomRequestsClear),
        8 => Event::FaultRaised(FaultCode::BoardStalled),
        9 => Event::I2cAcked,
        10 => Event::I2cNacked,
        11 => Event::StatusRead(rng.below(256) as u8),
        12 => Event::Elapsed(Milliseconds(rng.below(20_000) as u32)),
        _ => Event::Elapsed(ARMING_STEP_TIMEOUT),
    }
}

fn board_event(rng: &mut XorShift, records: &Records) -> Event {
    match rng.below(100) {
        0..=59 => Event::I2cAcked,
        60..=74 => Event::StatusRead(0x00),
        75..=89 => Event::Elapsed(Milliseconds(rng.below(120) as u32)),
        90..=94 => Event::I2cNacked,
        95 => Event::FaultRaised(FaultCode::Panic),
        _ => any_event(rng, records),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Walking { reached: usize, waited: u32 },
    AfterWrite,
}

struct Model {
    phase: Phase,
}

impl Model {
    fn expects(reached: usize, event: Event) -> bool {
        match (reached, event) {
            (0, Event::ProtocolSigned)
            | (1, Event::SpareConfirmed)
            | (2, Event::OutputsUnplugged) => true,
            (3, Event::ReplayRequested(record)) => record == clean_record(),
            (4, Event::NonceTyped(typed, record)) => typed == NONCE && record == clean_record(),
            _ => false,
        }
    }

    fn step(&mut self, event: Event) -> Verdict {
        let Phase::Walking { reached, waited } = self.phase else {
            return Verdict::NoWrite;
        };
        if Self::expects(reached, event) {
            if reached + 1 == WALK_LENGTH {
                self.phase = Phase::AfterWrite;
                return Verdict::Write;
            }
            self.phase = Phase::Walking {
                reached: reached + 1,
                waited: 0,
            };
            return Verdict::NoWrite;
        }
        if reached == 0 {
            return Verdict::NoWrite;
        }
        if let Event::Elapsed(Milliseconds(ms)) = event {
            let waited = waited.saturating_add(ms);
            if waited < ARMING_STEP_TIMEOUT.0 {
                self.phase = Phase::Walking { reached, waited };
                return Verdict::NoWrite;
            }
        }
        self.phase = Phase::Walking {
            reached: 0,
            waited: 0,
        };
        Verdict::Disarm
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Verdict {
    NoWrite,
    Write,
    Disarm,
}

fn is_pre_write(state: State) -> bool {
    matches!(
        state,
        State::Locked(_) | State::Arming(..) | State::NonceIssued(_)
    )
}

struct Step<'a> {
    seed: u64,
    index: usize,
    before: State,
    event: Event,
    next: State,
    outputs: &'a [Output],
    verdict: Verdict,
    phase: Phase,
    writes_so_far: usize,
}

impl Step<'_> {
    fn at(&self) -> String {
        format!(
            "seed {} step {} {:?} {:?} -> {:?} {:?}",
            self.seed, self.index, self.before, self.event, self.next, self.outputs
        )
    }

    fn writes(&self) -> usize {
        register_zero_writes(self.outputs)
    }
}

fn register_zero_writes(outputs: &[Output]) -> usize {
    outputs
        .iter()
        .filter(|output| matches!(output, Output::WriteRegisterZero(_)))
        .count()
}

fn walk_every_sequence(mut check: impl FnMut(&Step)) {
    let records = Records::new();
    for seed in 1..=SEQUENCES {
        let mut rng = XorShift(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut model = Model {
            phase: Phase::Walking {
                reached: 0,
                waited: 0,
            },
        };
        let mut state = locked();
        let mut writes_so_far = 0;
        for index in 0..EVENTS_PER_SEQUENCE {
            let event = match model.phase {
                Phase::Walking { reached, .. } => match rng.below(10) {
                    0..=6 => walk_event(reached, &mut rng, &records),
                    _ => any_event(&mut rng, &records),
                },
                Phase::AfterWrite => board_event(&mut rng, &records),
            };
            let (next, outputs) = transition(state, event);
            let verdict = model.step(event);
            writes_so_far += register_zero_writes(outputs.as_slice());
            check(&Step {
                seed,
                index,
                before: state,
                event,
                next,
                outputs: outputs.as_slice(),
                verdict,
                phase: model.phase,
                writes_so_far,
            });
            state = next;
        }
    }
}

#[test]
fn the_register_zero_write_leaves_only_from_the_armed_state_on_a_matching_nonce() {
    let mut writes = 0u64;
    walk_every_sequence(|step| {
        if step.writes() > 0 {
            writes += 1;
            assert!(
                matches!(step.before, State::NonceIssued(_)),
                "armed state only: {}",
                step.at()
            );
            assert!(
                matches!(
                    step.event,
                    Event::NonceTyped(typed, record) if typed == NONCE && record == clean_record()
                ),
                "a matching nonce with every board fact met: {}",
                step.at()
            );
        }
    });
    assert!(writes > SEQUENCES / 2, "writes reached: {writes}");
}

#[test]
fn arming_needs_every_precondition_in_order_and_nothing_less_emits_the_write() {
    walk_every_sequence(|step| {
        assert_eq!(
            step.writes() == 1,
            step.verdict == Verdict::Write,
            "the write appears exactly when the whole walk ends in a matching nonce: {}",
            step.at()
        );
        match step.phase {
            Phase::Walking { reached: 0, .. } => {
                assert_eq!(step.next, locked(), "{}", step.at())
            }
            Phase::Walking { .. } => assert!(
                is_pre_write(step.next) && step.next != locked(),
                "{}",
                step.at()
            ),
            Phase::AfterWrite => {}
        }
    });
}

#[test]
fn any_fault_timeout_port_close_lost_fact_or_stray_input_disarms_back_to_the_start() {
    let mut disarms = 0u64;
    walk_every_sequence(|step| {
        if step.verdict == Verdict::Disarm {
            disarms += 1;
            assert_eq!(
                step.next,
                locked(),
                "a disarm returns to the start: {}",
                step.at()
            );
            assert!(
                matches!(step.outputs, [Output::Disarmed(_)]),
                "a disarm says why and emits nothing else: {}",
                step.at()
            );
        }
    });
    assert!(disarms > SEQUENCES, "disarms reached: {disarms}");
}

#[test]
fn the_register_zero_write_is_emitted_at_most_once_per_session() {
    walk_every_sequence(|step| {
        assert!(step.writes_so_far <= 1, "at most one write: {}", step.at());
    });
}

#[test]
fn after_the_write_there_is_no_way_back_and_the_volume_only_moves_to_full_attenuation() {
    let mut held = 0u64;
    walk_every_sequence(|step| {
        if step.phase == Phase::AfterWrite {
            assert!(!is_pre_write(step.next), "no way back: {}", step.at());
            assert!(step.next.forbids_volume_changes(), "{}", step.at());
        }
        for output in step.outputs {
            if let Output::Command(Command::WriteDac(DacWrite::Volume(_, level))) = output {
                assert_eq!(
                    *level,
                    Attenuation::FULL,
                    "only full attenuation: {}",
                    step.at()
                );
            }
        }
        match step.before {
            State::HeldAttenuated | State::Failed(_) => {
                assert!(
                    step.outputs.is_empty(),
                    "terminal states are silent: {}",
                    step.at()
                );
                assert_eq!(
                    step.next,
                    step.before,
                    "terminal states absorb: {}",
                    step.at()
                );
            }
            State::Locked(_) | State::Arming(..) | State::NonceIssued(_) | State::Replaying(_) => {}
        }
        if step.next == State::HeldAttenuated && step.before != State::HeldAttenuated {
            held += 1;
        }
    });
    assert!(held > SEQUENCES / 10, "held reached: {held}");
}

#[test]
fn no_bus_command_leaves_the_session_before_the_write() {
    walk_every_sequence(|step| {
        if is_pre_write(step.before) {
            assert!(
                !step
                    .outputs
                    .iter()
                    .any(|output| matches!(output, Output::Command(_))),
                "no bus command before the write: {}",
                step.at()
            );
        }
    });
}

#[test]
fn a_failure_raises_one_fault_and_attenuates_every_volume_register() {
    let mut failures = 0u64;
    walk_every_sequence(|step| {
        let raised = step
            .outputs
            .iter()
            .filter(|output| {
                matches!(
                    output,
                    Output::Supervisor(supervisor::Event::FaultRaised(..))
                )
            })
            .count();
        if raised > 0 {
            assert_eq!(raised, 1, "{}", step.at());
            assert!(matches!(step.next, State::Failed(_)), "{}", step.at());
        }
        if matches!(step.next, State::Failed(_)) && !matches!(step.before, State::Failed(_)) {
            failures += 1;
            let attenuated: Vec<VolumeRegister> = step
                .outputs
                .iter()
                .filter_map(|output| match output {
                    Output::Command(Command::WriteDac(DacWrite::Volume(register, level)))
                        if *level == Attenuation::FULL =>
                    {
                        Some(*register)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(
                attenuated,
                VolumeRegister::ALL,
                "a failure attenuates every volume register: {}",
                step.at()
            );
        }
    });
    assert!(failures > SEQUENCES / 10, "failures reached: {failures}");
}
