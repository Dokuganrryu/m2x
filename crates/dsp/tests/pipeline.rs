#[allow(dead_code)]
mod seeded_sequences;

use dsp::{
    Band, BandFrequency, BandGain, BandQ, CaptureBlock, CoefficientExchange, CoefficientSet,
    Crossfade, DspMode, DspModeSwitch, FilterKind, FramesPerTick, Gain, Generation, I2sWord,
    InputMute, LimiterCoefficients, LimiterSettings, MAX_FRAMES, Meters, MonitorMix, OutputWords,
    PeqAtRate, PeqPreset, Pipeline, PlaybackBlock, PresetBand, RateFamily, SampleRate,
    StreamSample, TickControls, TickMix,
};
use seeded_sequences::XorShift;

const HALF: Gain = Gain::from_q5_27(0x0400_0000);
const LIVE: [InputMute; 2] = [InputMute::Live; 2];

struct Rig {
    pipeline: Pipeline,
    playback: PlaybackBlock,
    capture: CaptureBlock,
}

struct TickOut {
    playback: Vec<[i32; 2]>,
    capture: Vec<[i32; 2]>,
}

impl Rig {
    fn new() -> Self {
        Self {
            pipeline: Pipeline::default(),
            playback: PlaybackBlock::new(),
            capture: CaptureBlock::new(),
        }
    }

    fn tick(
        &mut self,
        controls: &TickControls<'_>,
        playback_in: &[[i32; 2]],
        adc_in: &[[i32; 2]],
    ) -> TickOut {
        let n = controls.frames.count();
        for (slot, words) in self.playback.input.iter_mut().zip(playback_in).take(n) {
            *slot = words.map(StreamSample::from_bits);
        }
        for (slot, words) in self.capture.input.iter_mut().zip(adc_in).take(n) {
            *slot = words.map(I2sWord::from_bits);
        }
        let mut meters = Meters::new();
        let (mut meter_tick, _) = meters.split();
        self.pipeline.tick(
            controls,
            &mut self.playback,
            &mut self.capture,
            &mut meter_tick,
        );
        TickOut {
            playback: self.playback.output[..n]
                .iter()
                .map(|f| f.map(I2sWord::to_bits))
                .collect(),
            capture: self.capture.output[..n]
                .iter()
                .map(|f| f.map(StreamSample::to_bits))
                .collect(),
        }
    }
}

fn controls<'a>(
    rate: SampleRate,
    mix: TickMix,
    coefficients: &'a CoefficientSet,
    monitor: MonitorMix,
    inputs: [InputMute; 2],
) -> TickControls<'a> {
    TickControls {
        frames: rate.frames_per_tick(),
        mix,
        coefficients,
        monitor,
        inputs,
        capture_gains: [Gain::UNITY; 2],
    }
}

fn band(kind: FilterKind, hz: f64, db: f64, q: f64) -> PresetBand {
    PresetBand {
        band: Band {
            kind,
            frequency: BandFrequency::from_hz(hz).unwrap(),
            gain: BandGain::from_db(db).unwrap(),
            q: BandQ::new(q).unwrap(),
        },
        enabled: true,
    }
}

fn twelve_db_boosts() -> [PeqPreset; 2] {
    let mut peak = PeqPreset::FLAT;
    peak.bands[2] = band(FilterKind::Peaking, 1_000.0, 12.0, 1.0);
    let mut shelves = PeqPreset::FLAT;
    shelves.bands[0] = band(FilterKind::LowShelf, 200.0, 12.0, 0.7);
    shelves.bands[4] = band(FilterKind::HighShelf, 6_000.0, 12.0, 0.7);
    [peak, shelves]
}

fn designed(preset: &PeqPreset, rate: SampleRate) -> CoefficientSet {
    CoefficientSet::design(preset, rate, Generation::FIRST)
}

fn flat(rate: SampleRate) -> CoefficientSet {
    designed(&PeqPreset::FLAT, rate)
}

