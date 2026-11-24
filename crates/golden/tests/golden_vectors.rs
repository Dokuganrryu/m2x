use std::path::PathBuf;

use dsp::{
    BandCoefficients, CoefficientFormat, CoefficientSet, Generation, I2sWord, LimiterCoefficients,
    LimiterSettings, SampleRate, TickMix,
};
use golden::{
    Channel, Cue, FILTER_KINDS, GoldenRun, STANDARD_SCRIPT, Script, ScriptCue, SeededNoise,
    TickDigest, design_grid_digest, five_band_preset, standard_run,
};

const PINNED_BY_PREFIX: &str = "pinned-by ticket ";
const PIN_REQUEST_VARIABLE: &str = "GOLDEN_PINNED_BY";

fn vectors_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vectors/golden.txt")
}

fn format_name(format: CoefficientFormat) -> &'static str {
    match format {
        CoefficientFormat::Q2_30 => "q2.30",
        CoefficientFormat::Q3_29 => "q3.29",
        CoefficientFormat::Q4_28 => "q4.28",
    }
}

fn band_line(rate: SampleRate, index: usize, band: &BandCoefficients) -> String {
    match band {
        BandCoefficients::Skip => format!("band {} {index} skip", rate.hz()),
        BandCoefficients::Filter(biquad) => {
            let [b0, b1, b2] = biquad.feedforward();
            let [f1, f2] = biquad.feedback();
            format!(
                "band {} {index} {} {b0} {b1} {b2} {f1} {f2}",
                rate.hz(),
                format_name(biquad.format())
            )
        }
    }
}

fn tick_line(rate: SampleRate, tick: u16, digest: TickDigest) -> String {
    let [pl, pr, cl, cr] = Channel::ALL.map(|channel| digest.of(channel));
    format!(
        "tick {} {tick} {pl:08x} {pr:08x} {cl:08x} {cr:08x}",
        rate.hz()
    )
}

fn host_vector_lines() -> Vec<String> {
    let mut lines = Vec::new();
    for rate in SampleRate::ALL {
        let set = CoefficientSet::design(&five_band_preset(), rate, Generation::FIRST);
        for (index, band) in set.bands().iter().enumerate() {
            lines.push(band_line(rate, index, band));
        }
    }
    for rate in SampleRate::ALL {
        for (name, kind) in FILTER_KINDS {
            lines.push(format!(
                "grid {} {name} {:08x}",
                rate.hz(),
                design_grid_digest(rate, kind)
            ));
        }
    }
    for rate in SampleRate::ALL {
        let mut run = standard_run(rate);
        while let Some(words) = run.advance() {
            lines.push(tick_line(rate, words.tick, words.digest()));
        }
    }
    lines
}

#[derive(Debug, PartialEq, Eq)]
enum Divergence {
    Line {
        number: usize,
        expected: String,
        actual: String,
    },
    TickChannel {
        rate: String,
        tick: String,
        channel: Channel,
        expected: String,
        actual: String,
    },
    Length {
        expected: usize,
        actual: usize,
    },
}

fn first_divergence(expected: &[&str], actual: &[&str]) -> Option<Divergence> {
    for (number, (want, got)) in expected.iter().zip(actual).enumerate() {
        if want == got {
            continue;
        }
        let want_fields: Vec<&str> = want.split(' ').collect();
        let got_fields: Vec<&str> = got.split(' ').collect();
        let same_tick = want_fields.len() == 7
            && got_fields.len() == 7
            && want_fields[0] == "tick"
            && want_fields[..3] == got_fields[..3];
        if same_tick {
            let channel = Channel::ALL
                .into_iter()
                .zip(3..)
                .find(|&(_, column)| want_fields[column] != got_fields[column]);
            if let Some((channel, column)) = channel {
                return Some(Divergence::TickChannel {
                    rate: want_fields[1].to_owned(),
                    tick: want_fields[2].to_owned(),
                    channel,
                    expected: want_fields[column].to_owned(),
                    actual: got_fields[column].to_owned(),
                });
            }
        }
        return Some(Divergence::Line {
            number: number + 1,
            expected: (*want).to_owned(),
            actual: (*got).to_owned(),
        });
    }
    (expected.len() != actual.len()).then_some(Divergence::Length {
        expected: expected.len(),
        actual: actual.len(),
    })
}

fn pinned_vectors() -> (String, Vec<String>) {
    let text = std::fs::read_to_string(vectors_path()).unwrap_or_default();
    let mut lines = text.lines().map(str::to_owned);
    let header = lines.next().unwrap_or_default();
    (header, lines.collect())
}

fn ticket_named_by(header: &str) -> Option<&str> {
    header
        .strip_prefix(PINNED_BY_PREFIX)
        .filter(|ticket| !ticket.is_empty() && ticket.chars().all(|c| c.is_ascii_digit()))
}

