// WHY: These pin the sound2 screen; sound2-ui draws screen2 (tests/lcd_screen2.rs)
#![cfg(not(feature = "screen2"))]

use device::breadcrumb::{Breadcrumb, Place, Weight};
use device::fault::FaultCode;
use device::lcd::meter::MeterFrame;
use device::lcd::panel::{
    INFERRED_PANEL_SCK, Microseconds, PANEL_COLUMNS, PANEL_ROWS, RamWindow, SpiByteCount,
};
use device::lcd::screen::DspBadge;
use device::lcd::screen::{
    BACKGROUND, ClockLock, FAULT_BACKGROUND, FAULT_WINDOW, FULL_SCREEN_CLEAR_SPI_BYTES,
    FatalScreen, LARGEST_REGION_SPI_BYTES, MAX_REGION_PIXELS, METER_FRAME_SPI_BYTES, Region,
    RegionPixels, RegionSet, Rgb565, Screen, StreamActivity, render, render_fault,
};
use device::lcd::wire::PanelWord;
use device::phantom::PhantomPower;
use dsp::{ClippedTicks, MeterReading, PeakLevel, SampleRate};

fn reading(peak_bits: u32) -> MeterReading {
    MeterReading {
        peak: PeakLevel::from_bits(peak_bits),
        clipped_ticks: ClippedTicks::from_count(0),
    }
}

fn quiet_screen() -> Screen {
    Screen {
        rate: SampleRate::Hz48000,
        clock: ClockLock::Locked,
        playback: StreamActivity::Idle,
        capture: StreamActivity::Idle,
        phantom: [PhantomPower::Off, PhantomPower::Off],
        meters: MeterFrame::SILENT,
        dsp: DspBadge::Bypass,
        breadcrumb: at_running(NO_CODE),
        previous_boot: None,
    }
}

const NO_CODE: Option<(u8, u8)> = None;
const CODE_16_NOTED_AT_PLACE_3: Option<(u8, u8)> = Some((16, 3));

fn crumb(latest: u8, code: Option<(u8, u8)>) -> Breadcrumb {
    match code {
        Some((code, stage)) => Breadcrumb {
            place: Place { stage, step: 0 },
            code,
            weight: Weight::Noted,
            latest,
        },
        None => Breadcrumb {
            place: Place {
                stage: latest,
                step: 0,
            },
            code: 0,
            weight: Weight::Noted,
            latest,
        },
    }
}

fn at_running(code: Option<(u8, u8)>) -> Breadcrumb {
    crumb(16, code)
}

fn booting(latest: u8, code: Option<(u8, u8)>, previous_boot: Option<Breadcrumb>) -> Screen {
    Screen {
        breadcrumb: crumb(latest, code),
        previous_boot,
        ..quiet_screen()
    }
}

const PREVIOUS_BOOT_FAULTED: Breadcrumb = Breadcrumb::unpack(0x118B_040C);

fn glyph_read(rows: [u8; 7]) -> char {
    match rows {
        [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00] => ' ',
        [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E] => '0',
        [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E] => '1',
        [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F] => '2',
        [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E] => '3',
        [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02] => '4',
        [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E] => '5',
        [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E] => '6',
        [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08] => '7',
        [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E] => '8',
        [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C] => '9',
        [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00] => '-',
        [0x01, 0x01, 0x02, 0x04, 0x08, 0x10, 0x10] => '/',
        [0x0E, 0x11, 0x17, 0x15, 0x17, 0x10, 0x0E] => '@',
        [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11] => 'A',
        [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E] => 'C',
        [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10] => 'F',
        [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F] => 'L',
        [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10] => 'P',
        [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E] => 'S',
        [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04] => 'T',
        [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E] => 'U',
        _ => '?',
    }
}

struct Seen<'a> {
    pixels: &'a [PanelWord],
    height: usize,
    background: Rgb565,
}

