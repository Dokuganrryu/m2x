use device::command::Command;
use device::control_pass::Monitoring;
use device::dac::{Attenuation, DacWrite, VolumeRegister};
use device::expander::{ExpanderInputs, ExpanderOutputs};
use device::fault::{FaultCode, I2cPeer};
use device::i2c::Answer;
use device::panel_bus::{
    CONTROL_VOLUME_REGISTERS, Event, Output, PanelTransfer, State, transition,
};
use device::phantom::PhantomGrant;

fn leds(monitor_one: Monitoring) -> ExpanderOutputs {
    ExpanderOutputs::showing(PhantomGrant::NONE, [monitor_one, Monitoring::Off])
}

fn run(state: State, events: &[Event]) -> (State, Vec<Output>) {
    let mut state = state;
    let mut seen = Vec::new();
    for event in events {
        let (next, outputs) = transition(state, *event);
        state = next;
        seen.extend_from_slice(outputs.as_slice());
    }
    (state, seen)
}

fn level(steps: u8) -> Attenuation {
    Attenuation::from_half_decibel_steps(steps)
}

#[test]
fn nothing_starts_on_the_bus_until_the_bus_is_free() {
    let (state, seen) = run(State::START, &[Event::Wanted(PanelTransfer::Buttons)]);
    assert!(seen.is_empty());
    let (_, seen) = run(state, &[Event::BusFree]);
    assert_eq!(seen, vec![Output::Begin(Command::ReadExpanderInputs)]);
}

#[test]
fn no_expander_transfer_begins_before_the_expander_is_ready() {
    let byte = leds(Monitoring::On);
    let (held, seen) = run(
        State::AT_BOOT,
        &[
            Event::Wanted(PanelTransfer::Leds(byte)),
            Event::Wanted(PanelTransfer::Buttons),
            Event::BusFree,
            Event::BusFree,
        ],
    );
    assert!(seen.is_empty());
    assert_eq!(held.in_flight(), None);
    let (ready, seen) = run(held, &[Event::ExpanderReady]);
    assert!(seen.is_empty());
    let (_, seen) = run(
        ready,
        &[
            Event::BusFree,
            Event::Answered(Answer::Acked),
            Event::BusFree,
        ],
    );
    assert_eq!(
        seen,
        vec![
            Output::Begin(Command::WriteExpanderOutputs(byte)),
            Output::Begin(Command::ReadExpanderInputs),
        ]
    );
}

#[test]
fn a_volume_write_does_not_wait_for_the_expander() {
    let (_, seen) = run(
        State::AT_BOOT,
        &[
            Event::Wanted(PanelTransfer::Buttons),
            Event::Wanted(PanelTransfer::Volume(VolumeRegister::MainA, level(7))),
            Event::BusFree,
        ],
    );
    assert_eq!(
        seen,
        vec![Output::Begin(Command::WriteDac(DacWrite::Volume(
            VolumeRegister::MainA,
            level(7)
        )))]
    );
}

#[test]
fn a_button_read_answer_reaches_the_panel_as_expander_inputs() {
    let (state, seen) = run(
        State::START,
        &[
            Event::Wanted(PanelTransfer::Buttons),
            Event::BusFree,
            Event::Answered(Answer::Read(0x7B)),
        ],
    );
    assert_eq!(
        seen,
        vec![
            Output::Begin(Command::ReadExpanderInputs),
            Output::ButtonsRead(ExpanderInputs(0x7B))
        ]
    );
    assert_eq!(state, State::START);
}

#[test]
fn the_leds_go_first_so_a_phantom_change_shows_at_once_then_volume_in_register_order_then_buttons()
{
    let wanted = [
        Event::Wanted(PanelTransfer::Buttons),
        Event::Wanted(PanelTransfer::Leds(leds(Monitoring::On))),
        Event::Wanted(PanelTransfer::Volume(VolumeRegister::HeadphoneB, level(9))),
        Event::Wanted(PanelTransfer::Volume(VolumeRegister::MainA, level(4))),
    ];
    let (mut state, _) = run(State::START, &wanted);
    let mut begun = Vec::new();
    for _ in 0..4 {
        let (next, seen) = run(state, &[Event::BusFree, Event::Answered(Answer::Acked)]);
        state = next;
        begun.extend(seen);
    }
    assert_eq!(
        begun,
        vec![
            Output::Begin(Command::WriteExpanderOutputs(leds(Monitoring::On))),
            Output::Begin(Command::WriteDac(DacWrite::Volume(
                VolumeRegister::MainA,
                level(4)
            ))),
            Output::Begin(Command::WriteDac(DacWrite::Volume(
                VolumeRegister::HeadphoneB,
                level(9)
            ))),
            Output::Begin(Command::ReadExpanderInputs),
        ]
    );
}

