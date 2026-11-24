mod console_support;

use console_support::typed;
use device::console::{ConsoleCommand, Output, Refusal};
use dsp::{
    Band, BandFrequency, BandGain, BandQ, Ceiling, FilterKind, LimiterSettings, PEQ_BANDS,
    ReleaseTime,
};

fn outcome(line: &str) -> Output {
    let text = format!("{line}\n");
    match typed(text.as_bytes()).as_slice() {
        [only] => *only,
        other => panic!("{line:?} gave {other:?}"),
    }
}

fn band(kind: FilterKind, hz: f64, db: f64, q: f64) -> Band {
    Band {
        kind,
        frequency: BandFrequency::from_hz(hz).unwrap(),
        gain: BandGain::from_db(db).unwrap(),
        q: BandQ::new(q).unwrap(),
    }
}

fn set_band(line: &str) -> (usize, Band) {
    match outcome(line) {
        Output::Execute(ConsoleCommand::SetBand { band, settings }) => (band.index(), settings),
        other => panic!("{line:?} gave {other:?}"),
    }
}

fn limiter(line: &str) -> LimiterSettings {
    match outcome(line) {
        Output::Execute(ConsoleCommand::SetLimiter(settings)) => settings,
        other => panic!("{line:?} gave {other:?}"),
    }
}

fn limiter_settings(tenths: i16, millis: u16) -> LimiterSettings {
    LimiterSettings {
        ceiling: Ceiling::from_tenths_of_dbfs(tenths).unwrap(),
        release: ReleaseTime::from_millis(millis).unwrap(),
    }
}

#[test]
fn peq_sets_one_band_from_its_number_kind_frequency_gain_and_q() {
    assert_eq!(
        set_band("peq 1 peak 1000 6 1"),
        (0, band(FilterKind::Peaking, 1000.0, 6.0, 1.0))
    );
    assert_eq!(
        set_band("peq 5 highshelf 8000.5 -3.25 0.707"),
        (4, band(FilterKind::HighShelf, 8000.5, -3.25, 0.707))
    );
}

#[test]
fn peq_names_each_of_the_five_filter_kinds() {
    for (word, kind) in [
        ("peak", FilterKind::Peaking),
        ("lowshelf", FilterKind::LowShelf),
        ("highshelf", FilterKind::HighShelf),
        ("highpass", FilterKind::HighPass),
        ("lowpass", FilterKind::LowPass),
    ] {
        let (_, settings) = set_band(&format!("peq 2 {word} 100 0 1"));
        assert_eq!(settings.kind, kind, "{word}");
    }
}

#[test]
fn peq_band_numbers_run_from_one_to_the_band_count() {
    for number in 1..=PEQ_BANDS {
        let (index, _) = set_band(&format!("peq {number} peak 100 0 1"));
        assert_eq!(index, number - 1);
    }
    for number in ["0", "6", "255", "256", "99999999999999999999"] {
        assert_eq!(
            outcome(&format!("peq {number} peak 100 0 1")),
            Output::Reject(Refusal::SettingOutOfRange),
            "{number}"
        );
    }
}

#[test]
fn peq_off_disables_one_band_by_number() {
    match outcome("peq off 3") {
        Output::Execute(ConsoleCommand::DisableBand(band)) => assert_eq!(band.index(), 2),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        outcome("peq off 6"),
        Output::Reject(Refusal::SettingOutOfRange)
    );
}

#[test]
fn peq_accepts_the_edges_of_gain_frequency_and_q() {
    for (line, expected) in [
        ("peq 1 peak 20 12 0.3", (20.0, 12.0, 0.3)),
        ("peq 1 peak 20000 -12 10", (20_000.0, -12.0, 10.0)),
        ("peq 1 peak 20000.0 +12.0 10.0", (20_000.0, 12.0, 10.0)),
    ] {
        let (_, settings) = set_band(line);
        assert_eq!(
            (
                settings.frequency.hz(),
                settings.gain.db(),
                settings.q.value()
            ),
            expected,
            "{line}"
        );
    }
}

