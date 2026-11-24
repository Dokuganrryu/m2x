mod rate_change_support;

use device::adc::{AdcAddress, AdcKind};
use device::command::Command;
use device::dac::{Attenuation, DacWrite, VolumeRegister};
use device::fault::{FaultCode, I2cPeer};
use device::output_pin::OutputPin;
use device::phantom::{Milliseconds, PinLevel};
use device::pll::audio_pll_stabilisation;
use device::rate_change::{
    ClockValidity, Event, FACTORY_ADC_SPEED_BITS_HOLD, Output, OutputLevel, State,
};
use device::steps::PLACEHOLDER_STEP_CONFIRMATION_TIMEOUT_UNTIL_T23;
use dsp::{RateFamily, SampleRate};
use rate_change_support::{completion_for, step};

const AT_48K: State = State::Steady {
    rate: SampleRate::Hz48000,
    adc: AdcKind::Answered(AdcAddress::At13),
};

#[test]
fn a_set_cur_is_reported_at_once_and_only_marks_the_clock_invalid() {
    let (state, outputs) = step(AT_48K, Event::SetCur(SampleRate::Hz96000));
    assert_eq!(state.reported_rate(), SampleRate::Hz96000);
    assert_eq!(state.clock_validity(), ClockValidity::Invalid);
    assert_eq!(outputs, [Output::ClockValidity(ClockValidity::Invalid)]);
}

#[test]
fn set_cur_of_the_running_rate_changes_nothing() {
    let (state, outputs) = step(AT_48K, Event::SetCur(SampleRate::Hz48000));
    assert_eq!(state, AT_48K);
    assert!(outputs.is_empty());
    assert_eq!(state.clock_validity(), ClockValidity::Valid);
}

#[test]
fn further_set_cur_within_twenty_milliseconds_keep_the_last_and_restart_the_window() {
    let (state, _) = step(AT_48K, Event::SetCur(SampleRate::Hz96000));
    let (state, outputs) = step(state, Event::Elapsed(Milliseconds(19)));
    assert!(outputs.is_empty());
    let (state, outputs) = step(state, Event::SetCur(SampleRate::Hz44100));
    assert!(outputs.is_empty());
    assert_eq!(state.reported_rate(), SampleRate::Hz44100);
    let (state, _) = step(state, Event::Elapsed(Milliseconds(20)));
    assert!(matches!(state, State::Coalescing { .. }));
    let (state, outputs) = step(state, Event::Elapsed(Milliseconds(1)));
    assert!(matches!(state, State::FadingOut { .. }));
    assert!(outputs.is_empty());
}

#[test]
fn a_window_that_ends_on_the_running_rate_returns_to_steady_without_touching_hardware() {
    let (state, _) = step(AT_48K, Event::SetCur(SampleRate::Hz96000));
    let (state, _) = step(state, Event::SetCur(SampleRate::Hz48000));
    let (state, outputs) = step(state, Event::Elapsed(Milliseconds(21)));
    assert_eq!(state, AT_48K);
    assert_eq!(outputs, [Output::ClockValidity(ClockValidity::Valid)]);
}

fn advance_until(mut state: State, done: impl Fn(&State) -> bool) -> (State, Vec<Output>) {
    let mut all = Vec::new();
    while !done(&state) {
        let event = completion_for(state).expect("the machine is still changing");
        let (next, outputs) = step(state, event);
        all.extend(outputs);
        state = next;
    }
    (state, all)
}

fn attenuating_96k(adc: AdcKind) -> State {
    let start = State::Steady {
        rate: SampleRate::Hz48000,
        adc,
    };
    let (state, _) = step(start, Event::SetCur(SampleRate::Hz96000));
    advance_until(state, |state| matches!(state, State::Attenuating { .. })).0
}

fn first_write() -> Output {
    Output::Board(Command::WriteDac(DacWrite::Volume(
        VolumeRegister::MainA,
        Attenuation::FULL,
    )))
}

#[test]
fn a_nacked_volume_write_is_retried_three_times_and_then_the_change_proceeds() {
    let mut state = attenuating_96k(AdcKind::Unanswered);
    for _ in 0..3 {
        let (next, outputs) = step(state, Event::I2cNacked);
        assert_eq!(outputs, [first_write()]);
        state = next;
    }
    let (state, outputs) = step(state, Event::I2cAcked);
    assert!(matches!(state, State::Attenuating { .. }));
    assert_eq!(
        outputs,
        [Output::Board(Command::WriteDac(DacWrite::Volume(
            VolumeRegister::MainB,
            Attenuation::FULL,
        )))]
    );
}

