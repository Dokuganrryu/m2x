use device::lcd::ballistics::{
    ALERT_HOLD_FRAMES, Alert, BAR_PIXELS, Decibels, Meter, MeterFrame, PEAK_HOLD_FRAMES,
    READOUT_LATCH_FRAMES,
};
use dsp::{ClippedTicks, MeterReading, PeakLevel};

const FULL_SCALE_MAGNITUDE: u32 = 0x7FFF_FFFF;

fn db(whole: i16) -> Decibels {
    Decibels::from_q8(whole * 256)
}

fn reading(magnitude: u32) -> MeterReading {
    MeterReading {
        peak: PeakLevel::from_bits(magnitude),
        clipped_ticks: ClippedTicks::from_count(0),
    }
}

fn magnitude_at(db: f64) -> u32 {
    (f64::from(FULL_SCALE_MAGNITUDE) * 10f64.powf(db / 20.0)) as u32
}

struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn attack_is_instant_the_bar_reaches_a_louder_input_in_one_frame() {
    for target in [-59, -40, -20, -6, 0] {
        let meter = Meter::SILENT.step(db(target), false);
        assert_eq!(meter.level(), db(target), "{target} dB");
        assert_eq!(meter.peak(), db(target), "{target} dB");
    }
}

#[test]
fn release_falls_twenty_decibels_in_seventy_seven_frames_plus_or_minus_one() {
    let mut meter = Meter::SILENT.step(db(0), false);
    let mut frames = 0;
    while meter.level() > db(-20) {
        meter = meter.step(Decibels::FLOOR, false);
        frames += 1;
    }
    assert!((76..=78).contains(&frames), "{frames} frames");
}

#[test]
fn the_peak_cap_holds_exactly_forty_one_frames_then_falls() {
    let mut meter = Meter::SILENT.step(db(-3), false);
    for frame in 1..=PEAK_HOLD_FRAMES {
        meter = meter.step(Decibels::FLOOR, false);
        assert_eq!(meter.peak(), db(-3), "held at frame {frame}");
    }
    meter = meter.step(Decibels::FLOOR, false);
    assert!(meter.peak() < db(-3), "falls at frame 42");
    assert_eq!(PEAK_HOLD_FRAMES, 41);
}

#[test]
fn the_cap_never_sits_below_the_bar_and_falls_monotonically_between_hits() {
    let mut random = XorShift(0x9E37_79B9_7F4A_7C15);
    let mut meter = Meter::SILENT;
    for _ in 0..200_000 {
        let roll = random.next();
        let input = if roll % 5 == 0 {
            Decibels::from_q8(-((random.next() % 15_361) as i16))
        } else {
            Decibels::FLOOR
        };
        let next = meter.step(input, false);
        assert!(next.peak() >= next.level(), "{next:?}");
        if input < meter.peak() {
            assert!(next.peak() <= meter.peak(), "{meter:?} -> {next:?}");
        }
        meter = next;
    }
}

#[test]
fn a_stale_cap_reaches_the_bar_in_finite_time() {
    let mut meter = Meter::SILENT.step(db(0), false);
    let mut frames = 0;
    while meter.peak() != meter.level() {
        meter = meter.step(Decibels::FLOOR, false);
        frames += 1;
        assert!(frames < 41 + 2 * 60, "the cap is still above the bar");
    }
}

#[test]
fn peak_words_convert_to_q8_decibels_within_three_hundredths_of_a_decibel() {
    for target in [0.0, -0.1, -3.0, -6.0206, -12.0, -20.0, -40.0, -59.0] {
        let got =
            f64::from(Decibels::of_peak(PeakLevel::from_bits(magnitude_at(target))).q8()) / 256.0;
        assert!((got - target).abs() < 0.03, "{target} dB gave {got}");
    }
    assert_eq!(Decibels::of_peak(PeakLevel::SILENCE), Decibels::FLOOR);
    assert_eq!(Decibels::of_peak(PeakLevel::from_bits(1)), Decibels::FLOOR);
}

#[test]
fn the_two_slope_scale_maps_decibels_to_bar_pixels() {
    for (whole, pixels) in [(0, 72), (-6, 59), (-12, 46), (-20, 28), (-40, 14), (-60, 0)] {
        assert_eq!(db(whole).bar_pixels(), pixels, "{whole} dB");
    }
    let mut previous = 0;
    for q8 in (-60 * 256..=0).step_by(7) {
        let pixels = Decibels::from_q8(q8).bar_pixels();
        assert!(pixels >= previous && pixels <= BAR_PIXELS);
        previous = pixels;
    }
}

#[test]
fn the_limiter_dot_lights_on_one_limited_tick_and_holds_exactly_two_seconds() {
    let quiet = [reading(magnitude_at(-30.0)); 4];
    let mut frame = MeterFrame::SILENT.next_frame(quiet, 1);
    assert_eq!(frame.output_alert(), Alert::Limiting);
    for held in 2..=ALERT_HOLD_FRAMES {
        frame = frame.next_frame(quiet, 0);
        assert_eq!(frame.output_alert(), Alert::Limiting, "frame {held}");
    }
    frame = frame.next_frame(quiet, 0);
    assert_eq!(frame.output_alert(), Alert::Quiet);
    assert_eq!(ALERT_HOLD_FRAMES, 102);
}

#[test]
fn an_output_at_or_over_minus_a_tenth_of_a_decibel_lights_the_over_dot_above_limiting() {
    let mut readings = [reading(magnitude_at(-30.0)); 4];
    readings[3] = reading(magnitude_at(-0.05));
    let frame = MeterFrame::SILENT.next_frame(readings, 5);
    assert_eq!(frame.output_alert(), Alert::Over);
    readings[3] = reading(magnitude_at(-0.5));
    assert_eq!(
        MeterFrame::SILENT.next_frame(readings, 0).output_alert(),
        Alert::Quiet
    );
}

