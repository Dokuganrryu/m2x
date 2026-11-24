mod rate_change_support;

use device::adc::AdcKind;
use device::command::Command;
use device::fault::FaultCode;
use device::output_pin::OutputPin;
use device::phantom::{Milliseconds, PinLevel};
use device::rate_change::{ClockValidity, Event, Output, State};
use dsp::{RateFamily, SampleRate};
use rate_change_support::{completion_for, step};

const EVENTS: [Event; 8] = [
    Event::SetCur(SampleRate::Hz96000),
    Event::Ticked,
    Event::Elapsed(Milliseconds(1)),
    Event::Elapsed(Milliseconds(251)),
    Event::I2cAcked,
    Event::I2cNacked,
    Event::StepDone,
    Event::PllLockTimedOut,
];

const TABLE: &[(&str, &str, &str, &str)] = &[
    ("steady", "set_cur", "V-", "coalescing"),
    ("steady", "tick", "", "steady"),
    ("steady", "elapsed_1", "", "steady"),
    ("steady", "elapsed", "", "steady"),
    ("steady", "acked", "", "steady"),
    ("steady", "nacked", "", "steady"),
    ("steady", "step_done", "", "steady"),
    ("coalescing", "set_cur", "", "coalescing"),
    ("coalescing", "tick", "", "coalescing"),
    ("coalescing", "elapsed_1", "", "coalescing"),
    ("coalescing", "elapsed", "", "fading_out"),
    ("coalescing", "acked", "", "coalescing"),
    ("coalescing", "nacked", "", "coalescing"),
    ("coalescing", "step_done", "", "coalescing"),
    ("fading_out", "set_cur", "", "fading_out"),
    ("fading_out", "tick", "", "fading_out"),
    ("fading_out", "elapsed_1", "", "fading_out"),
    ("fading_out", "elapsed", "", "fading_out"),
    ("fading_out", "acked", "", "fading_out"),
    ("fading_out", "nacked", "", "fading_out"),
    ("fading_out", "step_done", "", "fading_out"),
    ("attenuating", "set_cur", "", "attenuating"),
    ("attenuating", "tick", "", "attenuating"),
    ("attenuating", "elapsed_1", "", "attenuating"),
    ("attenuating", "elapsed", "W18", "attenuating"),
    ("attenuating", "acked", "W19", "attenuating"),
    ("attenuating", "nacked", "W18", "attenuating"),
    ("attenuating", "step_done", "", "attenuating"),
    ("stopping", "set_cur", "", "stopping"),
    ("stopping", "tick", "", "stopping"),
    ("stopping", "elapsed_1", "", "stopping"),
    ("stopping", "elapsed", "FAULT_OTHER", "failed"),
    ("stopping", "acked", "", "stopping"),
    ("stopping", "nacked", "", "stopping"),
    ("stopping", "step_done", "A29L WORD_Six", "speed_bits"),
    ("speed_bits", "set_cur", "", "speed_bits"),
    ("speed_bits", "tick", "", "speed_bits"),
    ("speed_bits", "elapsed_1", "", "speed_bits"),
    ("speed_bits", "elapsed", "FAULT_OTHER", "failed"),
    ("speed_bits", "acked", "", "speed_bits"),
    ("speed_bits", "nacked", "", "speed_bits"),
    ("speed_bits", "step_done", "", "speed_hold"),
    ("speed_hold", "set_cur", "", "speed_hold"),
    ("speed_hold", "tick", "", "speed_hold"),
    ("speed_hold", "elapsed_1", "", "speed_hold"),
    ("speed_hold", "elapsed", "A29H", "adc_enable"),
    ("speed_hold", "acked", "", "speed_hold"),
    ("speed_hold", "nacked", "", "speed_hold"),
    ("speed_hold", "step_done", "", "speed_hold"),
    ("adc_enable", "set_cur", "", "adc_enable"),
    ("adc_enable", "tick", "", "adc_enable"),
    ("adc_enable", "elapsed_1", "", "adc_enable"),
    ("adc_enable", "elapsed", "FAULT_OTHER", "failed"),
    ("adc_enable", "acked", "", "adc_enable"),
    ("adc_enable", "nacked", "", "adc_enable"),
    ("adc_enable", "step_done", "PLL44", "switching"),
    ("switching", "set_cur", "", "switching"),
    ("switching", "tick", "", "switching"),
    ("switching", "elapsed_1", "", "switching"),
    ("switching", "elapsed", "FAULT_OTHER", "failed"),
    ("switching", "acked", "", "switching"),
    ("switching", "nacked", "", "switching"),
    ("switching", "step_done", "", "stabilising"),
    ("stabilising", "set_cur", "", "stabilising"),
    ("stabilising", "tick", "", "stabilising"),
    ("stabilising", "elapsed_1", "", "stabilising"),
    (
        "stabilising",
        "elapsed",
        "RESET_44100 SWAP_44100 CLEAR",
        "reconfiguring",
    ),
    ("stabilising", "acked", "", "stabilising"),
    ("stabilising", "nacked", "", "stabilising"),
    ("stabilising", "step_done", "", "stabilising"),
    ("reconfiguring", "set_cur", "", "reconfiguring"),
    ("reconfiguring", "tick", "", "reconfiguring"),
    ("reconfiguring", "elapsed_1", "", "reconfiguring"),
    ("reconfiguring", "elapsed", "FAULT_OTHER", "failed"),
    ("reconfiguring", "acked", "", "reconfiguring"),
    ("reconfiguring", "nacked", "", "reconfiguring"),
    (
        "reconfiguring",
        "step_done",
        "D+ I+Six T+Six P28H",
        "restarting",
    ),
    ("restarting", "set_cur", "", "restarting"),
    ("restarting", "tick", "", "restarting"),
    ("restarting", "elapsed_1", "", "restarting"),
    ("restarting", "elapsed", "FAULT_OTHER", "failed"),
    ("restarting", "acked", "", "restarting"),
    ("restarting", "nacked", "", "restarting"),
    ("restarting", "step_done", "", "settling"),
    ("settling", "set_cur", "", "settling"),
    ("settling", "tick", "", "settling"),
    ("settling", "elapsed_1", "", "settling"),
    ("settling", "elapsed", "RAMP", "fading_in"),
    ("settling", "acked", "", "settling"),
    ("settling", "nacked", "", "settling"),
    ("settling", "step_done", "", "settling"),
    ("fading_in", "set_cur", "", "fading_in"),
    ("fading_in", "tick", "", "fading_in"),
    ("fading_in", "elapsed_1", "", "fading_in"),
    ("fading_in", "elapsed", "", "fading_in"),
    ("fading_in", "acked", "", "fading_in"),
    ("fading_in", "nacked", "", "fading_in"),
    ("fading_in", "step_done", "", "fading_in"),
    ("failed", "set_cur", "", "failed"),
    ("failed", "tick", "", "failed"),
    ("failed", "elapsed_1", "", "failed"),
    ("failed", "elapsed", "", "failed"),
    ("failed", "acked", "", "failed"),
    ("failed", "nacked", "", "failed"),
    ("failed", "step_done", "", "failed"),
    ("failed", "pll_timeout", "", "failed"),
    ("steady", "pll_timeout", "", "steady"),
    ("coalescing", "pll_timeout", "", "coalescing"),
    ("fading_out", "pll_timeout", "", "fading_out"),
    ("attenuating", "pll_timeout", "", "attenuating"),
    ("stopping", "pll_timeout", "", "stopping"),
    ("speed_bits", "pll_timeout", "", "speed_bits"),
    ("speed_hold", "pll_timeout", "", "speed_hold"),
    ("adc_enable", "pll_timeout", "", "adc_enable"),
    ("switching", "pll_timeout", "FAULT_OTHER", "failed"),
    ("stabilising", "pll_timeout", "FAULT_OTHER", "failed"),
    ("reconfiguring", "pll_timeout", "FAULT_OTHER", "failed"),
    ("restarting", "pll_timeout", "FAULT_OTHER", "failed"),
    ("settling", "pll_timeout", "FAULT_OTHER", "failed"),
    ("fading_in", "pll_timeout", "", "fading_in"),
];

