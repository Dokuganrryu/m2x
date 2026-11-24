use device::knob::{KnobReading, KnobReadings};
use device::knob_scan::{
    Awaiting, Event, GainCalibrationCode, GainCorrection, Output, ResultWord, ScanResults, State,
    transition,
};

fn result(tag: u32, value: u16) -> ResultWord {
    ResultWord(1 << 31 | tag << 24 | u32::from(value))
}

fn full_scan() -> ScanResults {
    [
        result(1, 0x1000),
        result(2, 0x2000),
        result(3, 0x3000),
        result(4, 0x4000),
        result(5, 0x5000),
    ]
    .into_iter()
    .fold(ScanResults::EMPTY, ScanResults::with)
}

fn step(state: State, event: Event) -> (State, Vec<Output>) {
    let (next, outputs) = transition(state, event);
    (next, outputs.as_slice().to_vec())
}

fn calibrated() -> State {
    let (state, _) = step(State::START, Event::CalibrationReady);
    let (state, _) = step(
        state,
        Event::GainCalibrated([GainCalibrationCode(0x8000), GainCalibrationCode(0x8000)]),
    );
    let (state, _) = step(state, Event::CalibrationReady);
    state
}

#[test]
fn the_scan_starts_with_the_offset_calibration_the_start_command_requested() {
    assert_eq!(State::START.awaiting(), Awaiting::CalibrationReady);
}

#[test]
fn offset_ready_requests_the_gain_calibration() {
    let (state, outputs) = step(State::START, Event::CalibrationReady);
    assert_eq!(outputs, vec![Output::RequestGainCalibration]);
    assert_eq!(state.awaiting(), Awaiting::GainCalibration);
}

#[test]
fn the_gain_corrections_follow_the_factory_formula_with_the_ready_bit() {
    assert_eq!(GainCorrection::of(GainCalibrationCode(0)).0, 0x0100_0000);
    assert_eq!(
        GainCorrection::of(GainCalibrationCode(0x8000)).0,
        0x0100_5555
    );
    assert_eq!(
        GainCorrection::of(GainCalibrationCode(0xFFFF)).0,
        0x0100_FFFF
    );
    assert_eq!(
        GainCorrection::of(GainCalibrationCode(0x7FFF)).0,
        0x0100_5554
    );
}

#[test]
fn gain_codes_are_written_as_corrections_and_then_the_unit_waits_for_calibration_ready() {
    let (state, _) = step(State::START, Event::CalibrationReady);
    let codes = [GainCalibrationCode(0x7FFF), GainCalibrationCode(0xFFFF)];
    let (state, outputs) = step(state, Event::GainCalibrated(codes));
    assert_eq!(
        outputs,
        vec![Output::WriteGainCorrections([
            GainCorrection(0x0100_5554),
            GainCorrection(0x0100_FFFF)
        ])]
    );
    assert_eq!(state.awaiting(), Awaiting::CalibrationReady);
    let (state, outputs) = step(state, Event::CalibrationReady);
    assert_eq!(outputs, vec![Output::ConfigureCommands, Output::Trigger]);
    assert_eq!(state.awaiting(), Awaiting::Results);
}

#[test]
fn a_full_scan_reports_the_readings_in_knob_order_and_starts_the_next_scan() {
    let (state, outputs) = step(calibrated(), Event::ResultsReady(full_scan()));
    assert_eq!(
        outputs,
        vec![
            Output::Readings(KnobReadings([
                KnobReading(0x1000),
                KnobReading(0x2000),
                KnobReading(0x3000),
                KnobReading(0x4000),
                KnobReading(0x5000)
            ])),
            Output::Trigger
        ]
    );
    assert_eq!(state.awaiting(), Awaiting::Results);
}

#[test]
fn a_scan_missing_a_knob_reports_nothing_and_scans_again() {
    let partial = [result(1, 1), result(2, 2), result(3, 3), result(5, 5)]
        .into_iter()
        .fold(ScanResults::EMPTY, ScanResults::with);
    let (_, outputs) = step(calibrated(), Event::ResultsReady(partial));
    assert_eq!(outputs, vec![Output::Trigger]);
}