#[test]
fn a_fourth_nack_fails_the_change_with_outputs_silent_and_the_clocks_untouched() {
    let mut state = attenuating_96k(AdcKind::Unanswered);
    for _ in 0..3 {
        state = step(state, Event::I2cNacked).0;
    }
    let (state, outputs) = step(state, Event::I2cNacked);
    assert_eq!(
        outputs,
        [Output::RaiseFault(FaultCode::Unanswered(I2cPeer::Dac))]
    );
    assert_eq!(state.output_level(), OutputLevel::Silent);
    assert_eq!(state.clock_validity(), ClockValidity::Invalid);
}

fn switching_to_44k1() -> State {
    let (state, _) = step(AT_48K, Event::SetCur(SampleRate::Hz44100));
    advance_until(state, |state| {
        matches!(state, State::SwitchingFamily { .. })
    })
    .0
}

fn stabilising_44k1_pll() -> State {
    let (state, outputs) = step(switching_to_44k1(), Event::StepDone);
    assert!(outputs.is_empty());
    assert!(matches!(state, State::StabilisingPll { .. }));
    state
}

#[test]
fn time_before_the_family_switch_is_confirmed_does_not_count_toward_the_pll_wait() {
    let (state, outputs) = step(
        switching_to_44k1(),
        Event::Elapsed(PLACEHOLDER_STEP_CONFIRMATION_TIMEOUT_UNTIL_T23),
    );
    assert!(matches!(state, State::SwitchingFamily { .. }));
    assert!(outputs.is_empty());
    let (state, outputs) = step(state, Event::StepDone);
    assert!(matches!(state, State::StabilisingPll { .. }));
    assert!(outputs.is_empty());
}

fn compositions_below(total: u32, prefix: &mut Vec<u32>, found: &mut Vec<Vec<u32>>) {
    found.push(prefix.clone());
    let spent: u32 = prefix.iter().sum();
    if prefix.len() >= 6 {
        return;
    }
    for part in 0..total - spent {
        prefix.push(part);
        compositions_below(total, prefix, found);
        prefix.pop();
    }
}

#[test]
fn a_pll_timeout_in_the_family_switch_fails_the_change_without_board_outputs() {
    let (state, outputs) = step(switching_to_44k1(), Event::PllLockTimedOut);
    assert_eq!(outputs, [Output::RaiseFault(FaultCode::AudioPllUnlocked)]);
    assert!(matches!(state, State::Failed { .. }));
    assert_eq!(state.clock_validity(), ClockValidity::Invalid);
    assert_eq!(state.output_level(), OutputLevel::Silent);
}

#[test]
fn a_pll_timeout_while_the_pll_stabilises_fails_the_change_without_board_outputs() {
    let (state, outputs) = step(stabilising_44k1_pll(), Event::PllLockTimedOut);
    assert_eq!(outputs, [Output::RaiseFault(FaultCode::AudioPllUnlocked)]);
    assert!(matches!(state, State::Failed { .. }));
    assert_eq!(state.clock_validity(), ClockValidity::Invalid);
    assert_eq!(state.output_level(), OutputLevel::Silent);
}

#[test]
fn a_pll_timeout_after_the_switch_stops_the_restart_and_the_volume_ramp() {
    for state in [
        switching_to_44k1(),
        stabilising_44k1_pll(),
        advance_until(stabilising_44k1_pll(), |state| {
            matches!(state, State::Reconfiguring { .. })
        })
        .0,
        advance_until(stabilising_44k1_pll(), |state| {
            matches!(state, State::Restarting { .. })
        })
        .0,
        advance_until(stabilising_44k1_pll(), |state| {
            matches!(state, State::Settling { .. })
        })
        .0,
    ] {
        let (after, outputs) = step(state, Event::PllLockTimedOut);
        assert!(matches!(after, State::Failed { .. }), "{state:?}");
        assert_eq!(
            outputs,
            [Output::RaiseFault(FaultCode::AudioPllUnlocked)],
            "{state:?}"
        );
        assert_eq!(after.clock_validity(), ClockValidity::Invalid);
    }
}