#[test]
fn peq_refuses_gain_frequency_or_q_just_past_their_ranges() {
    for line in [
        "peq 1 peak 1000 12.01 1",
        "peq 1 peak 1000 -12.5 1",
        "peq 1 peak 19.99 0 1",
        "peq 1 peak 20000.1 0 1",
        "peq 1 peak 1000 0 0.29",
        "peq 1 peak 1000 0 10.01",
        "peq 1 peak 0 0 1",
        "peq 1 peak 1000 0 0",
        "peq 1 peak 1000 0 -1",
        "peq 1 peak 1000 99999999999999999999999 1",
    ] {
        assert_eq!(
            outcome(line),
            Output::Reject(Refusal::SettingOutOfRange),
            "{line}"
        );
    }
}

#[test]
fn peq_refuses_numbers_that_are_not_plain_decimals() {
    for line in [
        "peq 1 peak 1e3 0 1",
        "peq 1 peak nan 0 1",
        "peq 1 peak inf 0 1",
        "peq 1 peak infinity 0 1",
        "peq 1 peak 0x400 0 1",
        "peq 1 peak 1000. 0 1",
        "peq 1 peak .5 0 1",
        "peq 1 peak 1,000 0 1",
        "peq 1 peak 1000 --1 1",
        "peq 1 peak 1000 + 1",
        "peq 1 peak 1000 - 1",
        "peq 1 peak 1000 0 1.2.3",
        "peq +1 peak 1000 0 1",
        "peq 1.0 peak 1000 0 1",
        "peq off -1",
    ] {
        assert_eq!(
            outcome(line),
            Output::Reject(Refusal::MalformedArguments),
            "{line}"
        );
    }
}

#[test]
fn peq_refuses_a_missing_or_extra_argument_or_an_unknown_kind() {
    for line in [
        "peq",
        "peq 1",
        "peq 1 peak",
        "peq 1 peak 1000",
        "peq 1 peak 1000 0",
        "peq 1 peak 1000 0 1 1",
        "peq 1 bell 1000 0 1",
        "peq 1 PEAK 1000 0 1",
        "peq off",
        "peq off 1 2",
        "peq off all",
    ] {
        assert_eq!(
            outcome(line),
            Output::Reject(Refusal::MalformedArguments),
            "{line}"
        );
    }
}

#[test]
fn lim_sets_the_ceiling_in_decibels_and_the_release_in_milliseconds() {
    assert_eq!(limiter("lim -1 50"), limiter_settings(-10, 50));
    assert_eq!(limiter("lim -1.5 200"), limiter_settings(-15, 200));
    assert_eq!(limiter("lim -0.1 5"), limiter_settings(-1, 5));
}

#[test]
fn lim_accepts_the_edges_of_the_d5_ranges() {
    assert_eq!(limiter("lim -20 1000"), limiter_settings(-200, 1000));
    assert_eq!(limiter("lim 0 5"), limiter_settings(0, 5));
    assert_eq!(limiter("lim -0 5"), limiter_settings(0, 5));
    assert_eq!(limiter("lim -20.0 1000"), limiter_settings(-200, 1000));
}

#[test]
fn lim_refuses_settings_just_past_the_d5_ranges() {
    for line in [
        "lim -20.1 50",
        "lim 0.1 50",
        "lim +1 50",
        "lim -1 4",
        "lim -1 1001",
        "lim -1 0",
        "lim -99999999999999 50",
        "lim -1 99999999999999",
    ] {
        assert_eq!(
            outcome(line),
            Output::Reject(Refusal::SettingOutOfRange),
            "{line}"
        );
    }
}

#[test]
fn lim_refuses_finer_than_tenths_fractional_milliseconds_or_missing_values() {
    for line in [
        "lim",
        "lim -1",
        "lim -1 50 1",
        "lim -1.25 50",
        "lim -1 50.0",
        "lim -1 +50",
        "lim -1 -50",
        "lim db 50",
        "lim -1. 50",
        "lim -1 5e1",
    ] {
        assert_eq!(
            outcome(line),
            Output::Reject(Refusal::MalformedArguments),
            "{line}"
        );
    }
}