impl Seen<'_> {
    fn of(window: RamWindow, pixels: &[PanelWord], background: Rgb565) -> Seen<'_> {
        assert_eq!(pixels.len(), window.pixel_count());
        Seen {
            pixels,
            height: usize::from(window.columns()),
            background,
        }
    }

    fn width(&self) -> usize {
        self.pixels.len() / self.height
    }

    fn lit(&self, x: usize, y: usize) -> bool {
        x < self.width()
            && y < self.height
            && self.pixels[x * self.height + (self.height - 1 - y)].colour() != self.background
    }

    fn cell_lit(&self, x: usize, y: usize, scale: usize) -> bool {
        let lit = self.lit(x, y);
        for dx in 0..scale {
            for dy in 0..scale {
                assert_eq!(self.lit(x + dx, y + dy), lit, "a scaled pixel is uniform");
            }
        }
        lit
    }

    fn text(&self, left: usize, top: usize, scale: usize, characters: usize) -> String {
        let read: String = (0..characters)
            .map(|character| {
                let mut rows = [0u8; 7];
                for (row, bits) in rows.iter_mut().enumerate() {
                    for column in 0..5 {
                        let x = left + (character * 6 + column) * scale;
                        if self.cell_lit(x, top + row * scale, scale) {
                            *bits |= 1 << (4 - column);
                        }
                    }
                }
                glyph_read(rows)
            })
            .collect();
        read.trim_end().to_string()
    }

    fn lit_pixels_in_rows(&self, rows: std::ops::Range<usize>) -> usize {
        rows.flat_map(|y| (0..self.width()).map(move |x| (x, y)))
            .filter(|(x, y)| self.lit(*x, *y))
            .count()
    }

    fn picture(&self) -> String {
        (0..self.height)
            .map(|y| {
                let row: String = (0..self.width())
                    .map(|x| if self.lit(x, y) { '#' } else { '.' })
                    .collect();
                format!("{}\n", row.trim_end_matches('.'))
            })
            .collect()
    }
}

const TEXT_LINE_LEFT: usize = 2;
const TEXT_LINE_TOP: usize = 3;
const TEXT_LINE_CHARACTERS: usize = 26;

fn text_line(screen: &Screen) -> String {
    let mut store = RegionPixels::new();
    let pixels = render(Region::Dsp, screen, &mut store);
    let seen = Seen::of(Region::Dsp.window(), pixels, BACKGROUND);
    let text = seen.text(TEXT_LINE_LEFT, TEXT_LINE_TOP, 1, TEXT_LINE_CHARACTERS);
    assert_eq!(
        seen.lit_pixels_in_rows(0..TEXT_LINE_TOP) + seen.lit_pixels_in_rows(TEXT_LINE_TOP + 7..12),
        0,
        "nothing is drawn above or below the line"
    );
    if std::env::var_os("LCD_LINE_PRINT").is_some() {
        println!("text line {text:?}:\n{}", seen.picture());
    }
    text
}

fn dsp_region_digest(screen: &Screen) -> u64 {
    let mut store = RegionPixels::new();
    fnv1a(render(Region::Dsp, screen, &mut store))
}

fn busy_screen() -> Screen {
    Screen {
        rate: SampleRate::Hz192000,
        clock: ClockLock::Locked,
        playback: StreamActivity::Active,
        capture: StreamActivity::Active,
        phantom: [PhantomPower::On, PhantomPower::Off],
        meters: MeterFrame::SILENT.next_frame([
            reading(0x2000_0000),
            reading(0x0080_0000),
            MeterReading {
                peak: PeakLevel::CLIP,
                clipped_ticks: ClippedTicks::from_count(3),
            },
            reading(0x6000_0000),
        ]),
        dsp: DspBadge::On,
        breadcrumb: at_running(NO_CODE),
        previous_boot: None,
    }
}

fn changing_screen() -> Screen {
    Screen {
        rate: SampleRate::Hz44100,
        clock: ClockLock::Changing,
        dsp: DspBadge::On,
        ..quiet_screen()
    }
}

fn fnv1a(pixels: &[PanelWord]) -> u64 {
    pixels.iter().fold(0xCBF2_9CE4_8422_2325, |hash, pixel| {
        pixel
            .colour()
            .bits()
            .to_le_bytes()
            .into_iter()
            .fold(hash, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01B3)
            })
    })
}