#[test]
fn a_newer_level_for_the_same_register_replaces_the_one_not_yet_sent() {
    let (_, seen) = run(
        State::START,
        &[
            Event::Wanted(PanelTransfer::Volume(VolumeRegister::MainB, level(10))),
            Event::Wanted(PanelTransfer::Volume(VolumeRegister::MainB, level(30))),
            Event::BusFree,
            Event::Answered(Answer::Acked),
            Event::BusFree,
        ],
    );
    assert_eq!(
        seen,
        vec![Output::Begin(Command::WriteDac(DacWrite::Volume(
            VolumeRegister::MainB,
            level(30)
        )))]
    );
}

#[test]
fn the_same_led_byte_is_written_once_and_not_again_while_it_stays_shown() {
    let byte = leds(Monitoring::On);
    let (state, first) = run(
        State::START,
        &[
            Event::Wanted(PanelTransfer::Leds(byte)),
            Event::BusFree,
            Event::Answered(Answer::Acked),
        ],
    );
    assert_eq!(
        first,
        vec![Output::Begin(Command::WriteExpanderOutputs(byte))]
    );
    let (_, again) = run(
        state,
        &[Event::Wanted(PanelTransfer::Leds(byte)), Event::BusFree],
    );
    assert!(again.is_empty());
}

#[test]
fn a_refused_led_write_is_tried_again_when_the_byte_is_next_offered() {
    let byte = leds(Monitoring::On);
    let (state, _) = run(
        State::START,
        &[
            Event::Wanted(PanelTransfer::Leds(byte)),
            Event::BusFree,
            Event::Answered(Answer::Nacked),
            Event::BusFree,
        ],
    );
    let (_, again) = run(
        state,
        &[Event::Wanted(PanelTransfer::Leds(byte)), Event::BusFree],
    );
    assert_eq!(
        again,
        vec![Output::Begin(Command::WriteExpanderOutputs(byte))]
    );
}

#[test]
fn a_refused_volume_write_raises_the_dac_fault_so_a_cut_that_failed_never_goes_unnoticed() {
    let (_, seen) = run(
        State::START,
        &[
            Event::Wanted(PanelTransfer::Volume(VolumeRegister::MainA, level(200))),
            Event::BusFree,
            Event::Answered(Answer::Nacked),
        ],
    );
    assert_eq!(
        seen.last(),
        Some(&Output::Fault(FaultCode::Unanswered(I2cPeer::Dac)))
    );
}

#[test]
fn a_transfer_the_sequencer_preempts_goes_out_again_unless_a_newer_value_replaced_it() {
    let (state, _) = run(
        State::START,
        &[
            Event::Wanted(PanelTransfer::Volume(VolumeRegister::MainA, level(8))),
            Event::BusFree,
            Event::Preempted,
            Event::BusFree,
        ],
    );
    assert_eq!(
        state.in_flight(),
        Some(PanelTransfer::Volume(VolumeRegister::MainA, level(8)))
    );
    let (state, _) = run(
        state,
        &[
            Event::Wanted(PanelTransfer::Volume(VolumeRegister::MainA, level(12))),
            Event::Preempted,
            Event::BusFree,
        ],
    );
    assert_eq!(
        state.in_flight(),
        Some(PanelTransfer::Volume(VolumeRegister::MainA, level(12)))
    );
}

#[test]
fn a_stray_answer_with_nothing_in_flight_changes_nothing() {
    for answer in [Answer::Acked, Answer::Nacked, Answer::Read(1)] {
        let (state, seen) = run(State::START, &[Event::Answered(answer), Event::Preempted]);
        assert_eq!(state, State::START);
        assert!(seen.is_empty());
    }
}

#[test]
fn only_control_pass_volume_writes_and_expander_traffic_become_panel_transfers() {
    assert_eq!(
        PanelTransfer::of_pass_command(Command::WriteDac(DacWrite::Volume(
            VolumeRegister::HeadphoneA,
            level(3)
        ))),
        Some(PanelTransfer::Volume(VolumeRegister::HeadphoneA, level(3)))
    );
    assert_eq!(
        PanelTransfer::of_pass_command(Command::WriteExpanderDirection),
        None
    );
    assert_eq!(PanelTransfer::of_pass_command(Command::StopDma), None);
    assert_eq!(CONTROL_VOLUME_REGISTERS.len(), 4);
}

