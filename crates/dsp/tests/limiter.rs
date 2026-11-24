use dsp::{
    Ceiling, DspSample, Frame, FramesPerTick, Limiter, LimiterCoefficients, LimiterSettings,
    MAX_FRAMES, RateFamily, ReleaseTime,
};

#[test]
fn ceiling_accepts_minus_twenty_to_zero_dbfs_in_tenths_and_rejects_the_rest() {
    assert!(Ceiling::from_tenths_of_dbfs(-200).is_ok());
    assert!(Ceiling::from_tenths_of_dbfs(0).is_ok());
    assert!(Ceiling::from_tenths_of_dbfs(-201).is_err());
    assert!(Ceiling::from_tenths_of_dbfs(1).is_err());
    assert_eq!(
        Ceiling::default(),
        Ceiling::from_tenths_of_dbfs(-10).unwrap()
    );
}

#[test]
fn release_accepts_five_milliseconds_to_one_second_and_rejects_the_rest() {
    assert!(ReleaseTime::from_millis(5).is_ok());
    assert!(ReleaseTime::from_millis(1000).is_ok());
    assert!(ReleaseTime::from_millis(4).is_err());
    assert!(ReleaseTime::from_millis(1001).is_err());
    assert_eq!(
        ReleaseTime::default(),
        ReleaseTime::from_millis(50).unwrap()
    );
}

#[test]
fn ceiling_level_is_the_q2_29_sample_peak_rounded_down() {
    let level = |tenths| {
        let settings = LimiterSettings {
            ceiling: Ceiling::from_tenths_of_dbfs(tenths).unwrap(),
            release: ReleaseTime::default(),
        };
        LimiterCoefficients::design(settings, RateFamily::FortyEightKilohertz)
            .ceiling()
            .to_q2_29()
    };
    assert_eq!(level(0), 1 << 29);
    assert_eq!(level(-1), 530_725_401);
    assert_eq!(level(-10), 478_486_703);
    assert_eq!(level(-200), 53_687_091);
}

const ALL_TICK_SHAPES: [(RateFamily, FramesPerTick); 6] = [
    (RateFamily::FortyFourPointOneKilohertz, FramesPerTick::Six),
    (RateFamily::FortyEightKilohertz, FramesPerTick::Six),
    (
        RateFamily::FortyFourPointOneKilohertz,
        FramesPerTick::Twelve,
    ),
    (RateFamily::FortyEightKilohertz, FramesPerTick::Twelve),
    (
        RateFamily::FortyFourPointOneKilohertz,
        FramesPerTick::TwentyFour,
    ),
    (RateFamily::FortyEightKilohertz, FramesPerTick::TwentyFour),
];

const STREAM_LSB_IN_Q2_29: i32 = 1 << 6;
const LOUDEST_STREAM_LEVEL_UNDER_DEFAULT_CEILING: i32 = 478_486_656;

type Block = [Frame<DspSample>; MAX_FRAMES];

fn block_of(raw: impl Fn(usize, usize) -> i32) -> Block {
    core::array::from_fn(|frame| core::array::from_fn(|ch| DspSample::from_q2_29(raw(frame, ch))))
}

fn raw_frames(block: &Block, frames: FramesPerTick) -> Vec<[i32; 2]> {
    block[..frames.count()]
        .iter()
        .map(|f| f.map(DspSample::to_q2_29))
        .collect()
}

fn run_tick(limiter: &mut Limiter, frames: FramesPerTick, input: Block) -> Block {
    *limiter.intake() = input;
    *limiter.process(frames)
}

fn default_limiter(family: RateFamily) -> Limiter {
    Limiter::new(LimiterCoefficients::design(
        LimiterSettings::default(),
        family,
    ))
}