fn state_name(state: State) -> &'static str {
    match state {
        State::Steady { .. } => "steady",
        State::Coalescing { .. } => "coalescing",
        State::FadingOut { .. } => "fading_out",
        State::Attenuating { .. } => "attenuating",
        State::Stopping { .. } => "stopping",
        State::SettingSpeedBits { .. } => "speed_bits",
        State::HoldingSpeedBits { .. } => "speed_hold",
        State::EnablingAdc { .. } => "adc_enable",
        State::SwitchingFamily { .. } => "switching",
        State::StabilisingPll { .. } => "stabilising",
        State::Reconfiguring { .. } => "reconfiguring",
        State::Restarting { .. } => "restarting",
        State::Settling { .. } => "settling",
        State::FadingIn { .. } => "fading_in",
        State::Failed { .. } => "failed",
    }
}

fn event_name(event: Event) -> &'static str {
    match event {
        Event::SetCur(_) => "set_cur",
        Event::Ticked => "tick",
        Event::Elapsed(Milliseconds(1)) => "elapsed_1",
        Event::Elapsed(_) => "elapsed",
        Event::I2cAcked => "acked",
        Event::I2cNacked => "nacked",
        Event::StepDone => "step_done",
        Event::PllLockTimedOut => "pll_timeout",
    }
}