fn region_name(region: Region) -> &'static str {
    match region {
        Region::Rate => "rate",
        Region::Stream => "stream",
        Region::Phantom => "phantom",
        Region::Meters => "meters",
        Region::Legend => "legend",
        Region::Dsp => "dsp",
    }
}

fn digests(screen: &Screen) -> Vec<(&'static str, u64)> {
    let mut store = RegionPixels::new();
    Region::ALL
        .into_iter()
        .map(|region| {
            (
                region_name(region),
                fnv1a(render(region, screen, &mut store)),
            )
        })
        .collect()
}

fn assert_golden(screen_name: &str, screen: &Screen, expected: &[(&str, u64)]) {
    let actual = digests(screen);
    if std::env::var_os("LCD_GOLDEN_PRINT").is_some() {
        println!("{screen_name}: {actual:#018X?}");
    }
    for ((region, digest), (expected_region, expected_digest)) in actual.iter().zip(expected) {
        assert_eq!(region, expected_region);
        assert_eq!(
            digest, expected_digest,
            "screen {screen_name}: region {region} renders differently, digest {digest:#018X}"
        );
    }
}

struct RamImage([[Rgb565; PANEL_COLUMNS as usize]; PANEL_ROWS as usize]);

impl RamImage {
    fn cleared(colour: Rgb565) -> Self {
        Self([[colour; PANEL_COLUMNS as usize]; PANEL_ROWS as usize])
    }

    fn write(&mut self, window: RamWindow, pixels: &[PanelWord]) {
        assert_eq!(pixels.len(), window.pixel_count());
        for (index, pixel) in pixels.iter().enumerate() {
            let row = usize::from(window.first_row()) + index / usize::from(window.columns());
            let column = usize::from(window.first_column()) + index % usize::from(window.columns());
            self.0[row][column] = pixel.colour();
        }
    }

    fn landscape_ppm(&self) -> Vec<u8> {
        let mut out = format!("P6\n{} {}\n255\n", PANEL_ROWS, PANEL_COLUMNS).into_bytes();
        for seen_y in 0..usize::from(PANEL_COLUMNS) {
            for seen_x in 0..usize::from(PANEL_ROWS) {
                let bits = self.0[seen_x][usize::from(PANEL_COLUMNS) - 1 - seen_y].bits();
                let red = ((bits >> 11) & 0x1F) as u8;
                let green = ((bits >> 5) & 0x3F) as u8;
                let blue = (bits & 0x1F) as u8;
                out.extend([red << 3, green << 2, blue << 3]);
            }
        }
        out
    }
}

fn composed(screen: &Screen) -> RamImage {
    let mut image = RamImage::cleared(BACKGROUND);
    let mut store = RegionPixels::new();
    for region in Region::ALL {
        image.write(region.window(), render(region, screen, &mut store));
    }
    image
}

#[test]
fn a_preview_of_each_golden_screen_is_written_when_asked() {
    let Some(directory) = std::env::var_os("LCD_PREVIEW_DIR") else {
        return;
    };
    let directory = std::path::PathBuf::from(directory);
    for (name, screen) in [
        ("quiet", quiet_screen()),
        ("busy", busy_screen()),
        ("changing", changing_screen()),
    ] {
        std::fs::write(
            directory.join(format!("{name}.ppm")),
            composed(&screen).landscape_ppm(),
        )
        .expect("the preview directory is writable");
    }
    let mut image = RamImage::cleared(FAULT_BACKGROUND);
    let mut store = RegionPixels::new();
    image.write(
        FAULT_WINDOW,
        render_fault(FaultCode::BoardStalled, PINNED_FAULT_STAGE, &mut store),
    );
    std::fs::write(directory.join("fault.ppm"), image.landscape_ppm()).expect("writable");
}

#[test]
fn the_quiet_screen_renders_its_pinned_pixels_in_every_region() {
    assert_golden("quiet", &quiet_screen(), &QUIET_GOLDEN);
}

#[test]
fn the_busy_screen_renders_its_pinned_pixels_in_every_region() {
    assert_golden("busy", &busy_screen(), &BUSY_GOLDEN);
}