#[test]
fn a_stream_signal_under_the_ceiling_leaves_bit_exact_exactly_one_tick_later() {
    for (family, frames) in ALL_TICK_SHAPES {
        let mut limiter = default_limiter(family);
        let mut previous = vec![[0, 0]; frames.count()];
        for tick in 0..50 {
            let input = block_of(|frame, ch| {
                let swing = LOUDEST_STREAM_LEVEL_UNDER_DEFAULT_CEILING
                    - STREAM_LSB_IN_Q2_29 * (tick * 24 + frame) as i32;
                if (frame + ch + tick) % 2 == 0 {
                    swing
                } else {
                    -swing
                }
            });
            let mut block = input;
            block = run_tick(&mut limiter, frames, block);
            assert_eq!(
                raw_frames(&block, frames),
                previous,
                "tick {tick} {frames:?}"
            );
            previous = raw_frames(&input, frames);
        }
    }
}

struct RepeatableNoise(u64);

impl RepeatableNoise {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn full_range_word(&mut self) -> i32 {
        (self.next() >> 32) as i32
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

const TWELVE_DB_OVER_FULL_SCALE: f64 = i32::MAX as f64;
const STRESS_TICKS: usize = 400;

fn stress_vectors() -> Vec<(&'static str, Vec<i32>)> {
    let length = STRESS_TICKS * MAX_FRAMES;
    let mut noise = RepeatableNoise(0x9E37_79B9_7F4A_7C15);
    let sweep = (0..length)
        .map(|n| {
            let progress = n as f64 / length as f64;
            let phase = 2.0 * core::f64::consts::PI * 0.0005 * length as f64 * progress.powi(3);
            (TWELVE_DB_OVER_FULL_SCALE * phase.sin()) as i32
        })
        .collect();
    let impulses = (0..length)
        .map(|n| match n % 97 {
            0 => i32::MAX,
            48 => i32::MIN,
            _ => 0,
        })
        .collect();
    let squares = (0..length)
        .map(|n| {
            let half_period = 1 + (n / 1000) % 37;
            if (n / half_period) % 2 == 0 {
                i32::MAX
            } else {
                i32::MIN
            }
        })
        .collect();
    let white = (0..length).map(|_| noise.full_range_word()).collect();
    let quiet_then_loud = (0..length)
        .map(|n| {
            let loud = (n / 333) % 2 == 1;
            let sign = if n % 2 == 0 { 1 } else { -1 };
            if loud { sign * i32::MAX } else { sign * 1000 }
        })
        .collect();
    vec![
        ("sweep", sweep),
        ("impulses", impulses),
        ("squares", squares),
        ("white noise", white),
        ("quiet then loud", quiet_then_loud),
    ]
}

fn stereo_block(signal: &[i32], tick: usize, frames: FramesPerTick) -> Block {
    let start = tick * frames.count();
    block_of(|frame, ch| {
        let index = (start + frame) % signal.len();
        if ch == 0 {
            signal[index]
        } else {
            signal[signal.len() - 1 - index].wrapping_neg()
        }
    })
}

fn settings(ceiling_tenths: i16, release_millis: u16) -> LimiterSettings {
    LimiterSettings {
        ceiling: Ceiling::from_tenths_of_dbfs(ceiling_tenths).unwrap(),
        release: ReleaseTime::from_millis(release_millis).unwrap(),
    }
}

fn assert_under_ceiling(block: &Block, frames: FramesPerTick, ceiling: DspSample, context: &str) {
    for (frame, samples) in block[..frames.count()].iter().enumerate() {
        for sample in samples {
            assert!(
                sample.to_q2_29().unsigned_abs() <= ceiling.to_q2_29() as u32,
                "{context} frame {frame}: {} over ceiling {}",
                sample.to_q2_29(),
                ceiling.to_q2_29()
            );
        }
    }
}

#[test]
fn every_output_sample_of_every_stress_vector_stays_at_or_under_the_ceiling() {
    for (family, frames) in ALL_TICK_SHAPES {
        for ceiling_tenths in [-200, -60, -10, -1, 0] {
            for release_millis in [5, 50, 1000] {
                let coefficients =
                    LimiterCoefficients::design(settings(ceiling_tenths, release_millis), family);
                let ceiling = coefficients.ceiling().to_q2_29();
                let hovering_over_the_ceiling = (0..STRESS_TICKS * MAX_FRAMES)
                    .map(|n| ceiling - 1024 + (n as i32 / 37 % 97) * 64)
                    .collect();
                let mut vectors = stress_vectors();
                vectors.push(("hovering over the ceiling", hovering_over_the_ceiling));
                for (name, signal) in &vectors {
                    let mut limiter = Limiter::new(coefficients);
                    for tick in 0..STRESS_TICKS {
                        let mut block = stereo_block(signal, tick, frames);
                        block = run_tick(&mut limiter, frames, block);
                        let context = format!(
                            "{name} {family:?} {frames:?} {ceiling_tenths} {release_millis} ms tick {tick}"
                        );
                        assert_under_ceiling(&block, frames, coefficients.ceiling(), &context);
                    }
                }
            }
        }
    }
}

#[test]
fn peaks_where_a_reciprocal_seed_is_almost_exact_are_still_held_under_the_ceiling() {
    let seed_buckets = 256u64;
    let exact_peaks: Vec<i64> = (0..seed_buckets)
        .flat_map(|bucket| {
            let seed = (1u64 << 40) / (2 * seed_buckets + 2 * bucket + 1);
            let mantissa = ((1u128 << 62) + seed as u128 / 2) / seed as u128;
            (0..=3).map(move |shift| (mantissa >> shift) as i64)
        })
        .flat_map(|peak| (-3..=3).map(move |offset| peak + offset))
        .filter(|peak| (1..=i32::MAX as i64).contains(peak))
        .collect();
    for (family, frames) in ALL_TICK_SHAPES {
        for ceiling_tenths in [-200, -137, -60, -10, -1, 0] {
            let coefficients = LimiterCoefficients::design(settings(ceiling_tenths, 5), family);
            for &peak in &exact_peaks {
                let peak = peak as i32;
                let mut limiter = Limiter::new(coefficients);
                for tick in 0..3 {
                    let mut block =
                        block_of(|frame, ch| if (frame + ch) % 2 == 0 { peak } else { -peak });
                    block = run_tick(&mut limiter, frames, block);
                    let context =
                        format!("{family:?} {frames:?} {ceiling_tenths} peak {peak} tick {tick}");
                    assert_under_ceiling(&block, frames, coefficients.ceiling(), &context);
                }
            }
        }
    }
}

#[test]
fn a_held_overload_is_limited_to_within_five_parts_per_million_of_the_ceiling() {
    for (family, frames) in ALL_TICK_SHAPES {
        for ceiling_tenths in [-200, -10, 0] {
            let coefficients = LimiterCoefficients::design(settings(ceiling_tenths, 50), family);
            let ceiling = coefficients.ceiling().to_q2_29() as f64;
            for overload in [ceiling as i32 + 1, 700_000_000, i32::MAX] {
                let mut limiter = Limiter::new(coefficients);
                let mut loudest = 0;
                for _ in 0..20 {
                    let mut block =
                        block_of(|frame, _| if frame % 2 == 0 { overload } else { -overload });
                    block = run_tick(&mut limiter, frames, block);
                    loudest = block[..frames.count()]
                        .iter()
                        .flatten()
                        .map(|s| s.to_q2_29().unsigned_abs())
                        .max()
                        .unwrap();
                }
                let shortfall = (ceiling - loudest as f64) / ceiling;
                assert!(
                    (0.0..5e-6).contains(&shortfall),
                    "{family:?} {frames:?} {ceiling_tenths} overload {overload} loudest {loudest} shortfall {shortfall}"
                );
            }
        }
    }
}

#[test]
fn after_an_overload_the_gain_closes_the_gap_to_unity_by_the_release_time_constant() {
    for (family, frames) in ALL_TICK_SHAPES {
        for release_millis in [5, 50, 1000] {
            let coefficients = LimiterCoefficients::design(settings(-10, release_millis), family);
            let ceiling = coefficients.ceiling().to_q2_29();
            let mut limiter = Limiter::new(coefficients);
            let mut loud = block_of(|_, _| 2 * ceiling);
            loud = run_tick(&mut limiter, frames, loud);
            let quiet_level = ceiling / 2;
            let ticks_per_time_constant =
                release_millis as f64 * family.ticks_per_second() as f64 / 1000.0;
            let ticks = ticks_per_time_constant.round() as usize;
            let mut last_gain = 0.0;
            for _ in 0..=ticks + 1 {
                let mut quiet = block_of(|_, _| quiet_level);
                quiet = run_tick(&mut limiter, frames, quiet);
                last_gain = quiet[frames.count() - 1][0].to_q2_29() as f64 / quiet_level as f64;
            }
            let start_gap = 0.5;
            let expected_gap = start_gap * (-(ticks as f64) / ticks_per_time_constant).exp();
            let gap = 1.0 - last_gain;
            assert!(
                (gap - expected_gap).abs() < 0.01 * start_gap,
                "{family:?} {frames:?} {release_millis} ms gap {gap} expected {expected_gap}"
            );
        }
    }
}

#[test]
fn after_an_overload_a_quiet_signal_returns_to_bit_exact() {
    for (family, frames) in ALL_TICK_SHAPES {
        let mut limiter = Limiter::new(LimiterCoefficients::design(settings(-10, 5), family));
        let mut loud = block_of(|_, _| i32::MAX);
        loud = run_tick(&mut limiter, frames, loud);
        let quiet =
            block_of(|frame, ch| STREAM_LSB_IN_Q2_29 * (1_500_000 - 1000 * (frame + ch) as i32));
        let mut block = quiet;
        for _ in 0..2000 {
            block = quiet;
            block = run_tick(&mut limiter, frames, block);
        }
        assert_eq!(raw_frames(&block, frames), raw_frames(&quiet, frames));
    }
}

#[test]
fn retuning_at_random_ticks_never_lets_a_sample_over_the_current_ceiling() {
    let mut noise = RepeatableNoise(0x2545_F491_4F6C_DD1D);
    let vectors = stress_vectors();
    for (family, frames) in ALL_TICK_SHAPES {
        for (name, signal) in &vectors {
            let mut coefficients = LimiterCoefficients::design(LimiterSettings::default(), family);
            let mut limiter = Limiter::new(coefficients);
            for tick in 0..STRESS_TICKS {
                if noise.below(8) == 0 {
                    let ceiling_tenths = -(noise.below(201) as i16);
                    let release_millis = 5 + noise.below(996) as u16;
                    coefficients = LimiterCoefficients::design(
                        settings(ceiling_tenths, release_millis),
                        family,
                    );
                    limiter.retune(coefficients);
                }
                let mut block = stereo_block(signal, tick, frames);
                block = run_tick(&mut limiter, frames, block);
                let context = format!("{name} {family:?} {frames:?} tick {tick}");
                assert_under_ceiling(&block, frames, coefficients.ceiling(), &context);
            }
        }
    }
}

const BOOT_DESIGN: LimiterCoefficients =
    LimiterCoefficients::design(LimiterSettings::DEFAULT, RateFamily::FortyEightKilohertz);

#[test]
fn the_default_settings_constant_is_the_derived_default() {
    assert_eq!(LimiterSettings::DEFAULT, LimiterSettings::default());
}

#[test]
fn a_limiter_designed_at_compile_time_equals_one_designed_at_run_time() {
    for family in [
        RateFamily::FortyEightKilohertz,
        RateFamily::FortyFourPointOneKilohertz,
    ] {
        let at_run_time = LimiterCoefficients::design(LimiterSettings::default(), family);
        let at_compile_time = match family {
            RateFamily::FortyEightKilohertz => BOOT_DESIGN,
            RateFamily::FortyFourPointOneKilohertz => {
                const {
                    LimiterCoefficients::design(
                        LimiterSettings::DEFAULT,
                        RateFamily::FortyFourPointOneKilohertz,
                    )
                }
            }
        };
        assert_eq!(at_compile_time, at_run_time);
    }
}
