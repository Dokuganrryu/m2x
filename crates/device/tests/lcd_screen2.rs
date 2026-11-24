#![cfg(feature = "screen2")]

mod console_support;

use console_support::typed;
use device::breadcrumb::{Breadcrumb, Place, Weight};
use device::console::{ConsoleCommand, Output, Refusal};
use device::control_pass::Monitoring;
use device::fault::FaultCode;
use device::lcd::ballistics::MeterFrame;
use device::lcd::panel::{PANEL_COLUMNS, PANEL_ROWS};
use device::lcd::screen::{
    BACKGROUND, Batch, ClockLock, DspBadge, FAULT_BACKGROUND, FAULT_WINDOW, Glass,
    MAX_REGION_PIXELS, MOST_BATCH_DESCRIPTORS, MOST_LCD_DESCRIPTORS, MOST_PIECES, MeterProfileName,
    PILL_EASE, PillFade, Region, RegionPixels, RegionSet, Rgb565, Screen, StreamActivity,
    render_batch, render_fault,
};
use device::lcd::wire::{MOST_TRANSFERS_PER_DESCRIPTOR, PanelWord, descriptor_chunks};
use device::phantom::PhantomPower;
use dsp::{ClippedTicks, MeterReading, PeakLevel, SampleRate};

const WIDTH: usize = PANEL_ROWS as usize;
const HEIGHT: usize = PANEL_COLUMNS as usize;
const EOT_HALF: u32 = 0x0F5E;
const PIXEL_HALF: u32 = 0x0F4E;

const CLARITY: u16 = 0x069F;
const WARM: u16 = 0xFE28;
const RAW: u16 = 0xE71C;
const GHOST: u16 = 0x18C3;
const SIGNAL: u16 = 0x1F0E;
const HOT: u16 = 0xFD84;
const OVER: u16 = 0xF986;
const PEAK_CAP: u16 = 0xF7BE;
const PHANTOM_ON: u16 = 0xF945;
const WHITE: u16 = 0xFFFF;
const CHIP_LINE: u16 = 0x2945;
const DOT_RING: u16 = 0x3186;
const SCALE_TICK: u16 = 0x39C7;

fn running() -> Breadcrumb {
    Breadcrumb {
        place: Place { stage: 16, step: 0 },
        code: 0,
        weight: Weight::Noted,
        latest: 16,
    }
}

fn screen(dsp: DspBadge) -> Screen {
    let mut screen = Screen {
        rate: SampleRate::Hz48000,
        clock: ClockLock::Locked,
        playback: StreamActivity::Idle,
        capture: StreamActivity::Idle,
        phantom: [PhantomPower::Off; 2],
        monitoring: [Monitoring::Off; 2],
        meters: MeterFrame::SILENT,
        dsp,
        pill: PillFade::AT_BOOT,
        profile: MeterProfileName::Studio,
        reduced_bands: None,
        breadcrumb: running(),
        previous_boot: None,
    };
    screen.pill = screen.pill.next_frame(screen.pill_target(), false);
    screen
}

struct Glass2 {
    pixels: Vec<u16>,
    glass: Glass,
    store: RegionPixels,
}

impl Glass2 {
    fn new() -> Self {
        Self {
            pixels: vec![0xDEAD; WIDTH * HEIGHT],
            glass: Glass::BLANK,
            store: RegionPixels::new(),
        }
    }

    fn at(&self, x: usize, y: usize) -> u16 {
        self.pixels[y * WIDTH + x]
    }

    fn send(&mut self, dirty: RegionSet, screen: &Screen) -> Batch {
        let batch = render_batch(dirty, screen, &mut self.glass, &mut self.store);
        let mut descriptors = 0;
        let mut pixels = 0;
        for piece in batch.pieces.iter().flatten() {
            let words = self.store.words(*piece);
            let (last, rest) = words.split_last().expect("a piece has pixels");
            assert_eq!(last.bits() >> 16, EOT_HALF, "only the last word ends");
            assert!(rest.iter().all(|word| word.bits() >> 16 == PIXEL_HALF));
            let window = piece.window;
            let height = usize::from(window.columns());
            let top = HEIGHT - usize::from(window.first_column()) - height;
            for (index, word) in words.iter().enumerate() {
                let x = usize::from(window.first_row()) + index / height;
                let y = top + height - 1 - index % height;
                self.pixels[y * WIDTH + x] = word.colour().bits();
            }
            descriptors += 1 + descriptor_chunks(words.len()).count();
            pixels += words.len();
        }
        assert!(batch.count() <= MOST_PIECES);
        assert!(pixels <= MAX_REGION_PIXELS);
        assert!(
            descriptors <= MOST_LCD_DESCRIPTORS,
            "{descriptors} descriptors"
        );
        batch
    }