#[test]
fn the_changing_rate_screen_renders_its_pinned_pixels_in_every_region() {
    assert_golden("changing", &changing_screen(), &CHANGING_GOLDEN);
}

#[test]
fn every_fault_code_renders_its_pinned_pixels() {
    let mut store = RegionPixels::new();
    let actual: Vec<(u8, u64)> = FaultCode::ALL
        .into_iter()
        .map(|code| {
            (
                code.number(),
                fnv1a(render_fault(code, PINNED_FAULT_STAGE, &mut store)),
            )
        })
        .collect();
    if std::env::var_os("LCD_GOLDEN_PRINT").is_some() {
        println!("fault: {actual:#018X?}");
    }
    assert_eq!(actual.len(), FAULT_GOLDEN.len(), "every code has a pin");
    for ((number, digest), expected) in actual.iter().zip(FAULT_GOLDEN) {
        assert_eq!(
            *digest, expected,
            "region fault code {number} renders differently, digest {digest:#018X}"
        );
    }
    let mut distinct: Vec<u64> = actual.iter().map(|(_, digest)| *digest).collect();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        FaultCode::ALL.len(),
        "each code looks different"
    );
}

#[test]
fn the_meter_region_shows_level_plus_one_lit_rows_as_the_factory_bars_do() {
    let mut store = RegionPixels::new();
    let silent = render(Region::Meters, &quiet_screen(), &mut store).to_vec();
    let loud = render(Region::Meters, &busy_screen(), &mut store).to_vec();
    assert_ne!(silent, loud);
    let lit_green = Rgb565::from_bits(0x07E0);
    let count = |pixels: &[PanelWord]| pixels.iter().filter(|p| p.colour() == lit_green).count();
    let bar_width = 12;
    assert_eq!(
        count(&silent),
        4 * bar_width,
        "silence lights one row per meter"
    );
}

#[test]
fn regions_never_overlap_and_all_fit_the_panel_window() {
    let windows: Vec<RamWindow> = Region::ALL.into_iter().map(Region::window).collect();
    for (index, a) in windows.iter().enumerate() {
        assert!(usize::from(a.first_column()) + usize::from(a.columns()) <= 128);
        assert!(usize::from(a.first_row()) + usize::from(a.rows()) <= 160);
        for b in &windows[index + 1..] {
            let apart = a.first_column() + a.columns() <= b.first_column()
                || b.first_column() + b.columns() <= a.first_column()
                || a.first_row() + a.rows() <= b.first_row()
                || b.first_row() + b.rows() <= a.first_row();
            assert!(apart, "{a:?} overlaps {b:?}");
        }
    }
}

#[test]
fn only_the_regions_whose_pixels_change_are_marked_dirty() {
    let quiet = quiet_screen();
    assert_eq!(quiet.regions_changed_from(&quiet), RegionSet::EMPTY);
    let phantom_on = Screen {
        phantom: [PhantomPower::Off, PhantomPower::On],
        ..quiet
    };
    assert_eq!(
        phantom_on.regions_changed_from(&quiet),
        RegionSet::of(Region::Phantom)
    );
    let peq_48k = Screen {
        dsp: DspBadge::On,
        ..quiet
    };
    let peq_96k = Screen {
        rate: SampleRate::Hz96000,
        ..peq_48k
    };
    assert_eq!(
        peq_96k.regions_changed_from(&peq_48k),
        RegionSet::of(Region::Rate)
    );
    let peq_192k = Screen {
        rate: SampleRate::Hz192000,
        ..peq_48k
    };
    assert_eq!(
        peq_192k.regions_changed_from(&peq_96k),
        RegionSet::of(Region::Rate).with(Region::Dsp)
    );
}

