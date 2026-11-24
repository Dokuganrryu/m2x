mod rate_change_support;

use device::adc::AdcKind;
use device::analog_word::{AnalogWord, PreampCode};
use device::command::Command;
use device::dac::{Attenuation, DacWrite, VolumeRegister};
use device::output_pin::OutputPin;
use device::phantom::{PhantomGrant, PinLevel};
use device::rate_change::{ClockValidity, Event, Output, OutputLevel, State};
use dsp::{FramesPerTick, RateFamily, SampleRate};
use rate_change_support::{ADC_KINDS, run_until_settled, step};

fn speed_from_the_d8_table(rate: SampleRate) -> FramesPerTick {
    match rate.hz() {
        44_100 | 48_000 => FramesPerTick::Six,
        88_200 | 96_000 => FramesPerTick::Twelve,
        176_400 | 192_000 => FramesPerTick::TwentyFour,
        other => panic!("{other} Hz is not one of the six rates"),
    }
}

fn family_from_the_t15_table(rate: SampleRate) -> RateFamily {
    match rate.hz() {
        44_100 | 88_200 | 176_400 => RateFamily::FortyFourPointOneKilohertz,
        48_000 | 96_000 | 192_000 => RateFamily::FortyEightKilohertz,
        other => panic!("{other} Hz is not one of the six rates"),
    }
}

fn board(command: Command) -> Output {
    Output::Board(command)
}

fn full(register: VolumeRegister) -> Output {
    board(Command::WriteDac(DacWrite::Volume(
        register,
        Attenuation::FULL,
    )))
}

fn d8_order(from: SampleRate, to: SampleRate, adc: AdcKind) -> Vec<Output> {
    let speed = speed_from_the_d8_table(to);
    let mut expected = vec![
        Output::ClockValidity(ClockValidity::Invalid),
        Output::HoldVolumeAtFull,
        full(VolumeRegister::MainA),
        full(VolumeRegister::MainB),
        full(VolumeRegister::HeadphoneA),
        full(VolumeRegister::HeadphoneB),
        board(Command::StopTickTimer),
        board(Command::StopDma),
        board(Command::DrivePin(OutputPin::TransportActive, PinLevel::Low)),
        board(Command::StopI2s),
    ];
    if adc == AdcKind::Unanswered {
        expected.extend([
            board(Command::DrivePin(OutputPin::AdcEnable, PinLevel::Low)),
            Output::SendAnalogWordAtSpeed(speed),
            board(Command::DrivePin(OutputPin::AdcEnable, PinLevel::High)),
        ]);
    }
    if family_from_the_t15_table(from) != family_from_the_t15_table(to) {
        expected.push(board(Command::SwitchPll0Family(family_from_the_t15_table(
            to,
        ))));
    }
    expected.extend([
        Output::ResetStreams(to),
        Output::SwapCoefficients(to),
        Output::ClearDspState,
        board(Command::StartDma),
        board(Command::StartI2s(speed)),
        board(Command::StartTickTimer(speed)),
        board(Command::DrivePin(
            OutputPin::TransportActive,
            PinLevel::High,
        )),
        Output::RampVolumeFromFull,
        Output::ClockValidity(ClockValidity::Valid),
    ]);
    expected
}

fn change(
    from: SampleRate,
    to: SampleRate,
    adc: AdcKind,
) -> (State, Vec<(Event, State, Vec<Output>)>) {
    let (state, outputs) = step(State::Steady { rate: from, adc }, Event::SetCur(to));
    let mut trace = vec![(Event::SetCur(to), state, outputs)];
    let end = run_until_settled(state, &mut trace);
    (end, trace)
}

fn every_change() -> impl Iterator<Item = (SampleRate, SampleRate, AdcKind)> {
    SampleRate::ALL.into_iter().flat_map(|from| {
        SampleRate::ALL
            .into_iter()
            .filter(move |to| *to != from)
            .flat_map(move |to| ADC_KINDS.into_iter().map(move |adc| (from, to, adc)))
    })
}

#[test]
fn every_pair_of_rates_and_every_adc_kind_emit_the_d8_order() {
    let mut checked = 0;
    for (from, to, adc) in every_change() {
        let (end, trace) = change(from, to, adc);
        let emitted: Vec<Output> = trace.into_iter().flat_map(|(_, _, out)| out).collect();
        assert_eq!(
            emitted,
            d8_order(from, to, adc),
            "{from:?} to {to:?} with {adc:?}"
        );
        assert_eq!(end, State::Steady { rate: to, adc });
        checked += 1;
    }
    assert_eq!(checked, 6 * 5 * 3);
}