    fn draw_all(&mut self, screen: &Screen) -> usize {
        let mut dirty = RegionSet::ALL;
        let mut batches = 0;
        while !dirty.is_empty() {
            dirty = self.send(dirty, screen).leftover;
            batches += 1;
            assert!(batches < 10, "a full draw never finishes");
        }
        batches
    }
}

fn cleared() -> Glass2 {
    let mut glass = Glass2::new();
    glass.pixels.fill(BACKGROUND.bits());
    glass
}

fn reading(db: f64) -> MeterReading {
    MeterReading {
        peak: PeakLevel::from_bits((f64::from(0x7FFF_FFFFu32) * 10f64.powf(db / 20.0)) as u32),
        clipped_ticks: ClippedTicks::from_count(0),
    }
}

#[test]
fn the_ground_is_true_black_and_the_wire_swaps_red_and_blue() {
    assert_eq!(BACKGROUND, Rgb565::from_bits(0x0000));
    assert_eq!(
        PanelWord::pixel(Rgb565::from_bits(CLARITY)).bits() & 0xFFFF,
        0xFE80
    );
    assert_eq!(
        PanelWord::pixel(Rgb565::from_bits(WARM)).bits() & 0xFFFF,
        0x463F
    );
    assert_eq!(
        PanelWord::pixel(Rgb565::from_bits(RAW)).bits() & 0xFFFF,
        RAW.into()
    );
}

#[test]
fn the_arena_is_2560_words_and_no_batch_can_pass_24_descriptors() {
    assert_eq!(MAX_REGION_PIXELS, 2_560);
    assert_eq!(MOST_PIECES, 8);
    const { assert!(MOST_BATCH_DESCRIPTORS <= MOST_LCD_DESCRIPTORS) };
    assert_eq!(MOST_LCD_DESCRIPTORS, 24);
    let worst: usize = (0..MOST_PIECES)
        .map(|piece| {
            let words = if piece == 0 {
                MAX_REGION_PIXELS - (MOST_PIECES - 1)
            } else {
                1
            };
            1 + words.div_ceil(MOST_TRANSFERS_PER_DESCRIPTOR)
        })
        .sum();
    assert!(worst <= MOST_LCD_DESCRIPTORS, "{worst}");
}

#[test]
fn a_full_draw_covers_every_region_once_in_a_few_batches_within_the_arena() {
    let mut glass = Glass2::new();
    let batches = glass.draw_all(&screen(DspBadge::Clarity));
    assert!((2..=6).contains(&batches), "{batches} batches");
    assert_eq!(glass.at(0, 0), 0xDEAD, "the margin is never redrawn");
    assert_eq!(
        glass.at(3, 2),
        BACKGROUND.bits(),
        "the pill corner is ground"
    );
}

#[test]
fn the_preset_pill_is_cyan_for_clarity_gold_for_the_warm_and_an_outline_for_raw() {
    for (badge, fill, line) in [
        (DspBadge::Clarity, CLARITY, CLARITY),
        (DspBadge::Warm, WARM, WARM),
        (DspBadge::Raw, 0x0000, RAW),
    ] {
        let mut glass = cleared();
        glass.draw_all(&screen(badge));
        assert_eq!(glass.at(5, 2), line, "{badge:?} top edge");
        assert_eq!(glass.at(3, 11), line, "{badge:?} left edge");
        assert_eq!(glass.at(6, 11), fill, "{badge:?} inside");
        assert_eq!(glass.at(3, 2), 0x0000, "{badge:?} rounded corner");
    }
}

