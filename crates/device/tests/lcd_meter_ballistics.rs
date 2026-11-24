use device::lcd::meter::{
    ClipHoldFrames, FACTORY_CLIP_HOLD, METER_CHANNELS, MeterDisplay, MeterFrame, MeterLevel,
};
use dsp::{ClippedTicks, MeterChannel, MeterReading, PeakLevel};

const PEAK_SHIFT: u32 = 7;
const LAST_PEAK_INDEX: u32 = (1 << 24) - 1;

fn factory_level(peak_index: u32) -> u8 {
    if peak_index == 0 {
        return 0;
    }
    let x = f64::from(peak_index) * 2f64.powi(-24);
    let mut v = -14.0 * x.log2();
    if v > 24.0 {
        v = 12.0 + v / 2.0;
    }
    (76.0 - v).max(0.0) as u8
}

fn reading(peak_bits: u32) -> MeterReading {
    MeterReading {
        peak: PeakLevel::from_bits(peak_bits),
        clipped_ticks: ClippedTicks::from_count(0),
    }
}

fn clipped() -> MeterReading {
    MeterReading {
        peak: PeakLevel::CLIP,
        clipped_ticks: ClippedTicks::from_count(1),
    }
}

fn dbfs(decibels: f64) -> u32 {
    (10f64.powf(decibels / 20.0) * f64::from(0x7FFF_FF00u32)) as u32
}

#[test]
fn the_level_of_every_24_bit_peak_matches_the_factory_log_law() {
    let mut mismatches = Vec::new();
    for peak_index in 0..=LAST_PEAK_INDEX {
        let level = MeterLevel::of_peak(PeakLevel::from_bits(peak_index << PEAK_SHIFT)).steps();
        if level != factory_level(peak_index) {
            mismatches.push((peak_index, level, factory_level(peak_index)));
            if mismatches.len() > 8 {
                break;
            }
        }
    }
    assert!(
        mismatches.is_empty(),
        "peak index, model, factory: {mismatches:?}"
    );
}

#[test]
fn the_low_seven_bits_of_a_peak_do_not_change_its_level() {
    for peak_index in [0x8000u32, 0x0001_0000, 0x00F3_A24C] {
        let floor = MeterLevel::of_peak(PeakLevel::from_bits(peak_index << PEAK_SHIFT));
        let ceiling = MeterLevel::of_peak(PeakLevel::from_bits((peak_index << PEAK_SHIFT) | 0x7F));
        assert_eq!(floor, ceiling);
    }
}

#[test]
fn silence_shows_level_zero_and_a_full_scale_peak_shows_level_seventy_five() {
    assert_eq!(MeterLevel::of_peak(PeakLevel::SILENCE).steps(), 0);
    assert_eq!(MeterLevel::of_peak(PeakLevel::CLIP).steps(), 75);
    assert_eq!(
        MeterLevel::of_peak(PeakLevel::from_bits(u32::MAX >> 1)).steps(),
        75
    );
}

#[test]
fn the_meter_reaches_level_zero_near_minus_55_dbfs_as_the_t17_arithmetic_says() {
    assert_eq!(
        MeterLevel::of_peak(PeakLevel::from_bits(dbfs(-56.0))).steps(),
        0
    );
    assert!(MeterLevel::of_peak(PeakLevel::from_bits(dbfs(-54.0))).steps() > 0);
}

#[test]
fn a_rising_peak_shows_at_once() {
    let display = MeterDisplay::SILENT.next_frame(reading(PeakLevel::CLIP.to_bits() - 1));
    assert_eq!(display.level().steps(), 75);
}

#[test]
fn a_falling_peak_drops_one_level_per_frame_until_silence() {
    let mut display = MeterDisplay::SILENT.next_frame(reading(dbfs(-1.0)));
    let start = display.level().steps();
    for frame in 1..=u32::from(start) {
        display = display.next_frame(MeterReading::SILENCE);
        assert_eq!(
            u32::from(display.level().steps()),
            u32::from(start) - frame,
            "frame {frame}"
        );
    }
    display = display.next_frame(MeterReading::SILENCE);
    assert_eq!(display.level().steps(), 0);
}

#[test]
fn a_peak_below_the_falling_bar_does_not_pull_it_down_faster() {
    let high = MeterDisplay::SILENT.next_frame(reading(dbfs(-1.0)));
    let next = high.next_frame(reading(dbfs(-30.0)));
    assert_eq!(next.level().steps(), high.level().steps() - 1);
}

#[test]
fn a_clipped_peak_lights_the_clip_box_for_exactly_ninety_frames() {
    assert_eq!(FACTORY_CLIP_HOLD, ClipHoldFrames::new(90));
    let mut display = MeterDisplay::SILENT.next_frame(clipped());
    let mut lit_frames = 0;
    while display.clip_lit() {
        lit_frames += 1;
        display = display.next_frame(MeterReading::SILENCE);
        assert!(lit_frames <= 90, "the clip box stayed lit past 90 frames");
    }
    assert_eq!(lit_frames, 90);
}

#[test]
fn a_new_clip_during_the_hold_restarts_the_ninety_frames() {
    let mut display = MeterDisplay::SILENT.next_frame(clipped());
    for _ in 0..50 {
        display = display.next_frame(MeterReading::SILENCE);
    }
    display = display.next_frame(clipped());
    for _ in 0..89 {
        display = display.next_frame(MeterReading::SILENCE);
        assert!(display.clip_lit());
    }
    display = display.next_frame(MeterReading::SILENCE);
    assert!(!display.clip_lit());
}

#[test]
fn a_clipped_tick_count_lights_the_clip_box_even_when_the_peak_swap_missed_it() {
    let reading = MeterReading {
        peak: PeakLevel::SILENCE,
        clipped_ticks: ClippedTicks::from_count(1),
    };
    assert!(MeterDisplay::SILENT.next_frame(reading).clip_lit());
}

#[test]
fn a_peak_just_below_the_factory_threshold_never_lights_the_clip_box() {
    let display = MeterDisplay::SILENT.next_frame(reading(PeakLevel::CLIP.to_bits() - 1));
    assert!(!display.clip_lit());
}

#[test]
fn a_meter_frame_advances_each_channel_from_its_own_reading() {
    assert_eq!(
        METER_CHANNELS,
        [
            MeterChannel::Input1,
            MeterChannel::Input2,
            MeterChannel::OutputLeft,
            MeterChannel::OutputRight
        ]
    );
    let frame = MeterFrame::SILENT.next_frame([
        reading(dbfs(-1.0)),
        MeterReading::SILENCE,
        clipped(),
        reading(dbfs(-20.0)),
    ]);
    assert!(frame.channel(MeterChannel::Input1).level().steps() > 70);
    assert_eq!(frame.channel(MeterChannel::Input2), MeterDisplay::SILENT);
    assert!(frame.channel(MeterChannel::OutputLeft).clip_lit());
    assert!(!frame.channel(MeterChannel::OutputRight).clip_lit());
}