#[test]
fn speed_bits_follow_the_corrected_table_with_eighty_eight_point_two_at_double_speed() {
    let expected_bits = [
        (SampleRate::Hz44100, 0),
        (SampleRate::Hz48000, 0),
        (SampleRate::Hz88200, 1 << 15),
        (SampleRate::Hz96000, 1 << 15),
        (SampleRate::Hz176400, 1 << 7),
        (SampleRate::Hz192000, 1 << 7),
    ];
    for (to, bits) in expected_bits {
        let from = if to == SampleRate::Hz48000 {
            SampleRate::Hz44100
        } else {
            SampleRate::Hz48000
        };
        let (_, trace) = change(from, to, AdcKind::Unanswered);
        let speeds: Vec<FramesPerTick> = trace
            .iter()
            .flat_map(|(_, _, out)| out.iter())
            .filter_map(|output| match *output {
                Output::SendAnalogWordAtSpeed(speed) => Some(speed),
                _ => None,
            })
            .collect();
        assert_eq!(speeds.len(), 1, "{to:?} sends one word");
        let word = AnalogWord::new(
            PreampCode::MINIMUM,
            PreampCode::MINIMUM,
            speeds[0],
            PhantomGrant::NONE,
        );
        assert_eq!(word.bits() & (1 << 15 | 1 << 7), bits, "{to:?}");
    }
}

#[test]
fn an_adc_that_answers_on_i2c_never_gets_speed_bits_or_an_enable_pulse() {
    for (from, to, adc) in every_change().filter(|(_, _, adc)| *adc != AdcKind::Unanswered) {
        let (_, trace) = change(from, to, adc);
        assert!(
            trace
                .iter()
                .flat_map(|(_, _, out)| out.iter())
                .all(|output| !matches!(
                    output,
                    Output::SendAnalogWordAtSpeed(_)
                        | Output::Board(Command::DrivePin(OutputPin::AdcEnable, _))
                )),
            "{from:?} to {to:?} with {adc:?}"
        );
    }
}

#[test]
fn output_words_are_zero_from_the_end_of_the_fade_out_until_the_settle_time_has_passed() {
    for (from, to, adc) in every_change() {
        let (_, trace) = change(from, to, adc);
        let mut fade_out_ticks = 0;
        let mut fade_in_ticks = 0;
        let mut since_restart: Option<u32> = None;
        let mut before = State::Steady { rate: from, adc };
        for (event, state, outputs) in &trace {
            if let (Some(elapsed), Event::Elapsed(more)) = (since_restart.as_mut(), event) {
                *elapsed += more.0;
            }
            if outputs.contains(&board(Command::StartTickTimer(speed_from_the_d8_table(to)))) {
                since_restart = Some(0);
            }
            if *event == Event::Ticked {
                match before.output_level() {
                    OutputLevel::FadingOut { .. } => fade_out_ticks += 1,
                    OutputLevel::FadingIn { .. } => fade_in_ticks += 1,
                    OutputLevel::Open | OutputLevel::Silent => {}
                }
            }
            if let OutputLevel::FadingIn { .. } = state.output_level() {
                assert!(since_restart.is_some_and(|ms| ms >= 250), "settled first");
            }
            before = *state;
            let touches_hardware = outputs.iter().any(|output| match output {
                Output::Board(_) | Output::SendAnalogWordAtSpeed(_) => true,
                Output::HoldVolumeAtFull
                | Output::RampVolumeFromFull
                | Output::ResetStreams(_)
                | Output::SwapCoefficients(_)
                | Output::ClearDspState
                | Output::ClockValidity(_)
                | Output::RaiseFault(_) => false,
            });
            if touches_hardware {
                assert_eq!(state.output_level(), OutputLevel::Silent, "{event:?}");
            }
        }
        assert_eq!(fade_out_ticks, 32, "fade out takes 32 ticks");
        assert_eq!(fade_in_ticks, 32, "fade in takes 32 ticks");
    }
}

#[test]
fn clock_validity_reads_valid_only_once_the_change_is_complete() {
    for (from, to, adc) in every_change() {
        let (_, trace) = change(from, to, adc);
        let (last, rest) = trace.split_last().expect("a change emits steps");
        for (_, state, _) in rest {
            assert_eq!(state.clock_validity(), ClockValidity::Invalid);
        }
        assert_eq!(last.1.clock_validity(), ClockValidity::Valid);
    }
}

#[test]
fn get_cur_reports_the_new_rate_at_every_step_of_the_change() {
    for (from, to, adc) in every_change() {
        let (_, trace) = change(from, to, adc);
        assert!(
            trace
                .iter()
                .all(|(_, state, _)| state.reported_rate() == to)
        );
    }
}