#[test]
fn the_pill_label_changes_on_frame_zero_and_its_colour_eases_over_eight_frames() {
    let eq1 = screen(DspBadge::Clarity);
    let mut fade = eq1.pill;
    let eq2 = Screen {
        dsp: DspBadge::Warm,
        ..eq1
    };
    let mut fills = Vec::new();
    for _ in 0..PILL_EASE.len() + 2 {
        fade = fade.next_frame(eq2.pill_target(), true);
        assert_eq!(fade.label(), b"EQ2");
        fills.push(fade.colours().fill().bits());
    }
    assert_ne!(fills[0], CLARITY);
    assert_ne!(fills[0], WARM);
    assert_eq!(fills[PILL_EASE.len() - 1], WARM);
    assert_eq!(fills[PILL_EASE.len()], WARM);
    assert!(fade.settled());
    let blue = |bits: u16| bits & 0x1F;
    assert!(fills.windows(2).all(|pair| blue(pair[1]) <= blue(pair[0])));
}

#[test]
fn a_second_switch_mid_fade_retargets_from_the_colour_on_the_glass() {
    let eq1 = screen(DspBadge::Clarity);
    let eq2 = Screen {
        dsp: DspBadge::Warm,
        ..eq1
    };
    let mut fade = eq1.pill.next_frame(eq2.pill_target(), true);
    fade = fade.next_frame(eq2.pill_target(), true);
    let shown = fade.colours().fill();
    fade = fade.next_frame(eq1.pill_target(), true);
    assert_eq!(fade.label(), b"EQ1");
    let first = fade.colours().fill();
    assert_ne!(first, shown);
    assert_ne!(first.bits(), CLARITY);
    let gap_before = (i32::from(shown.bits() & 0x1F) - 0x1F).abs();
    let gap_after = (i32::from(first.bits() & 0x1F) - 0x1F).abs();
    assert!(gap_after < gap_before);
}

#[test]
fn with_lcd_motion_off_the_pill_switches_in_one_frame() {
    let eq1 = screen(DspBadge::Clarity);
    let eq2 = Screen {
        dsp: DspBadge::Warm,
        ..eq1
    };
    let fade = eq1.pill.next_frame(eq2.pill_target(), false);
    assert_eq!(fade.colours().fill().bits(), WARM);
    assert!(fade.settled());
}

#[test]
fn lcd_motion_parses_on_off_and_a_bare_query_and_refuses_anything_else() {
    let one = |line: &str| typed(format!("{line}\n").as_bytes());
    assert_eq!(
        one("lcd motion off"),
        [Output::Execute(ConsoleCommand::LcdMotion(Some(false)))]
    );
    assert_eq!(
        one("lcd motion on"),
        [Output::Execute(ConsoleCommand::LcdMotion(Some(true)))]
    );
    assert_eq!(
        one("lcd motion"),
        [Output::Execute(ConsoleCommand::LcdMotion(None))]
    );
    assert_eq!(
        one("lcd motion fast"),
        [Output::Reject(Refusal::MalformedArguments)]
    );
    assert_eq!(one("lcd"), [Output::Reject(Refusal::NotAllowlisted)]);
    assert_eq!(
        one("lcd motion on now"),
        [Output::Reject(Refusal::MalformedArguments)]
    );
    assert!(ConsoleCommand::LcdMotion(None).only_reads());
    assert!(!ConsoleCommand::LcdMotion(Some(false)).only_reads());
}

#[test]
fn chips_show_48v_in_red_monitoring_in_white_and_ghost_when_off() {
    let mut on = screen(DspBadge::Clarity);
    on.phantom = [PhantomPower::On, PhantomPower::Off];
    on.monitoring = [Monitoring::Off, Monitoring::On];
    let mut glass = cleared();
    glass.draw_all(&on);
    assert_eq!(glass.at(4, 115), PHANTOM_ON);
    assert_eq!(glass.at(42, 120), CHIP_LINE, "48V 2 off is a line");
    assert_eq!(glass.at(44, 116), 0x0000, "48V 2 off is hollow");
    assert_eq!(glass.at(121, 115), WHITE, "MON 2 on is filled white");
    assert_eq!(glass.at(81, 120), CHIP_LINE);
}

#[test]
fn a_phantom_change_redraws_only_its_chip() {
    let quiet = screen(DspBadge::Clarity);
    let mut glass = cleared();
    glass.draw_all(&quiet);
    let mut on = quiet;
    on.phantom[0] = PhantomPower::On;
    let dirty = on.regions_changed_from(&quiet);
    assert_eq!(dirty, RegionSet::of(Region::PhantomChip1));
    let batch = glass.send(dirty, &on);
    assert_eq!(batch.count(), 1);
    assert_eq!(batch.pixels(), 37 * 14);
    assert_eq!(glass.at(4, 115), PHANTOM_ON);
}