#[test]
fn a_clip_hold_counting_down_redraws_nothing_until_the_box_goes_dark() {
    let clipped = Screen {
        meters: MeterFrame::SILENT.next_frame([
            MeterReading {
                peak: PeakLevel::SILENCE,
                clipped_ticks: ClippedTicks::from_count(1),
            },
            MeterReading::SILENCE,
            MeterReading::SILENCE,
            MeterReading::SILENCE,
        ]),
        ..quiet_screen()
    };
    let mut shown = clipped;
    for frame in 1..=90 {
        let next = Screen {
            meters: shown.meters.next_frame([MeterReading::SILENCE; 4]),
            ..shown
        };
        let dirty = next.regions_changed_from(&shown);
        if frame < 90 {
            assert_eq!(dirty, RegionSet::EMPTY, "frame {frame}");
        } else {
            assert_eq!(dirty, RegionSet::of(Region::Meters), "frame {frame}");
        }
        shown = next;
    }
}

#[test]
fn the_meter_frame_is_10219_spi_bytes_and_6813_microseconds_at_the_inferred_12_mhz_clock() {
    assert_eq!(METER_FRAME_SPI_BYTES, SpiByteCount(11 + 58 * 88 * 2));
    assert_eq!(METER_FRAME_SPI_BYTES, SpiByteCount(10_219));
    assert_eq!(
        METER_FRAME_SPI_BYTES.transfer_time(INFERRED_PANEL_SCK),
        Microseconds(6_813)
    );
}

#[test]
fn the_meter_frame_is_the_largest_region_transfer_and_fits_the_region_buffer() {
    assert_eq!(LARGEST_REGION_SPI_BYTES, METER_FRAME_SPI_BYTES);
    assert_eq!(MAX_REGION_PIXELS, 5_104);
    assert!(
        Region::ALL
            .into_iter()
            .all(|region| region.window().pixel_count() <= MAX_REGION_PIXELS)
    );
    assert!(FAULT_WINDOW.pixel_count() <= MAX_REGION_PIXELS);
}

#[test]
fn the_meter_frame_takes_under_half_of_a_51_hz_te_period_under_the_inferred_frame_rate() {
    let inferred_half_te_period = Microseconds(1_000_000 / 51 / 2);
    assert!(METER_FRAME_SPI_BYTES.transfer_time(INFERRED_PANEL_SCK) < inferred_half_te_period);
}

#[test]
fn a_full_screen_clear_is_the_factory_40971_bytes_and_27_point_3_ms() {
    assert_eq!(FULL_SCREEN_CLEAR_SPI_BYTES, SpiByteCount(40_971));
    assert_eq!(
        FULL_SCREEN_CLEAR_SPI_BYTES.transfer_time(INFERRED_PANEL_SCK),
        Microseconds(27_314)
    );
}

const PINNED_FAULT_STAGE: u8 = 11;

#[test]
fn while_booting_the_text_line_shows_the_latest_place_and_the_previous_record() {
    assert_eq!(
        PREVIOUS_BOOT_FAULTED,
        Breadcrumb {
            place: Place { stage: 12, step: 4 },
            code: 11,
            weight: Weight::Raised,
            latest: 17,
        }
    );
    assert_eq!(
        text_line(&booting(6, NO_CODE, Some(PREVIOUS_BOOT_FAULTED))),
        "S06  P12/17 C11"
    );
    assert_eq!(text_line(&booting(6, NO_CODE, None)), "S06  P--");
    let previous_boot_ran_clean = at_running(NO_CODE);
    assert_eq!(
        text_line(&booting(14, NO_CODE, Some(previous_boot_ran_clean))),
        "S14  P16/16 C00"
    );
    let previous_boot_stopped_early = Breadcrumb {
        place: Place { stage: 5, step: 10 },
        code: 8,
        weight: Weight::Raised,
        latest: 5,
    };
    assert_eq!(
        text_line(&booting(1, NO_CODE, Some(previous_boot_stopped_early))),
        "S01  P05/05 C08"
    );
    assert_eq!(text_line(&booting(17, NO_CODE, None)), "S17  P--");
}

