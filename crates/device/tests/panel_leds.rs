use device::command::Command;
use device::control_pass::Monitoring;
use device::expander::{
    EXPANDER_ADDRESS, EXPANDER_INPUT_REGISTER, EXPANDER_OUTPUT_REGISTER, ExpanderOutputs,
};
use device::i2c::{self, Answer, BusAction, MasterStatus, Transfer};
use device::input::Input;
use device::phantom::{self, Milliseconds, PhantomGrant, PhantomPower};

const EVERY_LED_DARK: u8 = 0x72;
const PHANTOM_ONE_LED: u8 = 1 << 6;
const PHANTOM_TWO_LED: u8 = 1 << 1;
const MONITOR_ONE_LED: u8 = 1 << 5;
const MONITOR_TWO_LED: u8 = 1 << 4;

fn granted(one: PhantomPower, two: PhantomPower) -> PhantomGrant {
    let mut state = phantom::State::Disarmed;
    let mut feed = |event| state = phantom::transition(state, event).0;
    feed(phantom::Event::Boot);
    feed(phantom::Event::Elapsed(Milliseconds(2000)));
    feed(phantom::Event::BootReachedRunning);
    for (input, power) in [(Input::One, one), (Input::Two, two)] {
        feed(phantom::Event::JackRemoved(input));
        if power == PhantomPower::On {
            feed(phantom::Event::ButtonPressed(input));
            feed(phantom::Event::Elapsed(Milliseconds(100)));
            feed(phantom::Event::ButtonReleased(input));
        }
    }
    let grant = state.grant();
    assert_eq!(
        (grant.power(Input::One), grant.power(Input::Two)),
        (one, two)
    );
    grant
}

#[test]
fn with_no_phantom_and_no_monitoring_the_output_byte_is_the_factory_first_word_0x72() {
    let outputs = ExpanderOutputs::showing(PhantomGrant::NONE, [Monitoring::Off; 2]);
    assert_eq!(outputs.bits(), EVERY_LED_DARK);
}

#[test]
fn each_lit_led_pulls_its_own_active_low_bit_down_and_no_other() {
    let powers = [PhantomPower::Off, PhantomPower::On];
    let monitoring = [Monitoring::Off, Monitoring::On];
    for one in powers {
        for two in powers {
            let grant = granted(one, two);
            for monitor_one in monitoring {
                for monitor_two in monitoring {
                    let mut expected = EVERY_LED_DARK;
                    for (lit, bit) in [
                        (one == PhantomPower::On, PHANTOM_ONE_LED),
                        (two == PhantomPower::On, PHANTOM_TWO_LED),
                        (monitor_one == Monitoring::On, MONITOR_ONE_LED),
                        (monitor_two == Monitoring::On, MONITOR_TWO_LED),
                    ] {
                        if lit {
                            expected &= !bit;
                        }
                    }
                    let outputs = ExpanderOutputs::showing(grant, [monitor_one, monitor_two]);
                    assert_eq!(outputs.bits(), expected);
                }
            }
        }
    }
}

#[test]
fn a_phantom_led_never_shows_off_while_its_grant_is_on() {
    let grant = granted(PhantomPower::On, PhantomPower::On);
    for monitors in [
        [Monitoring::Off, Monitoring::Off],
        [Monitoring::On, Monitoring::Off],
        [Monitoring::Off, Monitoring::On],
        [Monitoring::On, Monitoring::On],
    ] {
        let bits = ExpanderOutputs::showing(grant, monitors).bits();
        assert_eq!(bits & PHANTOM_ONE_LED, 0);
        assert_eq!(bits & PHANTOM_TWO_LED, 0);
    }
}

#[test]
fn the_button_bits_0_2_3_and_7_are_always_written_low_as_in_the_factory_word() {
    let grant = granted(PhantomPower::On, PhantomPower::Off);
    let bits = ExpanderOutputs::showing(grant, [Monitoring::On, Monitoring::Off]).bits();
    assert_eq!(bits & 0x8D, 0);
}

#[test]
fn the_expander_registers_are_input_0_and_output_1() {
    assert_eq!(EXPANDER_INPUT_REGISTER, 0);
    assert_eq!(EXPANDER_OUTPUT_REGISTER, 1);
}

fn polled(state: i2c::State, status: MasterStatus) -> (i2c::State, Vec<i2c::Output>) {
    let (next, outputs) = i2c::transition(state, i2c::Event::Polled(status));
    (next, outputs.as_slice().to_vec())
}

#[test]
fn writing_the_expander_outputs_sends_address_0x39_register_1_then_the_byte() {
    let outputs = ExpanderOutputs::showing(PhantomGrant::NONE, [Monitoring::On, Monitoring::Off]);
    let transfer = Transfer::of(Command::WriteExpanderOutputs(outputs)).expect("a bus command");
    let (state, first) = i2c::transition(i2c::State::Idle, i2c::Event::Begin(transfer));
    assert_eq!(
        first.as_slice(),
        &[
            i2c::Output::Act(BusAction::Transmit(EXPANDER_ADDRESS << 1)),
            i2c::Output::Act(BusAction::Start)
        ]
    );
    let (state, second) = polled(state, MasterStatus::TransmitReady);
    assert_eq!(
        second,
        vec![
            i2c::Output::Act(BusAction::Transmit(EXPANDER_OUTPUT_REGISTER)),
            i2c::Output::Act(BusAction::Continue)
        ]
    );
    let (_, third) = polled(state, MasterStatus::TransmitReady);
    assert_eq!(
        third,
        vec![
            i2c::Output::Act(BusAction::Transmit(outputs.bits())),
            i2c::Output::Act(BusAction::Continue)
        ]
    );
}

#[test]
fn reading_the_expander_inputs_reads_one_byte_from_register_0() {
    let transfer = Transfer::of(Command::ReadExpanderInputs).expect("a bus command");
    let (mut state, first) = i2c::transition(i2c::State::Idle, i2c::Event::Begin(transfer));
    assert_eq!(
        first.as_slice(),
        &[
            i2c::Output::Act(BusAction::Transmit(EXPANDER_ADDRESS << 1)),
            i2c::Output::Act(BusAction::Start)
        ]
    );
    let mut answered = None;
    let mut sent = Vec::new();
    for status in [
        MasterStatus::TransmitReady,
        MasterStatus::TransmitReady,
        MasterStatus::ReceiveReady(0x5A),
        MasterStatus::Idle,
    ] {
        let (next, outputs) = polled(state, status);
        state = next;
        for output in outputs {
            match output {
                i2c::Output::Act(action) => sent.push(action),
                i2c::Output::Answered(answer) => answered = Some(answer),
            }
        }
    }
    assert!(sent.contains(&BusAction::Transmit(EXPANDER_INPUT_REGISTER)));
    assert!(sent.contains(&BusAction::Transmit(EXPANDER_ADDRESS << 1 | 1)));
    assert_eq!(answered, Some(Answer::Read(0x5A)));
}