#[test]
fn silent_bars_show_the_ghost_ladder_and_the_scale_ticks() {
    let mut glass = cleared();
    glass.draw_all(&screen(DspBadge::Clarity));
    assert_eq!(glass.at(48, 101), GHOST, "bottom segment");
    assert_eq!(glass.at(48, 100), 0x0000, "gap row");
    assert_eq!(glass.at(3, 101), GHOST, "IN 1");
    assert_eq!(glass.at(44, 31), SCALE_TICK, "0 dB tick");
    assert_eq!(glass.at(45, 75), SCALE_TICK, "-20 dB tick");
    assert_eq!(glass.at(72, 25), DOT_RING, "the dot ring");
    assert_eq!(glass.at(74, 25), 0x0000, "the dot is hollow when quiet");
}

#[test]
fn a_full_scale_bar_runs_green_amber_red_with_a_white_peak_cap() {
    let mut loud = screen(DspBadge::Clarity);
    loud.meters = loud.meters.next_frame([reading(-0.2); 4], 0);
    let mut glass = cleared();
    glass.draw_all(&loud);
    assert_eq!(glass.at(60, 32), PEAK_CAP, "the cap sits on top");
    assert_eq!(glass.at(60, 35), OVER, "red zone");
    assert_eq!(glass.at(60, 45), HOT, "amber zone");
    assert_eq!(glass.at(60, 90), SIGNAL, "green zone");
    assert_eq!(glass.at(60, 100), 0x0000, "segment gap");
}

#[test]
fn a_one_segment_change_sends_a_few_rows_of_one_bar_not_the_whole_bar() {
    let mut music = screen(DspBadge::Clarity);
    music.meters = music.meters.next_frame([reading(-18.0); 4], 0);
    let mut glass = cleared();
    glass.draw_all(&music);
    let mut next = music;
    let mut readings = [reading(-18.0); 4];
    readings[2] = reading(-17.0);
    next.meters = music.meters.next_frame(readings, 0);
    let dirty = next.regions_changed_from(&music);
    assert!(dirty.contains(Region::OutputLeft));
    let batch = glass.send(dirty, &next);
    let left_rows = batch
        .pieces
        .iter()
        .flatten()
        .find(|piece| piece.window.first_row() == 48)
        .map(|piece| piece.window.columns())
        .expect("OUT L is sent");
    assert!(left_rows <= 6, "{left_rows} rows");
    let mut full = cleared();
    full.draw_all(&next);
    assert_eq!(
        glass.pixels, full.pixels,
        "the partial update leaves the same glass"
    );
}

#[test]
fn the_limiter_dot_fills_amber_and_the_over_dot_red() {
    let mut limiting = screen(DspBadge::Clarity);
    limiting.meters = limiting.meters.next_frame([reading(-6.0); 4], 3);
    let mut glass = cleared();
    glass.draw_all(&limiting);
    assert_eq!(glass.at(74, 25), HOT);
    let mut over = limiting;
    over.meters = over.meters.next_frame([reading(-0.01); 4], 0);
    let dirty = over.regions_changed_from(&limiting);
    assert!(dirty.contains(Region::Dot));
    glass.send(dirty, &over);
    assert_eq!(glass.at(74, 25), OVER);
}

#[test]
fn the_compact_fault_tile_fits_the_arena_and_ends_its_transfer() {
    let mut store = RegionPixels::new();
    let pixels = render_fault(FaultCode::ALL[0], 7, &mut store);
    assert_eq!(pixels.len(), FAULT_WINDOW.pixel_count());
    assert!(pixels.len() <= MAX_REGION_PIXELS);
    let (last, rest) = pixels.split_last().expect("pixels");
    assert_eq!(last.bits() >> 16, EOT_HALF);
    assert!(rest.iter().all(|word| word.bits() >> 16 == PIXEL_HALF));
    assert!(pixels.iter().any(|word| word.colour() == FAULT_BACKGROUND));
    assert!(pixels.iter().any(|word| word.colour().bits() == WHITE));
}