#[test]
fn at_running_the_text_line_shows_the_dsp_mode_again() {
    let (_, bypass_pixels) = QUIET_GOLDEN[5];
    let (_, peq_on_pixels) = CHANGING_GOLDEN[5];
    let booting_screen = booting(15, NO_CODE, Some(PREVIOUS_BOOT_FAULTED));
    assert_ne!(dsp_region_digest(&booting_screen), bypass_pixels);
    let running = Screen {
        breadcrumb: at_running(NO_CODE),
        ..booting_screen
    };
    assert_eq!(dsp_region_digest(&running), bypass_pixels);
    let running_with_peq = Screen {
        dsp: DspBadge::On,
        ..running
    };
    assert_eq!(dsp_region_digest(&running_with_peq), peq_on_pixels);
    assert_eq!(
        running.regions_changed_from(&booting_screen),
        RegionSet::of(Region::Dsp)
    );
}

#[test]
fn a_noted_code_stays_on_the_text_line_at_running() {
    let before_running = booting(6, CODE_16_NOTED_AT_PLACE_3, Some(PREVIOUS_BOOT_FAULTED));
    assert_eq!(text_line(&before_running), "S06  C16@03");
    let after_running = Screen {
        breadcrumb: at_running(CODE_16_NOTED_AT_PLACE_3),
        ..before_running
    };
    assert_eq!(text_line(&after_running), "C16@03");
    let peq_switched_on = Screen {
        dsp: DspBadge::On,
        ..after_running
    };
    assert_eq!(text_line(&peq_switched_on), "C16@03");
    assert_eq!(
        peq_switched_on.regions_changed_from(&after_running),
        RegionSet::EMPTY
    );
}

#[test]
fn the_text_line_is_redrawn_only_when_the_latest_place_or_the_code_changes() {
    let shown = booting(6, NO_CODE, None);
    let next_step_of_the_same_stage = Screen {
        breadcrumb: Breadcrumb {
            place: Place { stage: 6, step: 9 },
            ..shown.breadcrumb
        },
        ..shown
    };
    assert_eq!(
        next_step_of_the_same_stage.regions_changed_from(&shown),
        RegionSet::EMPTY
    );
    assert_eq!(
        booting(7, NO_CODE, None).regions_changed_from(&shown),
        RegionSet::of(Region::Dsp)
    );
    let noted = booting(6, CODE_16_NOTED_AT_PLACE_3, None);
    assert_eq!(
        noted.regions_changed_from(&shown),
        RegionSet::of(Region::Dsp)
    );
    assert_eq!(
        booting(7, CODE_16_NOTED_AT_PLACE_3, None).regions_changed_from(&noted),
        RegionSet::of(Region::Dsp)
    );
    assert_eq!(
        booting(6, Some((5, 3)), None).regions_changed_from(&noted),
        RegionSet::of(Region::Dsp)
    );
    assert_eq!(
        booting(6, Some((16, 6)), None).regions_changed_from(&noted),
        RegionSet::of(Region::Dsp)
    );
    let dsp_switched_while_booting = Screen {
        dsp: DspBadge::On,
        ..shown
    };
    assert_eq!(
        dsp_switched_while_booting.regions_changed_from(&shown),
        RegionSet::EMPTY
    );
    let running = quiet_screen();
    let running_after_another_previous_boot = Screen {
        previous_boot: Some(PREVIOUS_BOOT_FAULTED),
        ..running
    };
    assert_eq!(
        running_after_another_previous_boot.regions_changed_from(&running),
        RegionSet::EMPTY
    );
}

#[test]
fn a_number_above_99_shows_as_99() {
    let previous_boot_of_unknown_numbers = Breadcrumb {
        place: Place {
            stage: 255,
            step: 0,
        },
        code: 127,
        weight: Weight::Raised,
        latest: 100,
    };
    assert_eq!(
        text_line(&booting(6, NO_CODE, Some(previous_boot_of_unknown_numbers))),
        "S06  P99/99 C99"
    );
}

fn fault_screen_text(code: FaultCode, stage: u8) -> [String; 3] {
    let mut store = RegionPixels::new();
    let pixels = render_fault(code, stage, &mut store);
    let seen = Seen::of(FAULT_WINDOW, pixels, FAULT_BACKGROUND);
    let centred =
        |characters: usize, scale: usize| (seen.width() - (characters * 6 - 1) * scale) / 2;
    let digits = code.number().to_string().len();
    if std::env::var_os("LCD_LINE_PRINT").is_some() {
        println!("fault screen:\n{}", seen.picture());
    }
    [
        seen.text(centred(5, 2), 2, 2, 5),
        seen.text(centred(digits, 4), 22, 4, digits),
        seen.text(centred(3, 2), 54, 2, 3),
    ]
}