#[test]
fn the_pipeline_and_the_coefficient_design_reproduce_the_pinned_golden_vectors() {
    let actual = host_vector_lines();
    if let Ok(ticket) = std::env::var(PIN_REQUEST_VARIABLE) {
        let mut text = format!("{PINNED_BY_PREFIX}{ticket}\n");
        for line in &actual {
            text.push_str(line);
            text.push('\n');
        }
        std::fs::write(vectors_path(), text).expect("the vectors file is writable");
    }
    let (header, expected) = pinned_vectors();
    assert!(
        ticket_named_by(&header).is_some(),
        "{} must open with `{PINNED_BY_PREFIX}<number>`; found `{header}`",
        vectors_path().display()
    );
    let expected: Vec<&str> = expected.iter().map(String::as_str).collect();
    let actual: Vec<&str> = actual.iter().map(String::as_str).collect();
    if let Some(divergence) = first_divergence(&expected, &actual) {
        panic!(
            "golden vectors changed, first divergence: {divergence:?}. \
             A ticket that means to change pinned samples reruns this test with \
             {PIN_REQUEST_VARIABLE}=<its ticket number> and says so in its comments"
        );
    }
}

#[test]
fn a_planted_change_is_reported_at_its_first_tick_and_channel() {
    let expected = [
        "band 48000 0 q2.30 1 2 3 4 5",
        "tick 48000 0 00000001 00000002 00000003 00000004",
        "tick 48000 1 00000001 00000002 00000003 00000004",
        "tick 48000 2 00000001 00000002 00000003 00000004",
    ];
    let actual = [
        expected[0],
        expected[1],
        "tick 48000 1 00000001 00000002 0000dead 0000beef",
        "tick 48000 2 ffffffff 00000002 00000003 00000004",
    ];
    assert_eq!(
        first_divergence(&expected, &actual),
        Some(Divergence::TickChannel {
            rate: "48000".to_owned(),
            tick: "1".to_owned(),
            channel: Channel::CaptureLeft,
            expected: "00000003".to_owned(),
            actual: "0000dead".to_owned(),
        })
    );
}

#[test]
fn a_changed_coefficient_line_and_a_missing_tick_are_both_reported() {
    let expected = ["band 96000 3 q2.30 1 2 3 4 5", "tick 96000 0 0 0 0 0"];
    assert_eq!(
        first_divergence(&expected, &["band 96000 3 q3.29 1 2 3 4 5", expected[1]]),
        Some(Divergence::Line {
            number: 1,
            expected: expected[0].to_owned(),
            actual: "band 96000 3 q3.29 1 2 3 4 5".to_owned(),
        })
    );
    assert_eq!(
        first_divergence(&expected, &expected[..1]),
        Some(Divergence::Length {
            expected: 2,
            actual: 1,
        })
    );
}

#[test]
fn the_pinned_header_names_a_ticket_number() {
    assert_eq!(ticket_named_by("pinned-by ticket 09"), Some("09"));
    assert_eq!(ticket_named_by("pinned-by ticket "), None);
    assert_eq!(ticket_named_by("pinned-by ticket nine"), None);
    assert_eq!(ticket_named_by("tick 48000 0"), None);
}

#[derive(Default)]
struct Visited {
    bypass: usize,
    blend: usize,
    processed: usize,
}

#[test]
fn the_standard_script_plays_bypass_then_a_fade_then_processed_then_a_fade_back_at_every_rate() {
    for rate in SampleRate::ALL {
        let mut run = standard_run(rate);
        let mut visited = Visited::default();
        let mut sequence: Vec<&str> = Vec::new();
        while let Some(words) = run.advance() {
            let phase = match words.mix {
                TickMix::Bypass => {
                    visited.bypass += 1;
                    "bypass"
                }
                TickMix::Blend(_) => {
                    visited.blend += 1;
                    "blend"
                }
                TickMix::Processed => {
                    visited.processed += 1;
                    "processed"
                }
            };
            if sequence.last() != Some(&phase) {
                sequence.push(phase);
            }
        }
        assert_eq!(
            sequence,
            ["bypass", "blend", "processed", "blend", "bypass"],
            "{rate:?}"
        );
        assert_eq!(visited.blend, 2 * dsp::CROSSFADE_TICKS, "{rate:?}");
        assert!(visited.processed >= 16, "{rate:?} processed ticks");
        assert!(visited.bypass >= 16, "{rate:?} bypass ticks");
    }
}

#[test]
fn the_five_band_preset_filters_every_band_where_the_peq_runs() {
    for rate in SampleRate::ALL {
        let set = CoefficientSet::design(&five_band_preset(), rate, Generation::FIRST);
        assert_eq!(set.enabled_mask(), 0b1_1111, "{rate:?}");
    }
}