#[test]
fn before_running_the_pill_is_a_dim_outline_and_the_readout_shows_the_boot_stage() {
    let mut booting = screen(DspBadge::Clarity);
    booting.breadcrumb = Breadcrumb::START;
    booting.pill = PillFade::AT_BOOT.next_frame(booting.pill_target(), false);
    let mut glass = cleared();
    glass.draw_all(&booting);
    assert_eq!(glass.at(5, 2), CHIP_LINE);
    assert_eq!(glass.at(6, 11), 0x0000);
    let readout_lit = (106..157)
        .flat_map(|x| (46..71).map(move |y| (x, y)))
        .any(|(x, y)| glass.at(x, y) == WHITE);
    assert!(readout_lit, "the S stage line is drawn");
}

#[cfg(feature = "sound3")]
mod sound3_console {
    use super::console_support::typed;
    use device::console::{ConsoleCommand, Output, Refusal, Report};
    use device::lcd::screen::MeterProfileName;

    fn one(line: &str) -> Vec<Output> {
        typed(format!("{line}\n").as_bytes())
    }

    #[test]
    fn meter_obs_and_studio_switch_the_profile_and_meter_alone_reports_it() {
        assert_eq!(
            one("meter obs"),
            [Output::Execute(ConsoleCommand::MeterProfile(Some(
                MeterProfileName::Obs
            )))]
        );
        assert_eq!(
            one("meter studio"),
            [Output::Execute(ConsoleCommand::MeterProfile(Some(
                MeterProfileName::Studio
            )))]
        );
        assert_eq!(
            one("meter"),
            [Output::Execute(ConsoleCommand::MeterProfile(None))]
        );
        assert_eq!(
            one("meter reset"),
            [Output::Execute(ConsoleCommand::ResetMeters)]
        );
        assert_eq!(
            one("meter loud"),
            [Output::Reject(Refusal::MalformedArguments)]
        );
        assert!(ConsoleCommand::MeterProfile(None).only_reads());
        assert!(!ConsoleCommand::MeterProfile(Some(MeterProfileName::Obs)).only_reads());
    }

    #[test]
    fn power_nmpa_is_a_read_only_report_and_power_alone_is_unchanged() {
        assert_eq!(
            one("power"),
            [Output::Execute(ConsoleCommand::Report(Report::Power))]
        );
        assert_eq!(
            one("power nmpa"),
            [Output::Execute(ConsoleCommand::Report(Report::PowerNmpa))]
        );
        assert_eq!(
            one("power nmpa now"),
            [Output::Reject(Refusal::MalformedArguments)]
        );
        assert_eq!(
            one("power raise"),
            [Output::Reject(Refusal::MalformedArguments)]
        );
        assert!(ConsoleCommand::Report(Report::PowerNmpa).only_reads());
    }
}

#[cfg(feature = "sound3")]
mod obs_screen {
    use super::*;
    use device::lcd::ballistics::OBS_RELEASE_PER_FRAME;
    use device::lcd::screen::{R_BAR_RIGHT_EDGE, readout_left_edge};

    const GREEN: u16 = 0x07E0;
    const YELLOW: u16 = 0xFFE0;
    const RED: u16 = 0xF800;

    fn obs(dsp: DspBadge) -> Screen {
        Screen {
            profile: MeterProfileName::Obs,
            ..screen(dsp)
        }
    }

    fn fed(mut screen: Screen, db: f64, frames: usize) -> Screen {
        for _ in 0..frames {
            screen.meters = screen.meters.next_frame_tapped(
                [reading(db); 4],
                [reading(-30.0); 2],
                0,
                OBS_RELEASE_PER_FRAME,
            );
        }
        screen
    }

    fn lit_left_edge(glass: &Glass2, rows: std::ops::Range<usize>) -> Option<usize> {
        (usize::from(R_BAR_RIGHT_EDGE) + 1..157)
            .find(|&x| rows.clone().any(|y| glass.at(x, y) != 0x0000))
    }

    #[test]
    fn a_full_obs_bar_is_continuous_red_yellow_green_in_pure_tn_primaries() {
        let mut glass = cleared();
        glass.draw_all(&fed(obs(DspBadge::Clarity), -0.3, 1));
        for y in 34..42 {
            assert_eq!(glass.at(60, y), RED, "row {y}");
        }
        for y in 42..55 {
            assert_eq!(glass.at(60, y), YELLOW, "row {y}");
        }
        for y in 55..103 {
            assert_eq!(
                glass.at(60, y),
                GREEN,
                "row {y}: continuous, no segment gaps"
            );
        }
        assert_eq!(glass.at(60, 32), PEAK_CAP);
    }