#[test]
fn the_pll_wait_never_ends_before_the_full_errata_time_for_any_split_of_elapsed_events() {
    let wait = audio_pll_stabilisation(RateFamily::FortyFourPointOneKilohertz).0;
    let mut splits = Vec::new();
    compositions_below(wait, &mut Vec::new(), &mut splits);
    assert_eq!(splits.len(), 84);
    for split in splits {
        let mut state = stabilising_44k1_pll();
        for part in &split {
            let (next, outputs) = step(state, Event::Elapsed(Milliseconds(*part)));
            assert!(matches!(next, State::StabilisingPll { .. }), "{split:?}");
            assert!(outputs.is_empty(), "{split:?}");
            state = next;
        }
        let spent: u32 = split.iter().sum();
        let (next, outputs) = step(state, Event::Elapsed(Milliseconds(wait - spent + 1)));
        assert!(matches!(next, State::Reconfiguring { .. }), "{split:?}");
        assert_eq!(
            outputs,
            [
                Output::ResetStreams(SampleRate::Hz44100),
                Output::SwapCoefficients(SampleRate::Hz44100),
                Output::ClearDspState,
            ]
        );
    }
}

#[test]
fn the_pll_wait_ignores_every_event_but_elapsed_time_and_set_cur() {
    let state = stabilising_44k1_pll();
    for event in [
        Event::StepDone,
        Event::Ticked,
        Event::I2cAcked,
        Event::I2cNacked,
    ] {
        let (next, outputs) = step(state, event);
        assert_eq!(next, state);
        assert!(outputs.is_empty());
    }
    let (next, outputs) = step(state, Event::SetCur(SampleRate::Hz192000));
    assert!(outputs.is_empty());
    assert!(matches!(next, State::StabilisingPll { .. }));
    assert_eq!(next.reported_rate(), SampleRate::Hz192000);
    assert_eq!(next.output_level(), OutputLevel::Silent);
}

fn setting_speed_bits_for_44k1() -> State {
    let start = State::Steady {
        rate: SampleRate::Hz48000,
        adc: AdcKind::Unanswered,
    };
    let (state, _) = step(start, Event::SetCur(SampleRate::Hz44100));
    advance_until(state, |state| {
        matches!(state, State::SettingSpeedBits { .. })
    })
    .0
}

fn holding_speed_bits_for_44k1() -> State {
    let (state, outputs) = step(setting_speed_bits_for_44k1(), Event::StepDone);
    assert!(outputs.is_empty());
    assert!(matches!(state, State::HoldingSpeedBits { .. }));
    state
}

#[test]
fn the_factory_speed_bit_hold_is_five_milliseconds() {
    assert_eq!(FACTORY_ADC_SPEED_BITS_HOLD, Milliseconds(5));
}

#[test]
fn the_speed_word_batch_lowers_adc_enable_and_sends_the_word_with_no_blocking_wait() {
    let start = State::Steady {
        rate: SampleRate::Hz48000,
        adc: AdcKind::Unanswered,
    };
    let (state, _) = step(start, Event::SetCur(SampleRate::Hz44100));
    let (state, _) = advance_until(state, |state| matches!(state, State::Stopping { .. }));
    let (state, outputs) = step(state, Event::StepDone);
    assert!(matches!(state, State::SettingSpeedBits { .. }));
    assert_eq!(
        outputs,
        [
            Output::Board(Command::DrivePin(OutputPin::AdcEnable, PinLevel::Low)),
            Output::SendAnalogWordAtSpeed(SampleRate::Hz44100.frames_per_tick()),
        ]
    );
}

#[test]
fn time_before_the_speed_word_is_confirmed_does_not_count_toward_the_speed_bit_hold() {
    let (state, outputs) = step(
        setting_speed_bits_for_44k1(),
        Event::Elapsed(PLACEHOLDER_STEP_CONFIRMATION_TIMEOUT_UNTIL_T23),
    );
    assert!(matches!(state, State::SettingSpeedBits { .. }));
    assert!(outputs.is_empty());
    let (state, outputs) = step(state, Event::StepDone);
    assert!(matches!(state, State::HoldingSpeedBits { .. }));
    assert!(outputs.is_empty());
    let (state, outputs) = step(state, Event::Elapsed(Milliseconds(4)));
    assert!(matches!(state, State::HoldingSpeedBits { .. }));
    assert!(outputs.is_empty());
}