fn code(output: Output) -> String {
    match output {
        Output::Board(command) => match command {
            Command::WriteDac(device::dac::DacWrite::Volume(register, level))
                if level == device::dac::Attenuation::FULL =>
            {
                format!("W{}", register as u8)
            }
            Command::StopTickTimer => "T-".into(),
            Command::StopDma => "D-".into(),
            Command::StopI2s => "I-".into(),
            Command::StartDma => "D+".into(),
            Command::StartI2s(speed) => format!("I+{speed:?}"),
            Command::StartTickTimer(speed) => format!("T+{speed:?}"),
            Command::DrivePin(OutputPin::TransportActive, PinLevel::Low) => "P28L".into(),
            Command::DrivePin(OutputPin::TransportActive, PinLevel::High) => "P28H".into(),
            Command::DrivePin(OutputPin::AdcEnable, PinLevel::Low) => "A29L".into(),
            Command::DrivePin(OutputPin::AdcEnable, PinLevel::High) => "A29H".into(),
            Command::SwitchPll0Family(RateFamily::FortyFourPointOneKilohertz) => "PLL44".into(),
            Command::SwitchPll0Family(RateFamily::FortyEightKilohertz) => "PLL48".into(),
            other => format!("{other:?}"),
        },
        Output::SendAnalogWordAtSpeed(speed) => format!("WORD_{speed:?}"),
        Output::HoldVolumeAtFull => "HOLD".into(),
        Output::RampVolumeFromFull => "RAMP".into(),
        Output::ResetStreams(rate) => format!("RESET_{}", rate.hz()),
        Output::SwapCoefficients(rate) => format!("SWAP_{}", rate.hz()),
        Output::ClearDspState => "CLEAR".into(),
        Output::ClockValidity(ClockValidity::Valid) => "V+".into(),
        Output::ClockValidity(ClockValidity::Invalid) => "V-".into(),
        Output::RaiseFault(FaultCode::Unanswered(_)) => "FAULT_DAC".into(),
        Output::RaiseFault(
            FaultCode::BoardStalled
            | FaultCode::Panic
            | FaultCode::HardFault
            | FaultCode::UnexpectedException
            | FaultCode::FenceUnavailable
            | FaultCode::RefusedCommand
            | FaultCode::CrystalNotReady
            | FaultCode::CorePllUnlocked
            | FaultCode::AudioPllUnlocked
            | FaultCode::UsbPhyPllUnlocked,
        ) => "FAULT_OTHER".into(),
        #[cfg(feature = "sound3")]
        Output::RaiseFault(FaultCode::CoreClockHeld) => "FAULT_OTHER".into(),
    }
}

fn one_of_each_state() -> Vec<State> {
    let mut state = State::Steady {
        rate: SampleRate::Hz48000,
        adc: AdcKind::Unanswered,
    };
    let mut states = vec![state];
    state = step(state, Event::SetCur(SampleRate::Hz44100)).0;
    while !matches!(state, State::Steady { .. }) {
        if states
            .iter()
            .all(|seen| state_name(*seen) != state_name(state))
        {
            states.push(state);
        }
        let event = completion_for(state).expect("a change in progress");
        state = step(state, event).0;
    }
    let mut attenuating = *states
        .iter()
        .find(|state| matches!(state, State::Attenuating { .. }))
        .expect("a change attenuates the DAC");
    for _ in 0..4 {
        attenuating = step(attenuating, Event::I2cNacked).0;
    }
    states.push(attenuating);
    states
}

#[test]
fn every_state_and_event_pair_has_a_pinned_result() {
    let states = one_of_each_state();
    assert_eq!(states.len(), 15, "one state per state variant");
    let mut checked = 0;
    for state in states {
        for event in EVENTS {
            let (next, outputs) = step(state, event);
            let codes: Vec<String> = outputs.into_iter().map(code).collect();
            let row = TABLE
                .iter()
                .find(|(s, e, _, _)| *s == state_name(state) && *e == event_name(event))
                .unwrap_or_else(|| panic!("no row for {state:?} on {event:?}"));
            assert_eq!(
                (codes.join(" ").as_str(), state_name(next)),
                (row.2, row.3),
                "{} on {}",
                row.0,
                row.1
            );
            if let Event::SetCur(rate) = event {
                assert_eq!(next.reported_rate(), rate, "{} reports SET CUR", row.0);
            }
            checked += 1;
        }
    }
    assert_eq!(checked, TABLE.len());
}