fn default_ceiling_in_output_word() -> i64 {
    let coefficients =
        LimiterCoefficients::design(LimiterSettings::default(), RateFamily::FortyEightKilohertz);
    i64::from(coefficients.ceiling().to_q2_29()) << 2
}

struct Signal(Vec<[i32; 2]>);

impl Signal {
    fn tick(&self, tick: usize, frames: FramesPerTick) -> Vec<[i32; 2]> {
        let n = frames.count();
        (0..n)
            .map(|i| self.0[(tick * n + i) % self.0.len()])
            .collect()
    }
}

const SIGNAL_FRAMES: usize = 9_600;

fn on_stream_grid(value: f64) -> i32 {
    ((value.clamp(-1.0, 1.0 - 1.0 / 8_388_608.0) * 8_388_608.0) as i32) << 8
}

fn stress_signals(level: f64) -> Vec<(&'static str, Signal)> {
    let mut noise = XorShift::seeded(7);
    let stereo = |f: &dyn Fn(usize) -> f64| {
        Signal(
            (0..SIGNAL_FRAMES)
                .map(|n| {
                    [
                        on_stream_grid(level * f(n)),
                        on_stream_grid(-level * f(SIGNAL_FRAMES - 1 - n)),
                    ]
                })
                .collect(),
        )
    };
    let sweep = stereo(&|n| {
        let progress = n as f64 / SIGNAL_FRAMES as f64;
        (core::f64::consts::PI * 0.4 * SIGNAL_FRAMES as f64 * progress * progress).sin()
    });
    let impulses = stereo(&|n| match n % 97 {
        0 => 1.0,
        48 => -1.0,
        _ => 0.0,
    });
    let squares = stereo(&|n| {
        let half_period = 1 + (n / 300) % 41;
        if (n / half_period) % 2 == 0 {
            1.0
        } else {
            -1.0
        }
    });
    let white: Vec<f64> = (0..SIGNAL_FRAMES)
        .map(|_| (noise.next_word() >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0)
        .collect();
    let white = stereo(&|n| white[n]);
    vec![
        ("sweep", sweep),
        ("impulses", impulses),
        ("full-scale squares", squares),
        ("white noise", white),
    ]
}

fn silence(frames: FramesPerTick) -> Vec<[i32; 2]> {
    vec![[0, 0]; frames.count()]
}

fn assert_under_ceiling(words: &[[i32; 2]], what: &str) {
    let ceiling = default_ceiling_in_output_word();
    for (i, frame) in words.iter().enumerate() {
        for word in frame {
            assert!(
                i64::from(*word).abs() <= ceiling,
                "{what} frame {i}: {word:#x} is over the ceiling {ceiling:#x}"
            );
        }
    }
}

#[test]
fn on_with_a_flat_preset_plays_a_signal_under_the_ceiling_bit_exact_one_tick_later() {
    for rate in SampleRate::ALL {
        let frames = rate.frames_per_tick();
        let set = flat(rate);
        let on = controls(rate, TickMix::Processed, &set, MonitorMix::OFF, LIVE);
        let signals = stress_signals(0.85);
        let signal = &signals[3].1;
        let mut rig = Rig::new();
        let mut previous = silence(frames);
        for tick in 0..400 {
            let input = signal.tick(tick, frames);
            let out = rig.tick(&on, &input, &silence(frames));
            assert_eq!(out.playback, previous, "{rate:?} tick {tick}");
            previous = input;
        }
    }
}

#[test]
fn every_output_sample_holds_the_ceiling_with_a_twelve_db_boost_at_every_rate() {
    for rate in SampleRate::ALL {
        let frames = rate.frames_per_tick();
        for preset in twelve_db_boosts() {
            let set = designed(&preset, rate);
            for monitor in [
                MonitorMix::OFF,
                MonitorMix {
                    playback: Gain::UNITY,
                    inputs: [Gain::UNITY; 2],
                },
            ] {
                let on = controls(rate, TickMix::Processed, &set, monitor, LIVE);
                for (name, signal) in stress_signals(1.0) {
                    let mut rig = Rig::new();
                    for tick in 0..SIGNAL_FRAMES / frames.count() {
                        let input = signal.tick(tick, frames);
                        let out = rig.tick(&on, &input, &input);
                        assert_under_ceiling(&out.playback, &format!("{rate:?} {name} {tick}"));
                    }
                }
            }
        }
    }
}

fn first_sound(rig: &mut Rig, rate: SampleRate, mix: TickMix, set: &CoefficientSet) -> usize {
    let frames = rate.frames_per_tick();
    let n = frames.count();
    let tick_controls = controls(rate, mix, set, MonitorMix::OFF, LIVE);
    let mut heard = Vec::new();
    for tick in 0..6 {
        let mut input = silence(frames);
        if tick == 2 {
            input[n / 2] = [0x4000_0000, 0x4000_0000];
        }
        heard.extend(rig.tick(&tick_controls, &input, &silence(frames)).playback);
    }
    heard
        .iter()
        .position(|f| *f != [0, 0])
        .expect("the impulse is heard")
}

#[test]
fn an_impulse_comes_out_on_the_same_sample_in_bypass_and_on() {
    for rate in SampleRate::ALL {
        let n = rate.frames_per_tick().count();
        let boost = designed(&twelve_db_boosts()[0], rate);
        let bypass = first_sound(&mut Rig::new(), rate, TickMix::Bypass, &boost);
        let on = first_sound(&mut Rig::new(), rate, TickMix::Processed, &boost);
        assert_eq!(bypass, 3 * n + n / 2, "{rate:?}: bypass is one tick late");
        assert_eq!(on, bypass, "{rate:?}: on has the same latency");
    }
}

#[test]
fn a_monitored_input_feeds_both_output_sides_equally_at_its_mix_gain() {
    let rate = SampleRate::Hz48000;
    let frames = rate.frames_per_tick();
    let set = flat(rate);
    let monitor = MonitorMix {
        playback: HALF,
        inputs: [HALF, Gain::UNITY],
    };
    for mix in [TickMix::Bypass, TickMix::Processed] {
        let mut rig = Rig::new();
        let tick_controls = controls(rate, mix, &set, monitor, LIVE);
        let playback = vec![[0x1000_0000, -0x1000_0000]; frames.count()];
        let adc = vec![[0x2000_0000, 0x0800_0000]; frames.count()];
        rig.tick(&tick_controls, &playback, &adc);
        let out = rig.tick(&tick_controls, &playback, &adc);
        let expected = [
            0x0800_0000 + 0x1000_0000 + 0x0800_0000,
            -0x0800_0000 + 0x1000_0000 + 0x0800_0000,
        ];
        assert_eq!(out.playback, vec![expected; frames.count()], "{mix:?}");
    }
}

#[test]
fn a_monitor_sum_over_full_scale_saturates_at_the_output_and_never_wraps() {
    let rate = SampleRate::Hz96000;
    let frames = rate.frames_per_tick();
    let set = flat(rate);
    let double = Gain::from_q5_27(0x1000_0000);
    for (input_gain, expected) in [
        (Gain::UNITY, [0x7FFF_FFFC, 0x7FFF_FE00]),
        (double, [0x7FFF_FFFC, 0x7FFF_FFFC]),
    ] {
        let monitor = MonitorMix {
            playback: Gain::UNITY,
            inputs: [input_gain; 2],
        };
        let mut rig = Rig::new();
        let tick_controls = controls(rate, TickMix::Bypass, &set, monitor, LIVE);
        let playback = vec![[0x7FFF_FF00, i32::MIN]; frames.count()];
        let adc = vec![[0x7FFF_FFFF, 0x7FFF_FFFF]; frames.count()];
        rig.tick(&tick_controls, &playback, &adc);
        let out = rig.tick(&tick_controls, &playback, &adc);
        assert_eq!(
            out.playback,
            vec![expected; frames.count()],
            "{input_gain:?}"
        );
    }
}

fn reference_monitor_word(playback: i32, inputs: [i32; 2], gains: [i32; 3]) -> i32 {
    let q2_29 = |stream: i32| i128::from(stream >> 2);
    let sum = q2_29(playback) * i128::from(gains[0])
        + q2_29(inputs[0]) * i128::from(gains[1])
        + q2_29(inputs[1]) * i128::from(gains[2]);
    let rounded = (sum + (1 << 26)) >> 27;
    let full_scale = 1i128 << 29;
    (rounded.clamp(-full_scale, full_scale - 1) << 2) as i32
}

#[test]
fn the_monitor_sum_is_rounded_to_nearest_q2_29_and_saturated_once() {
    let rate = SampleRate::Hz88200;
    let frames = rate.frames_per_tick();
    let set = flat(rate);
    let mut random = XorShift::seeded(88);
    for _ in 0..2_000 {
        let gain = |random: &mut XorShift| random.below(0x1800_0001) as i32 - 0x0800_0000;
        let gains = [gain(&mut random), gain(&mut random), gain(&mut random)];
        let word = |random: &mut XorShift| (random.next_word() as i32) & !0xFF;
        let playback = [word(&mut random), word(&mut random)];
        let adc = [word(&mut random), word(&mut random)];
        let monitor = MonitorMix {
            playback: Gain::from_q5_27(gains[0]),
            inputs: [Gain::from_q5_27(gains[1]), Gain::from_q5_27(gains[2])],
        };
        if !monitor.monitors_an_input() {
            continue;
        }
        let tick_controls = controls(rate, TickMix::Bypass, &set, monitor, LIVE);
        let mut rig = Rig::new();
        let playback_in = vec![playback; frames.count()];
        let adc_in = vec![adc; frames.count()];
        rig.tick(&tick_controls, &playback_in, &adc_in);
        let out = rig.tick(&tick_controls, &playback_in, &adc_in);
        let expected = [
            reference_monitor_word(playback[0], adc, gains),
            reference_monitor_word(playback[1], adc, gains),
        ];
        assert_eq!(
            out.playback[0], expected,
            "{playback:x?} {adc:x?} {gains:x?}"
        );
    }
}

#[test]
fn with_no_input_monitored_playback_is_bit_exact_whatever_the_playback_gain_says() {
    let rate = SampleRate::Hz44100;
    let frames = rate.frames_per_tick();
    let set = flat(rate);
    let knob_at_half = MonitorMix {
        playback: HALF,
        inputs: [Gain::ZERO; 2],
    };
    let tick_controls = controls(rate, TickMix::Bypass, &set, knob_at_half, LIVE);
    let signals = stress_signals(1.0);
    let signal = &signals[2].1;
    let mut rig = Rig::new();
    let mut previous = silence(frames);
    for tick in 0..200 {
        let input = signal.tick(tick, frames);
        let out = rig.tick(&tick_controls, &input, &input);
        assert_eq!(out.playback, previous, "tick {tick}");
        previous = input;
    }
}

#[test]
fn a_muted_input_is_silent_in_capture_and_in_the_monitor_path() {
    let rate = SampleRate::Hz192000;
    let frames = rate.frames_per_tick();
    let set = flat(rate);
    let monitor = MonitorMix {
        playback: Gain::UNITY,
        inputs: [Gain::UNITY, HALF],
    };
    let muted_first = [InputMute::Muted, InputMute::Live];
    let tick_controls = controls(rate, TickMix::Bypass, &set, monitor, muted_first);
    let mut rig = Rig::new();
    let adc = vec![[0x3000_0000, 0x2000_0000]; frames.count()];
    let first = rig.tick(&tick_controls, &silence(frames), &adc);
    assert_eq!(first.capture, vec![[0, 0x2000_0000]; frames.count()]);
    let second = rig.tick(&tick_controls, &silence(frames), &adc);
    assert_eq!(second.playback, vec![[0x1000_0000; 2]; frames.count()]);
}

#[test]
fn where_the_peq_is_off_at_a_rate_the_limiter_still_runs_and_elsewhere_the_peq_is_heard() {
    for rate in SampleRate::ALL {
        let frames = rate.frames_per_tick();
        let boost = designed(&twelve_db_boosts()[1], rate);
        let flat_set = flat(rate);
        let signals = stress_signals(1.0);
        let signal = &signals[0].1;
        let mut boosted = Rig::new();
        let mut plain = Rig::new();
        let mut differs = false;
        for tick in 0..SIGNAL_FRAMES / frames.count() {
            let input = signal.tick(tick, frames);
            let a = boosted.tick(
                &controls(rate, TickMix::Processed, &boost, MonitorMix::OFF, LIVE),
                &input,
                &silence(frames),
            );
            let b = plain.tick(
                &controls(rate, TickMix::Processed, &flat_set, MonitorMix::OFF, LIVE),
                &input,
                &silence(frames),
            );
            assert_under_ceiling(&b.playback, &format!("{rate:?} tick {tick}"));
            differs |= a.playback != b.playback;
        }
        match PeqAtRate::at(rate) {
            PeqAtRate::Runs => assert!(differs, "{rate:?}: the boost is heard"),
            PeqAtRate::OffAtThisRate => assert!(!differs, "{rate:?}: the PEQ is bypassed"),
        }
    }
}

fn processed_reference(mix: TickMix) -> TickMix {
    match mix {
        TickMix::Bypass => TickMix::Bypass,
        TickMix::Blend(_) | TickMix::Processed => TickMix::Processed,
    }
}

fn output_words(words: &[[i32; 2]]) -> OutputWords {
    let mut block = [[I2sWord::from_bits(0); 2]; MAX_FRAMES];
    for (slot, frame) in block.iter_mut().zip(words) {
        *slot = frame.map(I2sWord::from_bits);
    }
    block
}

fn between(word: i32, a: i32, b: i32) -> bool {
    a.min(b) <= word && word <= a.max(b)
}

#[test]
fn dsp_switches_and_coefficient_swaps_mid_stream_stay_under_the_ceiling_and_inside_the_envelope() {
    let boosts = twelve_db_boosts();
    let both_inputs_at_half = MonitorMix {
        playback: Gain::UNITY,
        inputs: [HALF, HALF],
    };
    let signals = stress_signals(0.85);
    let microphones = &signals[3].1;
    for (rate, monitor) in SampleRate::ALL
        .into_iter()
        .flat_map(|rate| [(rate, MonitorMix::OFF), (rate, both_inputs_at_half)])
    {
        let frames = rate.frames_per_tick();
        for (name, signal) in &signals {
            let mut random = XorShift::seeded(u64::from(rate.hz()));
            let mut exchange = CoefficientExchange::flat_at(rate);
            let (mut publisher, mut coefficients) = exchange.split();
            let switch = DspModeSwitch::new();
            let mut crossfade = Crossfade::new();
            let mut switching = Rig::new();
            let mut reference = Rig::new();
            let mut bypassing = Rig::new();
            let mut blends = 0;
            let mut previous_mix = TickMix::Bypass;
            for tick in 0..3_000 {
                if random.below(40) == 0 {
                    switch.request(match random.below(2) {
                        0 => DspMode::Bypass,
                        _ => DspMode::On,
                    });
                }
                if random.below(50) == 0 {
                    publisher.change(&boosts[random.below(2) as usize], rate);
                }
                publisher.poll();
                let mix = crossfade.tick(&switch);
                let set = coefficients.adopt();
                let input = signal.tick(tick, frames);
                let adc = microphones.tick(tick + 1, frames);
                let out = switching.tick(&controls(rate, mix, set, monitor, LIVE), &input, &adc);
                let processed = reference.tick(
                    &controls(rate, processed_reference(mix), set, monitor, LIVE),
                    &input,
                    &adc,
                );
                let bypassed = bypassing.tick(
                    &controls(rate, TickMix::Bypass, set, monitor, LIVE),
                    &input,
                    &adc,
                );
                let what = format!("{rate:?} {monitor:?} {name} tick {tick} {mix:?}");
                if !monitor.monitors_an_input() {
                    assert_under_ceiling(&out.playback, &what);
                }
                match mix {
                    TickMix::Bypass => assert_eq!(out.playback, bypassed.playback, "{what}"),
                    TickMix::Processed => assert_eq!(out.playback, processed.playback, "{what}"),
                    TickMix::Blend(_) => {
                        blends += 1;
                        let expected = match previous_mix {
                            TickMix::Processed => processed.playback.clone(),
                            TickMix::Bypass | TickMix::Blend(_) => {
                                let bypass = output_words(&bypassed.playback);
                                let mut words = output_words(&processed.playback);
                                mix.apply(frames, &bypass, &mut words);
                                words[..frames.count()]
                                    .iter()
                                    .map(|f| f.map(I2sWord::to_bits))
                                    .collect()
                            }
                        };
                        assert_eq!(out.playback, expected, "{what}: the crossfade blend");
                        for ((word, bypass), processed) in out
                            .playback
                            .iter()
                            .flatten()
                            .zip(bypassed.playback.iter().flatten())
                            .zip(processed.playback.iter().flatten())
                        {
                            assert!(between(*word, *bypass, *processed), "{what}");
                        }
                    }
                }
                previous_mix = mix;
            }
            assert!(blends > 200, "{rate:?} {name}: the run crossfades often");
        }
    }
}

#[test]
fn the_peq_runs_up_to_96_khz_and_is_reported_off_at_176_4_and_192_khz() {
    use PeqAtRate::{OffAtThisRate, Runs};
    let expected = [Runs, Runs, Runs, Runs, OffAtThisRate, OffAtThisRate];
    for (rate, expected) in SampleRate::ALL.into_iter().zip(expected) {
        assert_eq!(PeqAtRate::at(rate), expected, "{rate:?}");
        assert_eq!(
            PeqAtRate::for_frames(rate.frames_per_tick()),
            expected,
            "{rate:?}"
        );
    }
}

#[test]
fn an_entry_into_on_that_skips_the_peq_still_starts_the_peq_from_silence_when_it_first_filters() {
    let filtering_rate = SampleRate::Hz96000;
    let filtering_frames = filtering_rate.frames_per_tick();
    let boost = designed(&twelve_db_boosts()[1], filtering_rate);
    let skipping_entries = [
        (
            SampleRate::Hz192000,
            designed(&twelve_db_boosts()[1], SampleRate::Hz192000),
        ),
        (filtering_rate, flat(filtering_rate)),
    ];
    let signals = stress_signals(0.5);
    let signal = &signals[3].1;
    for (entry_rate, entry_set) in &skipping_entries {
        let mut dirty = Rig::new();
        let mut fresh = Rig::new();
        for tick in 0..40 {
            dirty.tick(
                &controls(
                    filtering_rate,
                    TickMix::Processed,
                    &boost,
                    MonitorMix::OFF,
                    LIVE,
                ),
                &signal.tick(tick, filtering_frames),
                &silence(filtering_frames),
            );
        }
        for rig in [&mut dirty, &mut fresh] {
            for _ in 0..4 {
                rig.tick(
                    &controls(
                        *entry_rate,
                        TickMix::Bypass,
                        entry_set,
                        MonitorMix::OFF,
                        LIVE,
                    ),
                    &silence(entry_rate.frames_per_tick()),
                    &silence(entry_rate.frames_per_tick()),
                );
            }
            rig.tick(
                &controls(
                    *entry_rate,
                    TickMix::Processed,
                    entry_set,
                    MonitorMix::OFF,
                    LIVE,
                ),
                &silence(entry_rate.frames_per_tick()),
                &silence(entry_rate.frames_per_tick()),
            );
        }
        for tick in 0..20 {
            let on = controls(
                filtering_rate,
                TickMix::Processed,
                &boost,
                MonitorMix::OFF,
                LIVE,
            );
            let input = signal.tick(100 + tick, filtering_frames);
            let a = dirty.tick(&on, &input, &silence(filtering_frames));
            let b = fresh.tick(&on, &input, &silence(filtering_frames));
            assert_eq!(a.playback, b.playback, "{entry_rate:?} tick {tick}");
        }
    }
}