#[test]
fn a_volume_register_outside_the_control_pass_is_never_written_by_the_panel() {
    let (_, seen) = run(
        State::START,
        &[
            Event::Wanted(PanelTransfer::Volume(
                VolumeRegister::FactoryZeroA,
                level(0),
            )),
            Event::BusFree,
        ],
    );
    assert!(seen.is_empty());
}

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

fn random_event(rng: &mut Lcg) -> Event {
    let monitors = [Monitoring::Off, Monitoring::On];
    match rng.below(8) {
        0 => Event::Wanted(PanelTransfer::Volume(
            CONTROL_VOLUME_REGISTERS[rng.below(4) as usize],
            level(rng.below(256) as u8),
        )),
        1 => Event::Wanted(PanelTransfer::Leds(leds(monitors[rng.below(2) as usize]))),
        2 => Event::Wanted(PanelTransfer::Buttons),
        3 | 4 => Event::BusFree,
        5 => Event::Answered(Answer::Acked),
        6 => Event::Answered(match rng.below(3) {
            0 => Answer::Nacked,
            1 => Answer::Read(rng.below(256) as u8),
            _ => Answer::Acked,
        }),
        _ => Event::Preempted,
    }
}

#[test]
fn over_random_traffic_one_transfer_at_most_is_in_flight_and_the_last_wishes_are_written() {
    for seed in 0..2_000u64 {
        let mut rng = Lcg(seed);
        let mut state = State::START;
        let mut holding = false;
        let mut last_volume: [Option<Attenuation>; 4] = [None; 4];
        let mut last_leds = None;
        for _ in 0..300 {
            let event = random_event(&mut rng);
            if let Event::Wanted(PanelTransfer::Volume(register, value)) = event {
                let index = CONTROL_VOLUME_REGISTERS
                    .iter()
                    .position(|r| *r == register)
                    .unwrap();
                last_volume[index] = Some(value);
            }
            if let Event::Wanted(PanelTransfer::Leds(byte)) = event {
                last_leds = Some(byte);
            }
            let (next, outputs) = transition(state, event);
            for output in outputs.as_slice() {
                match output {
                    Output::Begin(_) => {
                        assert!(!holding, "seed {seed}: a second transfer began");
                        assert_eq!(event, Event::BusFree, "seed {seed}: began off a free bus");
                        holding = true;
                    }
                    Output::Fault(code) => {
                        assert_eq!(*code, FaultCode::Unanswered(I2cPeer::Dac));
                        assert!(matches!(state.in_flight(), Some(PanelTransfer::Volume(..))));
                        assert_eq!(event, Event::Answered(Answer::Nacked));
                    }
                    Output::ButtonsRead(_) => {
                        assert_eq!(state.in_flight(), Some(PanelTransfer::Buttons));
                    }
                }
            }
            if matches!(event, Event::Answered(_) | Event::Preempted) {
                holding = false;
            }
            assert_eq!(holding, next.in_flight().is_some(), "seed {seed}");
            state = next;
        }
        if holding {
            state = transition(state, Event::Answered(Answer::Acked)).0;
        }
        if let Some(byte) = last_leds {
            state = transition(state, Event::Wanted(PanelTransfer::Leds(byte))).0;
        }
        let mut written_volume: [Option<Attenuation>; 4] = [None; 4];
        for _ in 0..10 {
            let (next, outputs) = transition(state, Event::BusFree);
            if let [Output::Begin(Command::WriteDac(DacWrite::Volume(register, value)))] =
                outputs.as_slice()
            {
                let index = CONTROL_VOLUME_REGISTERS
                    .iter()
                    .position(|r| r == register)
                    .unwrap();
                written_volume[index] = Some(*value);
            }
            state = transition(next, Event::Answered(Answer::Acked)).0;
        }
        assert!(
            state.mailbox().is_empty(),
            "seed {seed}: the mailbox drains"
        );
        if let Some(byte) = last_leds {
            assert_eq!(
                state.mailbox().written_leds(),
                Some(byte),
                "seed {seed}: the last LED byte is shown"
            );
        }
        for (index, written) in written_volume.iter().enumerate() {
            if let Some(value) = written {
                assert_eq!(
                    Some(*value),
                    last_volume[index],
                    "seed {seed}: a drained level is the last one wanted"
                );
            }
        }
    }
}