fn ceiling_in_output_word(rate: SampleRate) -> i64 {
    let coefficients = LimiterCoefficients::design(LimiterSettings::default(), rate.family());
    i64::from(coefficients.ceiling().to_q2_29()) << 2
}

const HALF_A_DECIBEL_BELOW: f64 = 0.944;

#[test]
fn the_limiter_holds_the_processed_ticks_at_its_ceiling_and_reaches_it() {
    for rate in SampleRate::ALL {
        let ceiling = ceiling_in_output_word(rate);
        let mut run = standard_run(rate);
        let mut loudest = 0i64;
        while let Some(words) = run.advance() {
            if words.mix != TickMix::Processed {
                continue;
            }
            for word in words.playback.iter().flatten() {
                let magnitude = i64::from(I2sWord::to_bits(*word)).abs();
                assert!(magnitude <= ceiling, "{rate:?} tick {}", words.tick);
                loudest = loudest.max(magnitude);
            }
        }
        assert!(
            loudest as f64 >= ceiling as f64 * HALF_A_DECIBEL_BELOW,
            "{rate:?}: loudest {loudest:#x} never reached the ceiling {ceiling:#x}"
        );
    }
}

fn captured_peak_after(script: Script, rate: SampleRate, from_tick: u16) -> i64 {
    let mut run = GoldenRun::new(rate, script, SeededNoise::seeded(rate.hz()));
    let mut peak = 0i64;
    while let Some(words) = run.advance() {
        if words.tick >= from_tick {
            for word in words.capture.iter().flatten() {
                peak = peak.max(i64::from(word.to_bits()).abs());
            }
        }
    }
    peak
}

#[test]
fn the_capture_gain_cue_lowers_the_captured_level_against_a_run_without_it() {
    let gain_cue_tick = STANDARD_SCRIPT
        .cues
        .iter()
        .find_map(|cue| matches!(cue.cue, Cue::CaptureGains(_)).then_some(cue.tick))
        .expect("the standard script changes the capture gain");
    let without: Vec<ScriptCue> = STANDARD_SCRIPT
        .cues
        .iter()
        .copied()
        .filter(|cue| !matches!(cue.cue, Cue::CaptureGains(_)))
        .collect();
    let without = Script {
        ticks: STANDARD_SCRIPT.ticks,
        cues: without.leak(),
    };
    for rate in SampleRate::ALL {
        let with_cue = captured_peak_after(STANDARD_SCRIPT, rate, gain_cue_tick + 1);
        let without_cue = captured_peak_after(without, rate, gain_cue_tick + 1);
        assert!(
            with_cue * 3 < without_cue * 2,
            "{rate:?}: {with_cue:#x} against {without_cue:#x}"
        );
    }
}

fn capture_digests(script: Script, rate: SampleRate) -> Vec<[u32; 2]> {
    let mut run = GoldenRun::new(rate, script, SeededNoise::seeded(rate.hz()));
    let mut digests = Vec::new();
    while let Some(words) = run.advance() {
        let digest = words.digest();
        digests.push([
            digest.of(Channel::CaptureLeft),
            digest.of(Channel::CaptureRight),
        ]);
    }
    digests
}

#[test]
fn a_cue_that_changes_only_the_second_capture_gain_moves_only_the_second_input() {
    let (split_tick, gains) = STANDARD_SCRIPT
        .cues
        .iter()
        .find_map(|cue| match cue.cue {
            Cue::CaptureGains([first, second]) if first != second => {
                Some((cue.tick, [first, second]))
            }
            Cue::CaptureGains(_) | Cue::Dsp(_) | Cue::Monitor(_) => None,
        })
        .expect("the standard script gives the inputs different capture gains");
    let without: Vec<ScriptCue> = STANDARD_SCRIPT
        .cues
        .iter()
        .copied()
        .filter(|cue| cue.cue != Cue::CaptureGains(gains))
        .collect();
    let without = Script {
        ticks: STANDARD_SCRIPT.ticks,
        cues: without.leak(),
    };
    for rate in SampleRate::ALL {
        let with_split = capture_digests(STANDARD_SCRIPT, rate);
        let without_split = capture_digests(without, rate);
        for (tick, (with, without)) in with_split.iter().zip(&without_split).enumerate() {
            assert_eq!(with[0], without[0], "{rate:?} tick {tick}: the first input");
            if tick < usize::from(split_tick) {
                assert_eq!(with[1], without[1], "{rate:?} tick {tick}: before the cue");
            } else {
                assert_ne!(with[1], without[1], "{rate:?} tick {tick}: after the cue");
            }
        }
    }
}