#[test]
fn an_input_clip_lights_only_that_input_and_holds_two_seconds() {
    let mut readings = [reading(0); 4];
    readings[1].clipped_ticks = ClippedTicks::from_count(1);
    let mut frame = MeterFrame::SILENT.next_frame(readings, 0);
    assert!(!frame.meter(0).alert_lit());
    assert!(frame.meter(1).alert_lit());
    assert_eq!(frame.output_alert(), Alert::Quiet);
    for _ in 1..ALERT_HOLD_FRAMES {
        frame = frame.next_frame([reading(0); 4], 0);
    }
    assert!(frame.meter(1).alert_lit());
    frame = frame.next_frame([reading(0); 4], 0);
    assert!(!frame.meter(1).alert_lit());
}

#[test]
fn the_readout_latches_every_thirteen_frames_and_keeps_the_loudest_of_about_two_seconds() {
    let loud = {
        let mut readings = [reading(0); 4];
        readings[2] = reading(magnitude_at(-0.7));
        readings
    };
    let mut frame = MeterFrame::SILENT.next_frame(loud, 0);
    for _ in 1..READOUT_LATCH_FRAMES - 1 {
        frame = frame.next_frame([reading(0); 4], 0);
        assert_eq!(frame.readout(), Decibels::FLOOR);
    }
    frame = frame.next_frame([reading(0); 4], 0);
    assert_eq!(frame.readout().tenths(), -7);
    let mut frames = u32::from(READOUT_LATCH_FRAMES);
    while frame.readout() != Decibels::FLOOR {
        frame = frame.next_frame([reading(0); 4], 0);
        frames += 1;
        assert!(frames < 200);
    }
    assert_eq!(
        frames,
        8 * u32::from(READOUT_LATCH_FRAMES) + u32::from(READOUT_LATCH_FRAMES)
    );
}

mod obs {
    use device::lcd::ballistics::{
        Decibels, Meter, MeterFrame, OBS_RELEASE_PER_FRAME, Zone, zone_with_hysteresis,
    };
    use dsp::{ClippedTicks, MeterReading, PeakLevel};

    fn db(tenths: i32) -> Decibels {
        Decibels::from_q8((tenths * 256 / 10) as i16)
    }

    fn reading(db: f64) -> MeterReading {
        MeterReading {
            peak: PeakLevel::from_bits((f64::from(0x7FFF_FFFFu32) * 10f64.powf(db / 20.0)) as u32),
            clipped_ticks: ClippedTicks::from_count(0),
        }
    }

    #[test]
    fn obs_release_is_11_76_db_a_second_twenty_decibels_in_87_frames_plus_or_minus_one() {
        assert_eq!(OBS_RELEASE_PER_FRAME.q8(), 59);
        let mut meter = Meter::SILENT.step_with(db(0), false, OBS_RELEASE_PER_FRAME);
        let mut frames = 0;
        while meter.level() > db(-200) {
            meter = meter.step_with(Decibels::FLOOR, false, OBS_RELEASE_PER_FRAME);
            frames += 1;
        }
        assert!((86..=88).contains(&frames), "{frames} frames");
    }

    #[test]
    fn the_obs_scale_is_linear_from_minus_60_to_0_over_72_pixels_with_its_zone_rows() {
        for (tenths, pixels) in [(0, 72), (-90, 61), (-200, 48), (-400, 24), (-600, 0)] {
            assert_eq!(db(tenths).linear_pixels(), pixels, "{tenths}");
        }
        assert_eq!(72 - db(-200).linear_pixels(), 24, "-20.0 dB is row 24");
        assert_eq!(72 - db(-90).linear_pixels(), 11, "-9 dB is row 11");
    }

    #[test]
    fn the_readout_zone_moves_up_at_a_boundary_and_back_only_two_tenths_below_it() {
        assert_eq!(zone_with_hysteresis(db(-201), Zone::Green), Zone::Green);
        assert_eq!(zone_with_hysteresis(db(-200), Zone::Green), Zone::Yellow);
        assert_eq!(zone_with_hysteresis(db(-201), Zone::Yellow), Zone::Yellow);
        assert_eq!(zone_with_hysteresis(db(-202), Zone::Yellow), Zone::Yellow);
        assert_eq!(zone_with_hysteresis(db(-203), Zone::Yellow), Zone::Green);
        assert_eq!(zone_with_hysteresis(db(-90), Zone::Yellow), Zone::Red);
        assert_eq!(zone_with_hysteresis(db(-91), Zone::Red), Zone::Red);
        assert_eq!(zone_with_hysteresis(db(-93), Zone::Red), Zone::Yellow);
        assert_eq!(zone_with_hysteresis(db(-50), Zone::Green), Zone::Red);
    }

    #[test]
    fn the_bars_follow_the_tap_while_the_over_dot_follows_the_dac() {
        let bars = [reading(-30.0), reading(-30.0), reading(-1.0), reading(-1.0)];
        let dac = [reading(-0.05), reading(-8.0)];
        let frame = MeterFrame::SILENT.next_frame_tapped(bars, dac, 0, OBS_RELEASE_PER_FRAME);
        assert_eq!(frame.output_alert(), device::lcd::ballistics::Alert::Over);
        assert!(frame.meter(2).level() < Decibels::from_q8(-200));
        assert!(frame.meter(2).level() > Decibels::from_q8(-300));
    }
}