#[test]
fn the_fault_screen_shows_the_code_and_the_stage() {
    assert_eq!(
        fault_screen_text(FaultCode::HardFault, 5),
        ["FAULT", "8", "@05"]
    );
    assert_eq!(
        fault_screen_text(FaultCode::UsbPhyPllUnlocked, 17),
        ["FAULT", "16", "@17"]
    );
    assert_eq!(
        fault_screen_text(FaultCode::CrystalNotReady, 2),
        ["FAULT", "12", "@02"]
    );
}

const QUIET_GOLDEN: [(&str, u64); 6] = [
    ("rate", 0x7974_F0D4_EAE9_DFBD),
    ("stream", 0x0D16_7354_E6B6_3D72),
    ("phantom", 0xFC38_C196_6271_9205),
    ("meters", 0x3BBE_D226_1839_AE25),
    ("legend", 0x35D9_39C9_1817_502A),
    ("dsp", 0x514F_A3EF_603D_F36A),
];

const BUSY_GOLDEN: [(&str, u64); 6] = [
    ("rate", 0x3838_86A4_73ED_4615),
    ("stream", 0x1C24_31AC_AD48_E061),
    ("phantom", 0x8ECB_EFBB_779A_E091),
    ("meters", 0xED6F_5724_E37F_E7A5),
    ("legend", 0x35D9_39C9_1817_502A),
    ("dsp", 0xEF79_3927_0220_ED09),
];

const CHANGING_GOLDEN: [(&str, u64); 6] = [
    ("rate", 0x7A5A_5BE9_83BC_A8BD),
    ("stream", 0x0D16_7354_E6B6_3D72),
    ("phantom", 0xFC38_C196_6271_9205),
    ("meters", 0x3BBE_D226_1839_AE25),
    ("legend", 0x35D9_39C9_1817_502A),
    ("dsp", 0x20F2_94E0_D5A6_4FBE),
];

const FAULT_GOLDEN: [u64; 13] = [
    0x82C2_81E6_98F5_46FD,
    0x028C_1033_00C7_71FD,
    0x727E_0221_0A07_3E9D,
    0x991B_A066_C77B_F2DD,
    0x9123_E459_5571_045D,
    0x1192_6B2F_FF8F_C19D,
    0xEDA7_AFBD_0819_2EDD,
    0x2BD1_C74D_597D_021D,
    0x6327_E14A_EA35_263D,
    0x7EE6_5788_DD15_81BD,
    0xC7C1_FFA6_2A0F_1BBD,
    0x8BF9_2AA4_5B9C_62BD,
    0x2B44_5ABB_A721_719D,
];

#[test]
fn the_fatal_screen_is_drawn_only_once_and_only_after_the_screen_started() {
    let (after_a_fault_before_the_start, drawn) = FatalScreen::NotStarted.attempted();
    assert!(!drawn, "a fault before the screen started draws nothing");
    assert!(
        !after_a_fault_before_the_start.attempted().1,
        "a second fault before the start draws nothing"
    );
    assert!(
        !after_a_fault_before_the_start.started().attempted().1,
        "a start after an attempt does not arm a second attempt"
    );

    let started = FatalScreen::NotStarted.started();
    let (after_the_first_attempt, first) = started.attempted();
    assert!(first, "the first fatal fault after the start draws");
    let (after_the_second_attempt, second) = after_the_first_attempt.attempted();
    assert!(!second, "a fault inside the first attempt draws nothing");
    assert!(!after_the_second_attempt.attempted().1);

    for state in [
        FatalScreen::NotStarted,
        started,
        after_the_first_attempt,
        after_the_second_attempt,
    ] {
        assert_eq!(FatalScreen::from_bits(state as u8), state);
    }
}