#[test]
fn an_invalid_fifo_entry_or_an_unknown_tag_is_ignored() {
    let scan = full_scan()
        .with(ResultWord(6 << 24 | 0x7777 | 1 << 31))
        .with(ResultWord(2 << 24 | 0x9999));
    assert_eq!(scan.complete(), full_scan().complete());
}

#[test]
fn no_conversion_is_triggered_before_both_calibrations_finish() {
    let events = [
        Event::ResultsReady(full_scan()),
        Event::GainCalibrated([GainCalibrationCode(1); 2]),
        Event::CalibrationReady,
    ];
    for first in events {
        for second in events {
            let (state, a) = step(State::START, first);
            let (_, b) = step(state, second);
            let triggered = a.iter().chain(&b).any(|o| *o == Output::Trigger);
            assert!(
                !triggered,
                "{first:?} then {second:?} triggered a conversion"
            );
        }
    }
}

const STATES: [&str; 4] = ["offset", "gain", "settling", "converting"];

fn state_of(name: &str) -> State {
    match name {
        "offset" => State::START,
        "gain" => step(State::START, Event::CalibrationReady).0,
        "settling" => {
            step(
                step(State::START, Event::CalibrationReady).0,
                Event::GainCalibrated([GainCalibrationCode(0x8000); 2]),
            )
            .0
        }
        _ => calibrated(),
    }
}

fn label(state: State) -> &'static str {
    STATES
        .into_iter()
        .find(|name| state_of(name) == state)
        .expect("a known state")
}

#[test]
fn every_state_and_event_pair_has_its_pinned_result() {
    let table = [
        ("offset", "ready", "gain", "gain-request"),
        ("offset", "gain-codes", "offset", ""),
        ("offset", "results", "offset", ""),
        ("gain", "ready", "gain", ""),
        ("gain", "gain-codes", "settling", "corrections"),
        ("gain", "results", "gain", ""),
        ("settling", "ready", "converting", "commands trigger"),
        ("settling", "gain-codes", "settling", ""),
        ("settling", "results", "settling", ""),
        ("converting", "ready", "converting", ""),
        ("converting", "gain-codes", "converting", ""),
        ("converting", "results", "converting", "readings trigger"),
    ];
    for (from, event, to, emitted) in table {
        let event_value = match event {
            "ready" => Event::CalibrationReady,
            "gain-codes" => Event::GainCalibrated([GainCalibrationCode(0x8000); 2]),
            _ => Event::ResultsReady(full_scan()),
        };
        let (next, outputs) = step(state_of(from), event_value);
        let names: Vec<&str> = outputs
            .iter()
            .map(|output| match output {
                Output::RequestGainCalibration => "gain-request",
                Output::WriteGainCorrections(_) => "corrections",
                Output::ConfigureCommands => "commands",
                Output::Trigger => "trigger",
                Output::Readings(_) => "readings",
            })
            .collect();
        assert_eq!(label(next), to, "{from} on {event}");
        assert_eq!(names.join(" "), emitted, "{from} on {event}");
    }
}

#[test]
fn over_random_events_no_conversion_starts_before_offset_gain_and_settling_have_all_passed() {
    for seed in 0..1_000u64 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut state = State::START;
        let mut offset_done = false;
        let mut gain_written = false;
        for _ in 0..200 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let event = match x % 3 {
                0 => Event::CalibrationReady,
                1 => Event::GainCalibrated([GainCalibrationCode((x >> 8) as u16); 2]),
                _ => Event::ResultsReady(full_scan()),
            };
            let (next, outputs) = step(state, event);
            for output in &outputs {
                match output {
                    Output::RequestGainCalibration => offset_done = true,
                    Output::WriteGainCorrections(_) => {
                        assert!(offset_done, "seed {seed}: gain before offset");
                        gain_written = true;
                    }
                    Output::ConfigureCommands | Output::Trigger | Output::Readings(_) => {
                        assert!(gain_written, "seed {seed}: scan before calibration");
                    }
                }
            }
            state = next;
        }
    }
}