#[test]
fn the_speed_bit_hold_never_ends_before_the_full_factory_time_for_any_split_of_elapsed_events() {
    let hold = FACTORY_ADC_SPEED_BITS_HOLD.0;
    let mut splits = Vec::new();
    compositions_below(hold, &mut Vec::new(), &mut splits);
    assert_eq!(splits.len(), 462);
    for split in splits {
        let mut state = holding_speed_bits_for_44k1();
        for part in &split {
            let (next, outputs) = step(state, Event::Elapsed(Milliseconds(*part)));
            assert!(matches!(next, State::HoldingSpeedBits { .. }), "{split:?}");
            assert!(outputs.is_empty(), "{split:?}");
            state = next;
        }
        let spent: u32 = split.iter().sum();
        let (next, outputs) = step(state, Event::Elapsed(Milliseconds(hold - spent + 1)));
        assert!(matches!(next, State::EnablingAdc { .. }), "{split:?}");
        assert_eq!(
            outputs,
            [Output::Board(Command::DrivePin(
                OutputPin::AdcEnable,
                PinLevel::High
            ))],
            "{split:?}"
        );
    }
}

#[test]
fn the_speed_bit_hold_ignores_every_event_but_elapsed_time_and_set_cur() {
    let state = holding_speed_bits_for_44k1();
    for event in [
        Event::StepDone,
        Event::Ticked,
        Event::I2cAcked,
        Event::I2cNacked,
    ] {
        let (next, outputs) = step(state, event);
        assert_eq!(next, state);
        assert!(outputs.is_empty());
    }
    let (next, outputs) = step(state, Event::SetCur(SampleRate::Hz192000));
    assert!(outputs.is_empty());
    assert!(matches!(next, State::HoldingSpeedBits { .. }));
    assert_eq!(next.reported_rate(), SampleRate::Hz192000);
    assert_eq!(next.output_level(), OutputLevel::Silent);
    assert_eq!(next.clock_validity(), ClockValidity::Invalid);
}

#[test]
fn the_family_switch_waits_until_the_board_confirms_adc_enable_is_high_again() {
    let (state, _) = step(
        holding_speed_bits_for_44k1(),
        Event::Elapsed(Milliseconds(FACTORY_ADC_SPEED_BITS_HOLD.0 + 1)),
    );
    let (state, outputs) = step(
        state,
        Event::Elapsed(PLACEHOLDER_STEP_CONFIRMATION_TIMEOUT_UNTIL_T23),
    );
    assert!(matches!(state, State::EnablingAdc { .. }));
    assert!(outputs.is_empty());
    let (state, outputs) = step(state, Event::StepDone);
    assert!(matches!(state, State::SwitchingFamily { .. }));
    assert_eq!(
        outputs,
        [Output::Board(Command::SwitchPll0Family(
            RateFamily::FortyFourPointOneKilohertz
        ))]
    );
}

#[test]
fn a_set_cur_mid_sequence_is_reported_at_once_and_runs_as_the_next_change() {
    let (state, _) = step(AT_48K, Event::SetCur(SampleRate::Hz96000));
    let (state, _) = advance_until(state, |state| matches!(state, State::Settling { .. }));
    let (state, outputs) = step(state, Event::SetCur(SampleRate::Hz176400));
    assert!(outputs.is_empty());
    assert_eq!(state.reported_rate(), SampleRate::Hz176400);
    let (state, _) = advance_until(state, |state| matches!(state, State::Coalescing { .. }));
    assert_eq!(state.clock_validity(), ClockValidity::Invalid);
    let (state, outputs) = advance_until(state, |state| matches!(state, State::Steady { .. }));
    assert_eq!(
        state,
        State::Steady {
            rate: SampleRate::Hz176400,
            adc: AdcKind::Answered(AdcAddress::At13),
        }
    );
    assert!(outputs.contains(&Output::ResetStreams(SampleRate::Hz176400)));
    assert!(outputs.contains(&Output::Board(Command::SwitchPll0Family(
        RateFamily::FortyFourPointOneKilohertz
    ))));
}

#[test]
fn a_set_cur_mid_sequence_for_the_rate_being_reached_needs_no_second_change() {
    let (state, _) = step(AT_48K, Event::SetCur(SampleRate::Hz96000));
    let (state, _) = advance_until(state, |state| matches!(state, State::Stopping { .. }));
    let (state, _) = step(state, Event::SetCur(SampleRate::Hz44100));
    let (state, _) = step(state, Event::SetCur(SampleRate::Hz96000));
    let (state, outputs) = advance_until(state, |state| matches!(state, State::Steady { .. }));
    assert_eq!(state.reported_rate(), SampleRate::Hz96000);
    assert_eq!(
        outputs
            .iter()
            .filter(|output| matches!(output, Output::ResetStreams(_)))
            .count(),
        1
    );
}