    #[test]
    fn a_quiet_obs_bar_shows_no_ghost_ladder_only_the_two_zone_gates() {
        let mut glass = cleared();
        glass.draw_all(&obs(DspBadge::Clarity));
        assert_eq!(glass.at(48, 101), 0x0000, "no ghost ladder");
        assert_eq!(glass.at(60, 41), 0x6000, "the -9 dB gate");
        assert_eq!(glass.at(60, 54), 0x6300, "the -20 dB gate");
        assert_eq!(glass.at(60, 70), 0x0000);
        assert_eq!(glass.at(44, 42), RED, "the 9 tick");
        assert_eq!(glass.at(44, 55), YELLOW, "the 20 tick");
        let nine_red = (32..43).any(|x| (39..46).any(|y| glass.at(x, y) == RED));
        assert!(nine_red, "the 9 label is red");
        let twenty_yellow = (32..43).any(|x| (52..59).any(|y| glass.at(x, y) == YELLOW));
        assert!(twenty_yellow, "the 20 label is yellow");
    }

    #[test]
    fn the_widest_readout_never_touches_the_r_bar() {
        assert_eq!(R_BAR_RIGHT_EDGE, 101);
        for text in [b"-60.0".as_slice(), b"-31.3", b"-15.1", b"-0.7"] {
            assert!(
                readout_left_edge(text) >= R_BAR_RIGHT_EDGE + 3,
                "{}",
                String::from_utf8_lossy(text)
            );
        }
        for db in [-59.96, -31.3, -15.1] {
            let mut glass = cleared();
            let screen = fed(obs(DspBadge::Clarity), db, 13);
            glass.draw_all(&screen);
            let left = lit_left_edge(&glass, 46..60).expect("the readout is drawn");
            assert!(
                left >= usize::from(R_BAR_RIGHT_EDGE) + 3,
                "{db} dB readout starts at x {left}"
            );
            for x in usize::from(R_BAR_RIGHT_EDGE) + 1..left {
                assert!(
                    (46..60).all(|y| glass.at(x, y) == 0x0000),
                    "column {x} is dark"
                );
            }
        }
    }

    #[test]
    fn the_readout_digits_take_their_zone_colour() {
        for (db, colour) in [(-0.7, RED), (-15.1, YELLOW), (-31.3, GREEN)] {
            let mut glass = cleared();
            glass.draw_all(&fed(obs(DspBadge::Clarity), db, 13));
            let lit: Vec<u16> = (104..157)
                .flat_map(|x| (46..60).map(move |y| (x, y)))
                .map(|(x, y)| glass.at(x, y))
                .filter(|&c| c != 0x0000)
                .collect();
            assert!(!lit.is_empty());
            assert!(lit.iter().all(|&c| c == colour), "{db} dB");
        }
    }

    #[test]
    fn the_amber_band_cue_shows_only_when_fewer_bands_run_than_the_preset_holds() {
        let mut reduced = obs(DspBadge::Clarity);
        reduced.rate = SampleRate::Hz96000;
        reduced.reduced_bands = Some(7);
        let mut glass = cleared();
        glass.draw_all(&reduced);
        let amber = (70..120).any(|x| (5..19).any(|y| glass.at(x, y) == 0xFD84));
        assert!(amber);
        let mut full = reduced;
        full.reduced_bands = None;
        let dirty = full.regions_changed_from(&reduced);
        assert_eq!(dirty, RegionSet::of(Region::Rate));
        glass.send(dirty, &full);
        let amber = (70..120).any(|x| (5..19).any(|y| glass.at(x, y) == 0xFD84));
        assert!(!amber);
    }

    #[test]
    fn switching_profile_redraws_the_scale_labels_and_bars_and_leaves_the_same_glass_as_a_full_draw()
     {
        let studio = screen(DspBadge::Warm);
        let mut glass = cleared();
        glass.draw_all(&studio);
        let obs = Screen {
            profile: MeterProfileName::Obs,
            ..studio
        };
        let dirty = obs.regions_changed_from(&studio);
        for region in [
            Region::Scale,
            Region::Labels,
            Region::OutputLeft,
            Region::Input1,
        ] {
            assert!(dirty.contains(region), "{region:?}");
        }
        let mut left = dirty;
        while !left.is_empty() {
            left = glass.send(left, &obs).leftover;
        }
        let mut full = cleared();
        full.draw_all(&obs);
        assert_eq!(glass.pixels, full.pixels);
    }
}
