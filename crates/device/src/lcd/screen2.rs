use core::sync::atomic::{AtomicBool, Ordering};

use dsp::{DspMode, PeqAtRate, SampleRate};

use crate::boot::RUNNING;
use crate::breadcrumb::Breadcrumb;
use crate::fault::FaultCode;
pub use crate::lcd::ballistics::MeterFrame;
use crate::lcd::ballistics::{Alert, BAR_PIXELS, Decibels, Meter, Zone};
use crate::lcd::glyphs::{
    GLYPH_ADVANCE, GLYPH_HEIGHT, GLYPH_WIDTH, GlyphRows, character, has_glyph,
};
use crate::lcd::panel::{PANEL_COLUMNS, RamWindow, SpiByteCount};
use crate::lcd::wire::{MOST_TRANSFERS_PER_DESCRIPTOR, PanelWord};
use crate::monitor::Monitoring;
use crate::phantom::PhantomPower;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DspBadge {
    Bypass,
    On,
    Clarity,
    Warm,
    Raw,
    Custom,
}

impl From<DspMode> for DspBadge {
    fn from(mode: DspMode) -> Self {
        match mode {
            DspMode::Bypass => Self::Bypass,
            DspMode::On => Self::On,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb565(u16);

impl Rgb565 {
    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    const fn eased_toward(self, target: Self, weight: u8) -> Self {
        let from = self.0 as i32;
        let to = target.0 as i32;
        let weight = weight as i32;
        let blue = blend(from & 0x1F, to & 0x1F, weight);
        let green = blend((from >> 5) & 0x3F, (to >> 5) & 0x3F, weight);
        let red = blend((from >> 11) & 0x1F, (to >> 11) & 0x1F, weight);
        Self(((red << 11) | (green << 5) | blue) as u16)
    }
}

const fn blend(from: i32, to: i32, weight: i32) -> i32 {
    (from * (255 - weight) + to * weight + 127) / 255
}

// WHY: 0x0000 is the darkest a TN panel can show, so the ground no longer glows
pub const BACKGROUND: Rgb565 = Rgb565(0x0000);
pub const FAULT_BACKGROUND: Rgb565 = Rgb565(0x8000);
const GROUND: Rgb565 = BACKGROUND;
const GHOST: Rgb565 = Rgb565(0x18C3);
const SIGNAL: Rgb565 = Rgb565(0x1F0E);
const HOT: Rgb565 = Rgb565(0xFD84);
const OVER: Rgb565 = Rgb565(0xF986);
const PEAK_CAP: Rgb565 = Rgb565(0xF7BE);
const TEXT: Rgb565 = Rgb565(0xFFFF);
const TEXT_2: Rgb565 = Rgb565(0x8C71);
const TEXT_3: Rgb565 = Rgb565(0x4A49);
const CLARITY: Rgb565 = Rgb565(0x069F);
const WARM: Rgb565 = Rgb565(0xFE28);
const RAW: Rgb565 = Rgb565(0xE71C);
const PHANTOM_ON: Rgb565 = Rgb565(0xF945);
const CHIP_LINE: Rgb565 = Rgb565(0x2945);
const CHIP_TEXT: Rgb565 = Rgb565(0x39E7);
const DOT_RING: Rgb565 = Rgb565(0x3186);
const SCALE_TICK: Rgb565 = Rgb565(0x39C7);
const FAULT_TEXT: Rgb565 = Rgb565(0xFFFF);
// WHY: A TN pixel at full drive moves least with the viewing angle, so the obs zones use only full-scale channels and the zone gates are a dim single channel
const OBS_GREEN: Rgb565 = Rgb565(0x07E0);
const OBS_YELLOW: Rgb565 = Rgb565(0xFFE0);
const OBS_RED: Rgb565 = Rgb565(0xF800);
const GATE_RED: Rgb565 = Rgb565(0x6000);
const GATE_YELLOW: Rgb565 = Rgb565(0x6300);
const REDUCED_BANDS: Rgb565 = Rgb565(0xFD84);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockLock {
    Locked,
    Changing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamActivity {
    Idle,
    Active,
}

static MOTION: AtomicBool = AtomicBool::new(true);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeterProfileName {
    Obs,
    Studio,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeterTap {
    UsbIntake,
    Dac,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MeterScale {
    Linear,
    TwoSlope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ScaleMark {
    text: &'static [u8],
    row: u8,
    text_colour: Rgb565,
    tick_colour: Rgb565,
}

// WHY: One profile value carries the tap, scale, zones, labels and release, so the renderer reads data and never branches on a profile name
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeterProfile {
    pub name: &'static [u8],
    pub tap: MeterTap,
    pub release: Decibels,
    scale: MeterScale,
    zone_rows: [u8; 2],
    zone_colours: [Rgb565; 3],
    segmented: bool,
    gates: [(u8, Rgb565); 2],
    marks: [ScaleMark; 5],
    readout_in_zone_colour: bool,
}

const fn mark(text: &'static [u8], row: u8, text_colour: Rgb565, tick_colour: Rgb565) -> ScaleMark {
    ScaleMark {
        text,
        row,
        text_colour,
        tick_colour,
    }
}

const NO_MARK: ScaleMark = mark(b"", 0, GROUND, GROUND);

const fn two_slope_row(db: i16) -> u8 {
    BAR_PIXELS - Decibels::from_q8(db * 256).bar_pixels()
}

const fn linear_row(db: i16) -> u8 {
    BAR_PIXELS - Decibels::from_q8(db * 256).linear_pixels()
}

// WHY: OBS-style meter: the USB intake on a linear -60 to 0 dB scale, green below -20, yellow to -9, red above, released at 11.76 dB/s
pub const OBS: MeterProfile = MeterProfile {
    name: b"obs",
    tap: MeterTap::UsbIntake,
    release: crate::lcd::ballistics::OBS_RELEASE_PER_FRAME,
    scale: MeterScale::Linear,
    zone_rows: [linear_row(-9), linear_row(-20)],
    zone_colours: [OBS_RED, OBS_YELLOW, OBS_GREEN],
    segmented: false,
    gates: [
        (linear_row(-9) - 1, GATE_RED),
        (linear_row(-20) - 1, GATE_YELLOW),
    ],
    marks: [
        mark(b"0", 0, TEXT_3, SCALE_TICK),
        mark(b"9", linear_row(-9), OBS_RED, OBS_RED),
        mark(b"20", linear_row(-20), OBS_YELLOW, OBS_YELLOW),
        mark(b"40", linear_row(-40), TEXT_3, SCALE_TICK),
        NO_MARK,
    ],
    readout_in_zone_colour: true,
};

pub const STUDIO: MeterProfile = MeterProfile {
    name: b"studio",
    tap: MeterTap::Dac,
    release: crate::lcd::ballistics::RELEASE_PER_FRAME,
    scale: MeterScale::TwoSlope,
    zone_rows: [RED_ROWS_FROM_TOP, AMBER_ROWS_FROM_TOP],
    zone_colours: [OVER, HOT, SIGNAL],
    segmented: true,
    gates: [(u8::MAX, GROUND), (u8::MAX, GROUND)],
    marks: [
        mark(b"0", two_slope_row(0), TEXT_3, SCALE_TICK),
        mark(b"6", two_slope_row(-6), TEXT_3, SCALE_TICK),
        mark(b"12", two_slope_row(-12), TEXT_3, SCALE_TICK),
        mark(b"20", two_slope_row(-20), TEXT_3, SCALE_TICK),
        mark(b"40", two_slope_row(-40), TEXT_3, SCALE_TICK),
    ],
    readout_in_zone_colour: false,
};

impl MeterProfileName {
    pub const fn profile(self) -> &'static MeterProfile {
        match self {
            MeterProfileName::Obs => &OBS,
            MeterProfileName::Studio => &STUDIO,
        }
    }

    pub fn named(word: &[u8]) -> Option<Self> {
        match word {
            b"obs" => Some(Self::Obs),
            b"studio" => Some(Self::Studio),
            _ => None,
        }
    }
}

// WHY: Sound3 boots into obs; sound2-ui keeps its DAC meters
const PROFILE_AT_BOOT: MeterProfileName = if cfg!(feature = "sound3") {
    MeterProfileName::Obs
} else {
    MeterProfileName::Studio
};

static OBS_PROFILE: AtomicBool = AtomicBool::new(matches!(PROFILE_AT_BOOT, MeterProfileName::Obs));

pub fn set_meter_profile(profile: MeterProfileName) {
    OBS_PROFILE.store(matches!(profile, MeterProfileName::Obs), Ordering::Relaxed);
}

pub fn meter_profile() -> MeterProfileName {
    if OBS_PROFILE.load(Ordering::Relaxed) {
        MeterProfileName::Obs
    } else {
        MeterProfileName::Studio
    }
}

pub fn set_motion(on: bool) {
    MOTION.store(on, Ordering::Relaxed);
}

pub fn motion() -> bool {
    MOTION.load(Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PillColours {
    line: Rgb565,
    fill: Rgb565,
    ink: Rgb565,
}

impl PillColours {
    pub const fn line(self) -> Rgb565 {
        self.line
    }

    pub const fn fill(self) -> Rgb565 {
        self.fill
    }

    pub const fn ink(self) -> Rgb565 {
        self.ink
    }

    const fn filled(fill: Rgb565) -> Self {
        Self {
            line: fill,
            fill,
            ink: GROUND,
        }
    }

    const fn outlined(line: Rgb565) -> Self {
        Self {
            line,
            fill: GROUND,
            ink: line,
        }
    }

    const fn eased_toward(self, target: Self, weight: u8) -> Self {
        Self {
            line: self.line.eased_toward(target.line, weight),
            fill: self.fill.eased_toward(target.fill, weight),
            ink: self.ink.eased_toward(target.ink, weight),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PillTarget {
    pub label: &'static [u8],
    pub colours: PillColours,
}

// WHY: Ease-out cubic-bezier(0.23, 1, 0.32, 1) over 8 TE frames, about 156 ms
pub const PILL_EASE: [u8; 8] = [123, 198, 232, 246, 252, 254, 255, 255];
const EASE_DONE: u8 = PILL_EASE.len() as u8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PillFade {
    label: &'static [u8],
    from: PillColours,
    to: PillColours,
    step: u8,
}

const NOT_RUNNING_PILL: PillTarget = PillTarget {
    label: b"",
    colours: PillColours::outlined(CHIP_LINE),
};

impl PillFade {
    pub const AT_BOOT: Self = Self {
        label: NOT_RUNNING_PILL.label,
        from: NOT_RUNNING_PILL.colours,
        to: NOT_RUNNING_PILL.colours,
        step: EASE_DONE,
    };

    pub const fn colours(self) -> PillColours {
        if self.step >= EASE_DONE {
            self.to
        } else {
            self.from
                .eased_toward(self.to, PILL_EASE[self.step as usize])
        }
    }

    pub const fn label(self) -> &'static [u8] {
        self.label
    }

    pub const fn settled(self) -> bool {
        self.step >= EASE_DONE
    }

    pub fn next_frame(self, target: PillTarget, motion: bool) -> Self {
        if target.colours != self.to {
            Self {
                label: target.label,
                from: self.colours(),
                to: target.colours,
                step: if motion { 0 } else { EASE_DONE },
            }
        } else {
            Self {
                label: target.label,
                step: if motion {
                    (self.step + 1).min(EASE_DONE)
                } else {
                    EASE_DONE
                },
                ..self
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Screen {
    pub rate: SampleRate,
    pub clock: ClockLock,
    pub playback: StreamActivity,
    pub capture: StreamActivity,
    pub phantom: [PhantomPower; 2],
    pub monitoring: [Monitoring; 2],
    pub meters: MeterFrame,
    pub dsp: DspBadge,
    pub pill: PillFade,
    pub profile: MeterProfileName,
    pub reduced_bands: Option<u8>,
    pub breadcrumb: Breadcrumb,
    pub previous_boot: Option<Breadcrumb>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    PhantomChip1,
    PhantomChip2,
    MonitorChip1,
    MonitorChip2,
    Dot,
    Pill,
    Rate,
    Readout,
    Input1,
    Input2,
    OutputLeft,
    OutputRight,
    Scale,
    Labels,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionSet(u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SeenRect {
    x: u8,
    y: u8,
    width: u8,
    height: u8,
}

impl SeenRect {
    const fn ram_window(self) -> RamWindow {
        RamWindow::new(
            PANEL_COLUMNS - self.y - self.height,
            self.height,
            self.x,
            self.width,
        )
    }

    const fn checked_window(self) -> Option<RamWindow> {
        match PANEL_COLUMNS.checked_sub(self.y) {
            Some(below) if below >= self.height => {
                RamWindow::checked(below - self.height, self.height, self.x, self.width)
            }
            Some(_) | None => None,
        }
    }

    const fn pixel_count(self) -> usize {
        self.width as usize * self.height as usize
    }

    const fn rows(self, top: u8, bottom: u8) -> Self {
        Self {
            y: top,
            height: bottom - top + 1,
            ..self
        }
    }

    const fn right(self) -> u8 {
        self.x + self.width - 1
    }
}

const fn rect(x: u8, y: u8, width: u8, height: u8) -> SeenRect {
    SeenRect {
        x,
        y,
        width,
        height,
    }
}

const EDGE_RIGHT: u8 = 156;
const PILL_AREA: SeenRect = rect(3, 2, 46, 20);
const RATE_AREA: SeenRect = rect(70, 5, 87, 14);
const REDUCED_BANDS_GAP: u8 = 3;
const DOT_AREA: SeenRect = rect(72, 23, 6, 6);
const READOUT_AREA: SeenRect = rect(104, 46, 53, 25);
const CHIP_TOP: u8 = 114;
const CHIP_WIDTH: u8 = 37;
const CHIP_HEIGHT: u8 = 14;
const CHIP_AREAS: [SeenRect; 4] = [
    rect(3, CHIP_TOP, CHIP_WIDTH, CHIP_HEIGHT),
    rect(42, CHIP_TOP, CHIP_WIDTH, CHIP_HEIGHT),
    rect(81, CHIP_TOP, CHIP_WIDTH, CHIP_HEIGHT),
    rect(120, CHIP_TOP, CHIP_WIDTH, CHIP_HEIGHT),
];
const BAR_TOP: u8 = 31;
const BAR_BOTTOM: u8 = BAR_TOP + BAR_PIXELS - 1;
const CLIP_CAP_TOP: u8 = 28;
const CLIP_CAP_ROWS: u8 = 2;
const INPUT_AREAS: [SeenRect; 2] = [
    rect(3, CLIP_CAP_TOP, 10, BAR_BOTTOM - CLIP_CAP_TOP + 1),
    rect(15, CLIP_CAP_TOP, 10, BAR_BOTTOM - CLIP_CAP_TOP + 1),
];
const OUTPUT_AREAS: [SeenRect; 2] = [
    rect(48, BAR_TOP, 26, BAR_PIXELS),
    rect(76, BAR_TOP, 26, BAR_PIXELS),
];
const SCALE_AREA: SeenRect = rect(32, 28, 14, 68);
const SCALE_TEXT_RIGHT: u8 = 42;
const SCALE_TICK_LEFT: u8 = 44;
const SCALE_TICK_WIDTH: u8 = 2;
const LABELS_AREA: SeenRect = rect(3, 105, 99, GLYPH_HEIGHT);
const FAULT_AREA: SeenRect = rect(48, 44, 64, 40);

const RED_ROWS_FROM_TOP: u8 = 7;
const AMBER_ROWS_FROM_TOP: u8 = 26;
const SEGMENT_PITCH: u8 = 3;
const CAP_ROWS: u8 = 2;

pub const MAX_REGION_PIXELS: usize = 2_560;
pub const MOST_PIECES: usize = 8;
// WHY: Board lcd_link MOST_CHAINED_DESCRIPTORS, each piece is one header descriptor plus its pixel descriptors, and the pixels of a batch never pass one arena
pub const MOST_BATCH_DESCRIPTORS: usize =
    MOST_PIECES + MOST_PIECES + MAX_REGION_PIXELS.div_ceil(MOST_TRANSFERS_PER_DESCRIPTOR);
pub const MOST_LCD_DESCRIPTORS: usize = 24;

const SCALE_MARKS: [(&[u8], i16); 5] = [
    (b"0", 0),
    (b"6", -6),
    (b"12", -12),
    (b"20", -20),
    (b"40", -40),
];
const LABEL_TEXT: [&[u8]; 4] = [b"1", b"2", b"L", b"R"];
const CHIP_TEXT_OF: [&[u8]; 4] = [b"48V 1", b"48V 2", b"MON 1", b"MON 2"];
const PEAK_LABEL: &[u8] = b"PK";
const FAULT_WORD: &[u8] = b"FAULT";
const NUMBER: u8 = b'#';
const NUMBER_DIGITS: usize = 2;
const LARGEST_NUMBER: u8 = 99;
const MOST_NUMBERS: usize = 4;
const FAULT_NUMBER_SCALE: u8 = 3;
const FAULT_STAGE: &[u8] = b"@#";
const HERO_SCALE: u8 = 2;
const RUNNING_WITH_A_CODE: [&[u8]; 2] = [b"C#", b"@#"];
const BOOTING_AFTER_A_RECORD: [&[u8]; 3] = [b"S#", b"P#/#", b"C#"];
const BOOTING_AFTER_NO_RECORD: [&[u8]; 2] = [b"S#", b"P--"];
const BOOTING_WITH_A_CODE: [&[u8]; 2] = [b"S#", b"C#@#"];
const SILENT_READOUT: &[u8] = b"--";
const READOUT_LINE_PITCH: u8 = GLYPH_HEIGHT + 1;
const MOST_READOUT_CHARACTERS: usize = 5;
// WHY: A five-character readout like -31.3 at scale 2 touched the R bar; the point glyph lights only columns 1 and 2, so it advances three columns instead of six and the widest readout keeps three dark columns from the bar
const POINT_COLUMNS: u8 = 3;
const POINT_LEAD_COLUMNS: u8 = 1;

const ALL_AREAS: [SeenRect; 15] = [
    CHIP_AREAS[0],
    CHIP_AREAS[1],
    CHIP_AREAS[2],
    CHIP_AREAS[3],
    DOT_AREA,
    PILL_AREA,
    RATE_AREA,
    READOUT_AREA,
    INPUT_AREAS[0],
    INPUT_AREAS[1],
    OUTPUT_AREAS[0],
    OUTPUT_AREAS[1],
    SCALE_AREA,
    LABELS_AREA,
    FAULT_AREA,
];

const fn every_byte_has_a_glyph(text: &[u8]) -> bool {
    let mut index = 0;
    while index < text.len() {
        if text[index] != NUMBER && !has_glyph(text[index]) {
            return false;
        }
        index += 1;
    }
    true
}

const fn text_width(characters: usize, scale: u8) -> u8 {
    if characters == 0 {
        return 0;
    }
    (characters as u8 * GLYPH_ADVANCE - 1) * scale
}

const fn shown_characters(pattern: &[u8]) -> usize {
    let mut characters = 0;
    let mut index = 0;
    while index < pattern.len() {
        characters += if pattern[index] == NUMBER {
            NUMBER_DIGITS
        } else {
            1
        };
        index += 1;
    }
    characters
}

const fn lines_fit(lines: &[&[u8]]) -> bool {
    let mut line = 0;
    while line < lines.len() {
        if !every_byte_has_a_glyph(lines[line])
            || text_width(shown_characters(lines[line]), 1) > READOUT_AREA.width
        {
            return false;
        }
        line += 1;
    }
    (lines.len() as u8 - 1) * READOUT_LINE_PITCH + GLYPH_HEIGHT <= READOUT_AREA.height
}

const _: () = {
    let mut index = 0;
    while index < ALL_AREAS.len() {
        assert!(ALL_AREAS[index].pixel_count() <= MAX_REGION_PIXELS);
        assert!(ALL_AREAS[index].x as usize + ALL_AREAS[index].width as usize <= 160);
        assert!(ALL_AREAS[index].y as usize + ALL_AREAS[index].height as usize <= 128);
        index += 1;
    }
    assert!(MOST_BATCH_DESCRIPTORS <= MOST_LCD_DESCRIPTORS);
    assert!(RATE_AREA.right() == EDGE_RIGHT);
    assert!(READOUT_AREA.right() == EDGE_RIGHT);
    assert!(CHIP_AREAS[3].right() == EDGE_RIGHT);
    assert!(PILL_AREA.x == 3);
    let mut rate = 0;
    while rate < SampleRate::ALL.len() {
        let text = rate_text(SampleRate::ALL[rate]);
        assert!(every_byte_has_a_glyph(text));
        assert!(text_width(text.len(), HERO_SCALE) <= RATE_AREA.width);
        rate += 1;
    }
    let mut chip = 0;
    while chip < CHIP_TEXT_OF.len() {
        assert!(every_byte_has_a_glyph(CHIP_TEXT_OF[chip]));
        assert!(text_width(CHIP_TEXT_OF[chip].len(), 1) + 2 <= CHIP_WIDTH);
        chip += 1;
    }
    assert!(every_byte_has_a_glyph(b"COSVITRAWUSREQ"));
    assert!(text_width(3, HERO_SCALE) + 4 <= PILL_AREA.width);
    assert!(every_byte_has_a_glyph(b"-0123456789."));
    assert!(
        text_width(MOST_READOUT_CHARACTERS, HERO_SCALE)
            - (GLYPH_ADVANCE - POINT_COLUMNS) * HERO_SCALE
            <= READOUT_AREA.width
    );
    assert!(OUTPUT_AREAS[1].right() + 3 < EDGE_RIGHT + 1 - readout_width(b"-60.0"));
    assert!(every_byte_has_a_glyph(PEAK_LABEL));
    assert!(every_byte_has_a_glyph(FAULT_WORD));
    assert!(every_byte_has_a_glyph(FAULT_STAGE));
    assert!(lines_fit(&RUNNING_WITH_A_CODE));
    assert!(lines_fit(&BOOTING_AFTER_A_RECORD));
    assert!(lines_fit(&BOOTING_AFTER_NO_RECORD));
    assert!(lines_fit(&BOOTING_WITH_A_CODE));
    assert!(text_width(3, FAULT_NUMBER_SCALE) <= FAULT_AREA.width);
    assert!(FAULT_STAGE_TOP + GLYPH_HEIGHT <= FAULT_AREA.height);
    let mut mark = 0;
    while mark < SCALE_MARKS.len() {
        let row = scale_row(SCALE_MARKS[mark].1);
        assert!(row >= SCALE_AREA.y + GLYPH_HEIGHT / 2);
        assert!(row + GLYPH_HEIGHT / 2 < SCALE_AREA.y + SCALE_AREA.height);
        assert!(text_width(SCALE_MARKS[mark].0.len(), 1) <= SCALE_TEXT_RIGHT + 1 - SCALE_AREA.x);
        mark += 1;
    }
};

const fn scale_row(db: i16) -> u8 {
    BAR_BOTTOM + 1 - Decibels::from_q8(db * 256).bar_pixels()
}

const fn rate_text(rate: SampleRate) -> &'static [u8] {
    match rate {
        SampleRate::Hz44100 => b"44.1k",
        SampleRate::Hz48000 => b"48k",
        SampleRate::Hz88200 => b"88.2k",
        SampleRate::Hz96000 => b"96k",
        SampleRate::Hz176400 => b"176.4k",
        SampleRate::Hz192000 => b"192k",
    }
}

impl Region {
    pub const ALL: [Region; 14] = [
        Region::PhantomChip1,
        Region::PhantomChip2,
        Region::MonitorChip1,
        Region::MonitorChip2,
        Region::Dot,
        Region::Pill,
        Region::Rate,
        Region::Readout,
        Region::Input1,
        Region::Input2,
        Region::OutputLeft,
        Region::OutputRight,
        Region::Scale,
        Region::Labels,
    ];

    const fn position(self) -> usize {
        match self {
            Region::PhantomChip1 => 0,
            Region::PhantomChip2 => 1,
            Region::MonitorChip1 => 2,
            Region::MonitorChip2 => 3,
            Region::Dot => 4,
            Region::Pill => 5,
            Region::Rate => 6,
            Region::Readout => 7,
            Region::Input1 => 8,
            Region::Input2 => 9,
            Region::OutputLeft => 10,
            Region::OutputRight => 11,
            Region::Scale => 12,
            Region::Labels => 13,
        }
    }

    const fn area(self) -> SeenRect {
        ALL_AREAS[self.position()]
    }

    pub const fn window(self) -> RamWindow {
        self.area().ram_window()
    }

    const fn bit(self) -> u16 {
        1 << self.position()
    }

    pub const fn following(self) -> Region {
        Region::ALL[(self.position() + 1) % Region::ALL.len()]
    }

    const fn bar(self) -> Option<usize> {
        match self {
            Region::Input1 => Some(0),
            Region::Input2 => Some(1),
            Region::OutputLeft => Some(2),
            Region::OutputRight => Some(3),
            Region::PhantomChip1
            | Region::PhantomChip2
            | Region::MonitorChip1
            | Region::MonitorChip2
            | Region::Dot
            | Region::Pill
            | Region::Rate
            | Region::Readout
            | Region::Scale
            | Region::Labels => None,
        }
    }

    fn differs(self, shown: &Screen, wanted: &Screen) -> bool {
        match self {
            Region::PhantomChip1 | Region::PhantomChip2 => {
                let input = self.position();
                shown.phantom[input] != wanted.phantom[input]
            }
            Region::MonitorChip1 | Region::MonitorChip2 => {
                let input = self.position() - 2;
                shown.monitoring[input] != wanted.monitoring[input]
            }
            Region::Dot => shown.meters.output_alert() != wanted.meters.output_alert(),
            Region::Pill => {
                shown.pill.label() != wanted.pill.label()
                    || shown.pill.colours() != wanted.pill.colours()
            }
            Region::Rate => {
                shown.rate != wanted.rate
                    || shown.clock != wanted.clock
                    || shown.reduced_bands != wanted.reduced_bands
            }
            Region::Readout => shown.readout_look() != wanted.readout_look(),
            Region::Input1 | Region::Input2 | Region::OutputLeft | Region::OutputRight => {
                let bar = self.position() - 8;
                BarLook::of(shown, bar) != BarLook::of(wanted, bar)
            }
            Region::Scale | Region::Labels => shown.profile != wanted.profile,
        }
    }
}

impl RegionSet {
    pub const EMPTY: Self = Self(0);
    pub const ALL: Self = Self((1 << Region::ALL.len()) - 1);

    pub const fn of(region: Region) -> Self {
        Self(region.bit())
    }

    pub const fn with(self, region: Region) -> Self {
        Self(self.0 | region.bit())
    }

    pub const fn union(self, other: RegionSet) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn without(self, region: Region) -> Self {
        Self(self.0 & !region.bit())
    }

    pub const fn contains(self, region: Region) -> bool {
        self.0 & region.bit() != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn first_from(self, start: Region) -> Option<Region> {
        let mut candidate = start;
        for _ in Region::ALL {
            if self.contains(candidate) {
                return Some(candidate);
            }
            candidate = candidate.following();
        }
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BarLook {
    lit_segments: u8,
    cap: u8,
    clip: bool,
    input: bool,
    profile: MeterProfileName,
}

impl BarLook {
    fn of(screen: &Screen, bar: usize) -> Self {
        let meter: Meter = screen.meters.meter(bar);
        let input = bar < 2;
        let profile = screen.profile.profile();
        let (lit, cap) = match profile.scale {
            MeterScale::TwoSlope => (
                meter.level().bar_pixels().div_ceil(SEGMENT_PITCH),
                meter.peak().bar_pixels(),
            ),
            MeterScale::Linear => (meter.level().linear_pixels(), meter.peak().linear_pixels()),
        };
        Self {
            lit_segments: lit,
            cap,
            clip: input && meter.alert_lit(),
            input,
            profile: screen.profile,
        }
    }

    pub const fn lit_segments(self) -> u8 {
        self.lit_segments
    }

    pub const fn cap(self) -> u8 {
        self.cap
    }

    const fn row_colour(self, y: u8) -> Rgb565 {
        if y < BAR_TOP {
            return if self.clip && y < CLIP_CAP_TOP + CLIP_CAP_ROWS {
                OVER
            } else {
                GROUND
            };
        }
        let height = BAR_BOTTOM + 1 - y;
        if self.cap != 0 && height <= self.cap && height + CAP_ROWS > self.cap {
            return PEAK_CAP;
        }
        let profile = self.profile.profile();
        let from_top = y - BAR_TOP;
        let lit = if profile.segmented {
            if (height - 1) % SEGMENT_PITCH == SEGMENT_PITCH - 1 {
                return GROUND;
            }
            (height - 1) / SEGMENT_PITCH < self.lit_segments
        } else {
            height <= self.lit_segments
        };
        if !lit {
            return if profile.segmented {
                GHOST
            } else if from_top == profile.gates[0].0 {
                profile.gates[0].1
            } else if from_top == profile.gates[1].0 {
                profile.gates[1].1
            } else {
                GROUND
            };
        }
        if from_top < profile.zone_rows[0] {
            profile.zone_colours[0]
        } else if from_top < profile.zone_rows[1] {
            profile.zone_colours[1]
        } else {
            profile.zone_colours[2]
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReadoutLook {
    Peak {
        tenths_always: bool,
        tenths: Option<i16>,
        colour: Rgb565,
    },
    Lines {
        lines: &'static [&'static [u8]],
        numbers: [u8; MOST_NUMBERS],
    },
}

const fn pill_target(badge: DspBadge, rate: SampleRate) -> PillTarget {
    let (label, colour, filled): (&'static [u8], Rgb565, bool) = match badge {
        DspBadge::Clarity => (b"EQ1", CLARITY, true),
        DspBadge::Warm => (b"EQ2", WARM, true),
        DspBadge::Raw | DspBadge::Bypass => (b"RAW", RAW, false),
        DspBadge::Custom => (b"USR", TEXT_2, true),
        DspBadge::On => (b"EQ", TEXT_2, true),
    };
    let off_at_this_rate = matches!(PeqAtRate::at(rate), PeqAtRate::OffAtThisRate);
    PillTarget {
        label,
        colours: if filled && !off_at_this_rate {
            PillColours::filled(colour)
        } else {
            PillColours::outlined(colour)
        },
    }
}

// WHY: Code 17 notes that the core stayed at 100 MHz; the audio, presets and screen work, so the screen stays clean and shows the 7B cue instead of the boot lines
const ADVISORY_CODE: u8 = 17;

const fn advisory(code: u8) -> bool {
    code == 0 || code == ADVISORY_CODE
}

impl Screen {
    pub const fn runs_clean(&self) -> bool {
        self.breadcrumb.latest == RUNNING.stage && advisory(self.breadcrumb.code)
    }

    pub const fn pill_target(&self) -> PillTarget {
        if self.runs_clean() {
            pill_target(self.dsp, self.rate)
        } else {
            NOT_RUNNING_PILL
        }
    }

    pub fn regions_changed_from(&self, shown: &Screen) -> RegionSet {
        Region::ALL
            .into_iter()
            .filter(|region| region.differs(shown, self))
            .fold(RegionSet::EMPTY, RegionSet::with)
    }

    pub fn bar_look(&self, bar: usize) -> BarLook {
        BarLook::of(self, bar)
    }

    fn readout_look(&self) -> ReadoutLook {
        let Breadcrumb {
            place,
            code,
            latest,
            ..
        } = self.breadcrumb;
        let (lines, numbers): (&'static [&'static [u8]], [u8; MOST_NUMBERS]) =
            match (latest == RUNNING.stage, code, self.previous_boot) {
                (true, code, _) if advisory(code) => {
                    let peak = self.meters.readout();
                    let tenths = (peak > Decibels::FLOOR).then_some(peak.tenths());
                    let profile = self.profile.profile();
                    let colour = if profile.readout_in_zone_colour {
                        match self.meters.readout_zone() {
                            Zone::Red => OBS_RED,
                            Zone::Yellow => OBS_YELLOW,
                            Zone::Green => OBS_GREEN,
                        }
                    } else if peak >= Decibels::OVER {
                        OVER
                    } else if matches!(self.meters.output_alert(), Alert::Limiting) {
                        HOT
                    } else {
                        TEXT
                    };
                    return ReadoutLook::Peak {
                        tenths,
                        colour,
                        tenths_always: profile.readout_in_zone_colour,
                    };
                }
                (true, _, _) => (&RUNNING_WITH_A_CODE, [code, place.stage, 0, 0]),
                (false, 0, Some(previous)) => (
                    &BOOTING_AFTER_A_RECORD,
                    [latest, previous.place.stage, previous.latest, previous.code],
                ),
                (false, 0, None) => (&BOOTING_AFTER_NO_RECORD, [latest, 0, 0, 0]),
                (false, _, _) => (&BOOTING_WITH_A_CODE, [latest, code, place.stage, 0]),
            };
        ReadoutLook::Lines { lines, numbers }
    }
}

pub struct RegionPixels([PanelWord; MAX_REGION_PIXELS]);

impl RegionPixels {
    pub const fn new() -> Self {
        Self([PanelWord::UNSENT; MAX_REGION_PIXELS])
    }

    pub fn words(&self, piece: Piece) -> &[PanelWord] {
        &self.0[piece.first as usize..piece.first as usize + piece.words as usize]
    }
}

impl Default for RegionPixels {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    pub window: RamWindow,
    pub first: u16,
    pub words: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Batch {
    pub pieces: [Option<Piece>; MOST_PIECES],
    pub leftover: RegionSet,
}

impl Batch {
    pub fn count(&self) -> usize {
        self.pieces.iter().flatten().count()
    }

    pub fn pixels(&self) -> usize {
        self.pieces
            .iter()
            .flatten()
            .map(|piece| piece.words as usize)
            .sum()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glass {
    bars: [Option<BarLook>; 4],
    next: Region,
}

impl Glass {
    pub const BLANK: Self = Self {
        bars: [None; 4],
        next: Region::ALL[0],
    };
}

#[inline(never)]
fn bar_span(area: SeenRect, shown: Option<BarLook>, wanted: BarLook) -> Option<SeenRect> {
    let Some(shown) = shown else {
        return Some(area);
    };
    let bottom = area.y + area.height - 1;
    let mut changed = (area.y..=bottom).filter(|&y| shown.row_colour(y) != wanted.row_colour(y));
    let top = changed.next()?;
    let last = changed.next_back().unwrap_or(top);
    Some(area.rows(top, last))
}

#[inline(never)]
pub fn render_batch(
    dirty: RegionSet,
    screen: &Screen,
    glass: &mut Glass,
    store: &mut RegionPixels,
) -> Batch {
    let mut batch = Batch {
        pieces: [None; MOST_PIECES],
        leftover: RegionSet::EMPTY,
    };
    let mut used = 0;
    let mut count = 0;
    let mut region = glass.next;
    let mut first_left = None;
    for _ in Region::ALL {
        if dirty.contains(region) {
            let wanted_bar = region.bar().map(|bar| (bar, BarLook::of(screen, bar)));
            let span = match wanted_bar {
                Some((bar, look)) => bar_span(region.area(), glass.bars[bar], look),
                None => Some(region.area()),
            };
            match span.map(|area| (area, area.checked_window())) {
                None | Some((_, None)) => {}
                Some((area, Some(window)))
                    if count < MOST_PIECES && used + area.pixel_count() <= MAX_REGION_PIXELS =>
                {
                    let words = &mut store.0[used..used + area.pixel_count()];
                    draw(region, area, screen, words);
                    if let Some(last) = words.last_mut() {
                        *last = last.ending();
                    }
                    batch.pieces[count] = Some(Piece {
                        window,
                        first: used as u16,
                        words: area.pixel_count() as u16,
                    });
                    used += area.pixel_count();
                    count += 1;
                    if let Some((bar, look)) = wanted_bar {
                        glass.bars[bar] = Some(look);
                    }
                }
                Some(_) => {
                    batch.leftover = batch.leftover.with(region);
                    first_left = first_left.or(Some(region));
                }
            }
        }
        region = region.following();
    }
    if let Some(region) = first_left {
        glass.next = region;
    }
    batch
}

struct Canvas<'a> {
    area: SeenRect,
    pixels: &'a mut [PanelWord],
}

impl<'a> Canvas<'a> {
    fn filled(area: SeenRect, pixels: &'a mut [PanelWord], colour: Rgb565) -> Self {
        pixels.fill(PanelWord::pixel(colour));
        Self { area, pixels }
    }

    fn put(&mut self, x: u8, y: u8, colour: Rgb565) {
        let (Some(dx), Some(dy)) = (x.checked_sub(self.area.x), y.checked_sub(self.area.y)) else {
            return;
        };
        if dx < self.area.width && dy < self.area.height {
            let ram_index =
                dx as usize * self.area.height as usize + (self.area.height - 1 - dy) as usize;
            if let Some(pixel) = self.pixels.get_mut(ram_index) {
                *pixel = PanelWord::pixel(colour);
            }
        }
    }

    fn fill(&mut self, x: u8, y: u8, width: u8, height: u8, colour: Rgb565) {
        for dx in 0..width {
            for dy in 0..height {
                self.put(x.saturating_add(dx), y.saturating_add(dy), colour);
            }
        }
    }

    fn rounded(&mut self, area: SeenRect, insets: &[u8], colour: Rgb565) {
        for dy in 0..area.height {
            let from_edge = dy.min(area.height - 1 - dy) as usize;
            let inset = insets.get(from_edge).copied().unwrap_or(0);
            self.fill(
                area.x + inset,
                area.y + dy,
                area.width - 2 * inset,
                1,
                colour,
            );
        }
    }

    fn glyph(&mut self, x: u8, y: u8, rows: GlyphRows, scale: u8, colour: Rgb565) {
        for row in 0..GLYPH_HEIGHT {
            for column in 0..GLYPH_WIDTH {
                if rows.lit(column, row) {
                    self.fill(
                        x.saturating_add(column * scale),
                        y.saturating_add(row * scale),
                        scale,
                        scale,
                        colour,
                    );
                }
            }
        }
    }

    #[inline(never)]
    fn text(&mut self, x: u8, y: u8, text: &[u8], scale: u8, colour: Rgb565) {
        let mut left = x;
        for &byte in text {
            self.glyph(left, y, character(byte), scale, colour);
            left = left.saturating_add(GLYPH_ADVANCE * scale);
        }
    }

    fn text_right(&mut self, right: u8, y: u8, text: &[u8], scale: u8, colour: Rgb565) {
        let width = text_width(text.len(), scale);
        self.text(right + 1 - width, y, text, scale, colour);
    }

    fn readout_right(&mut self, right: u8, y: u8, text: &[u8], colour: Rgb565) {
        let mut left = right + 1 - readout_width(text);
        for &byte in text {
            if byte == b'.' {
                let x = left.saturating_sub(POINT_LEAD_COLUMNS * HERO_SCALE);
                self.glyph(x, y, character(byte), HERO_SCALE, colour);
                left = left.saturating_add(POINT_COLUMNS * HERO_SCALE);
            } else {
                self.glyph(left, y, character(byte), HERO_SCALE, colour);
                left = left.saturating_add(GLYPH_ADVANCE * HERO_SCALE);
            }
        }
    }

    #[inline(never)]
    fn line(&mut self, x: u8, y: u8, pattern: &[u8], numbers: &mut impl Iterator<Item = u8>) {
        let mut shown = [0u8; 2 * MOST_NUMBERS + 8];
        let mut length = 0;
        for &byte in pattern {
            let digits = if byte == NUMBER {
                let number = numbers.next().unwrap_or(0).min(LARGEST_NUMBER);
                [b'0' + number / 10, b'0' + number % 10]
            } else {
                [byte, 0]
            };
            for digit in digits.into_iter().filter(|&digit| digit != 0) {
                if let Some(slot) = shown.get_mut(length) {
                    *slot = digit;
                    length += 1;
                }
            }
        }
        self.text(x, y, &shown[..length], 1, TEXT);
    }
}

#[inline(never)]
fn draw(region: Region, area: SeenRect, screen: &Screen, words: &mut [PanelWord]) {
    if let Some(bar) = region.bar() {
        draw_bar(area, BarLook::of(screen, bar), words);
        return;
    }
    let mut canvas = Canvas::filled(area, words, GROUND);
    match region {
        Region::PhantomChip1 | Region::PhantomChip2 => {
            let on = matches!(screen.phantom[region.position()], PhantomPower::On);
            draw_chip(
                &mut canvas,
                region.position(),
                on.then_some((PHANTOM_ON, TEXT)),
            );
        }
        Region::MonitorChip1 | Region::MonitorChip2 => {
            let on = matches!(screen.monitoring[region.position() - 2], Monitoring::On);
            draw_chip(&mut canvas, region.position(), on.then_some((TEXT, GROUND)));
        }
        Region::Dot => {
            let colour = match screen.meters.output_alert() {
                Alert::Quiet => None,
                Alert::Limiting => Some(HOT),
                Alert::Over => Some(OVER),
            };
            canvas.rounded(DOT_AREA, &[2, 1], colour.unwrap_or(DOT_RING));
            if colour.is_none() {
                canvas.rounded(
                    rect(
                        DOT_AREA.x + 1,
                        DOT_AREA.y + 1,
                        DOT_AREA.width - 2,
                        DOT_AREA.height - 2,
                    ),
                    &[1],
                    GROUND,
                );
            }
        }
        Region::Pill => {
            let colours = screen.pill.colours();
            canvas.rounded(PILL_AREA, &[2, 1], colours.line);
            canvas.rounded(
                rect(
                    PILL_AREA.x + 1,
                    PILL_AREA.y + 1,
                    PILL_AREA.width - 2,
                    PILL_AREA.height - 2,
                ),
                &[1],
                colours.fill,
            );
            let label = screen.pill.label();
            let width = text_width(label.len(), HERO_SCALE);
            canvas.text(
                PILL_AREA.x + (PILL_AREA.width - width) / 2,
                PILL_AREA.y + (PILL_AREA.height - GLYPH_HEIGHT * HERO_SCALE) / 2,
                label,
                HERO_SCALE,
                colours.ink,
            );
        }
        Region::Rate => {
            let colour = match screen.clock {
                ClockLock::Locked => TEXT,
                ClockLock::Changing => TEXT_2,
            };
            let text = rate_text(screen.rate);
            canvas.text_right(EDGE_RIGHT, RATE_AREA.y, text, HERO_SCALE, colour);
            if let Some(bands) = screen.reduced_bands {
                let cue = [b'0' + bands.min(9), b'B'];
                let right = EDGE_RIGHT - text_width(text.len(), HERO_SCALE) - REDUCED_BANDS_GAP;
                canvas.text_right(
                    right,
                    RATE_AREA.y + (RATE_AREA.height - GLYPH_HEIGHT) / 2,
                    &cue,
                    1,
                    REDUCED_BANDS,
                );
            }
        }
        Region::Readout => match screen.readout_look() {
            ReadoutLook::Peak {
                tenths,
                colour,
                tenths_always,
            } => {
                let mut digits = [0u8; MOST_READOUT_CHARACTERS];
                let text = match (tenths, tenths_always) {
                    (Some(tenths), true) => readout_tenths_text(tenths, &mut digits),
                    (Some(tenths), false) => readout_text(tenths, &mut digits),
                    (None, _) => SILENT_READOUT,
                };
                canvas.readout_right(EDGE_RIGHT, READOUT_AREA.y, text, colour);
                canvas.text_right(EDGE_RIGHT, READOUT_AREA.y + 18, PEAK_LABEL, 1, TEXT_2);
            }
            ReadoutLook::Lines { lines, numbers } => {
                let mut numbers = numbers.into_iter();
                for (index, pattern) in lines.iter().enumerate() {
                    canvas.line(
                        READOUT_AREA.x,
                        READOUT_AREA.y + index as u8 * READOUT_LINE_PITCH,
                        pattern,
                        &mut numbers,
                    );
                }
            }
        },
        Region::Scale => {
            for mark in screen.profile.profile().marks {
                if mark.text.is_empty() {
                    continue;
                }
                let row = BAR_TOP + mark.row;
                canvas.text_right(
                    SCALE_TEXT_RIGHT,
                    row - GLYPH_HEIGHT / 2,
                    mark.text,
                    1,
                    mark.text_colour,
                );
                canvas.fill(SCALE_TICK_LEFT, row, SCALE_TICK_WIDTH, 1, mark.tick_colour);
            }
        }
        Region::Labels => {
            let bars = [
                INPUT_AREAS[0],
                INPUT_AREAS[1],
                OUTPUT_AREAS[0],
                OUTPUT_AREAS[1],
            ];
            for (bar, label) in bars.into_iter().zip(LABEL_TEXT) {
                canvas.text(
                    bar.x + (bar.width - GLYPH_WIDTH) / 2,
                    LABELS_AREA.y,
                    label,
                    1,
                    TEXT_2,
                );
            }
        }
        Region::Input1 | Region::Input2 | Region::OutputLeft | Region::OutputRight => {}
    }
}

fn draw_chip(canvas: &mut Canvas<'_>, chip: usize, on: Option<(Rgb565, Rgb565)>) {
    let area = CHIP_AREAS[chip];
    let ink = match on {
        Some((fill, ink)) => {
            canvas.rounded(area, &[1], fill);
            ink
        }
        None => {
            canvas.rounded(area, &[1], CHIP_LINE);
            canvas.fill(
                area.x + 1,
                area.y + 1,
                area.width - 2,
                area.height - 2,
                GROUND,
            );
            CHIP_TEXT
        }
    };
    let text = CHIP_TEXT_OF[chip];
    let width = text_width(text.len(), 1);
    canvas.text(
        area.x + (area.width - width) / 2,
        area.y + (area.height - GLYPH_HEIGHT) / 2,
        text,
        1,
        ink,
    );
}

#[inline(never)]
fn draw_bar(area: SeenRect, look: BarLook, words: &mut [PanelWord]) {
    let mut rows = [PanelWord::UNSENT; (BAR_BOTTOM - CLIP_CAP_TOP + 1) as usize];
    let height = area.height as usize;
    for (slot, y) in rows.iter_mut().zip((area.y..area.y + area.height).rev()) {
        *slot = PanelWord::pixel(look.row_colour(y));
    }
    for column in words.chunks_exact_mut(height) {
        column.copy_from_slice(&rows[..height]);
    }
}

const fn readout_width(text: &[u8]) -> u8 {
    let mut width = 0;
    let mut index = 0;
    while index < text.len() {
        width += if text[index] == b'.' {
            POINT_COLUMNS * HERO_SCALE
        } else {
            GLYPH_ADVANCE * HERO_SCALE
        };
        index += 1;
    }
    width.saturating_sub(HERO_SCALE)
}

pub const fn readout_left_edge(text: &[u8]) -> u8 {
    EDGE_RIGHT + 1 - readout_width(text)
}

pub const R_BAR_RIGHT_EDGE: u8 = OUTPUT_AREAS[1].x + OUTPUT_AREAS[1].width - 1;

fn readout_tenths_text(tenths: i16, digits: &mut [u8; MOST_READOUT_CHARACTERS]) -> &[u8] {
    let magnitude = tenths.unsigned_abs();
    let mut length = 0;
    let mut push = |byte: u8| {
        if let Some(slot) = digits.get_mut(length) {
            *slot = byte;
            length += 1;
        }
    };
    if tenths < 0 {
        push(b'-');
    }
    if magnitude >= 100 {
        push(b'0' + (magnitude / 100 % 10) as u8);
    }
    push(b'0' + (magnitude / 10 % 10) as u8);
    push(b'.');
    push(b'0' + (magnitude % 10) as u8);
    &digits[..length]
}

fn readout_text(tenths: i16, digits: &mut [u8; MOST_READOUT_CHARACTERS]) -> &[u8] {
    let magnitude = tenths.unsigned_abs();
    let sign: &[u8] = if tenths < 0 { b"-" } else { b"" };
    let mut length = 0;
    let mut push = |byte: u8| {
        if let Some(slot) = digits.get_mut(length) {
            *slot = byte;
            length += 1;
        }
    };
    sign.iter().for_each(|&byte| push(byte));
    if magnitude < 100 {
        push(b'0' + (magnitude / 10) as u8);
        push(b'.');
        push(b'0' + (magnitude % 10) as u8);
    } else {
        let whole = (magnitude + 5) / 10;
        if whole >= 10 {
            push(b'0' + (whole / 10 % 10) as u8);
        }
        push(b'0' + (whole % 10) as u8);
    }
    &digits[..length]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FatalScreen {
    NotStarted,
    Started,
    Attempted,
}

impl FatalScreen {
    pub const fn from_bits(bits: u8) -> Self {
        match bits {
            0 => Self::NotStarted,
            1 => Self::Started,
            _ => Self::Attempted,
        }
    }

    pub const fn started(self) -> Self {
        match self {
            Self::NotStarted => Self::Started,
            Self::Started | Self::Attempted => self,
        }
    }

    pub const fn attempted(self) -> (Self, bool) {
        (Self::Attempted, matches!(self, Self::Started))
    }
}

pub const FAULT_WINDOW: RamWindow = FAULT_AREA.ram_window();
pub const LARGEST_REGION_SPI_BYTES: SpiByteCount =
    SpiByteCount((MAX_REGION_PIXELS as u32) * 2 + MOST_PIECES as u32 * 11);
pub const FULL_SCREEN_CLEAR_SPI_BYTES: SpiByteCount = RamWindow::FULL.transfer_bytes();

const FAULT_NUMBER_TOP: u8 = 1 + GLYPH_HEIGHT + 2;
const FAULT_STAGE_TOP: u8 = FAULT_NUMBER_TOP + GLYPH_HEIGHT * FAULT_NUMBER_SCALE + 2;

pub fn render_fault(code: FaultCode, stage: u8, store: &mut RegionPixels) -> &[PanelWord] {
    let pixels = &mut store.0[..FAULT_AREA.pixel_count()];
    let mut canvas = Canvas::filled(FAULT_AREA, pixels, FAULT_BACKGROUND);
    let centre = |characters: usize, scale: u8| {
        FAULT_AREA.x + (FAULT_AREA.width - text_width(characters, scale)) / 2
    };
    canvas.text(
        centre(FAULT_WORD.len(), 1),
        FAULT_AREA.y + 1,
        FAULT_WORD,
        1,
        FAULT_TEXT,
    );
    let mut digits = [0u8; 3];
    let number = number_text(code.number(), &mut digits);
    canvas.text(
        centre(number.len(), FAULT_NUMBER_SCALE),
        FAULT_AREA.y + FAULT_NUMBER_TOP,
        number,
        FAULT_NUMBER_SCALE,
        FAULT_TEXT,
    );
    canvas.line(
        centre(shown_characters(FAULT_STAGE), 1),
        FAULT_AREA.y + FAULT_STAGE_TOP,
        FAULT_STAGE,
        &mut [stage].into_iter(),
    );
    let pixels = canvas.pixels;
    if let Some(last) = pixels.last_mut() {
        *last = last.ending();
    }
    pixels
}

fn number_text(number: u8, digits: &mut [u8; 3]) -> &[u8] {
    let hundreds = number / 100;
    let tens = number / 10 % 10;
    let ones = number % 10;
    *digits = [b'0' + hundreds, b'0' + tens, b'0' + ones];
    let leading_zeros = match (hundreds, tens) {
        (0, 0) => 2,
        (0, _) => 1,
        _ => 0,
    };
    &digits[leading_zeros..]
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    fn shown(tenths: i16) -> std::string::String {
        let mut digits = [0u8; MOST_READOUT_CHARACTERS];
        std::string::String::from_utf8(readout_text(tenths, &mut digits).to_vec()).unwrap()
    }

    #[test]
    fn the_readout_shows_tenths_near_full_scale_and_whole_decibels_below_minus_ten() {
        for (tenths, text) in [
            (0, "0.0"),
            (-1, "-0.1"),
            (-7, "-0.7"),
            (-99, "-9.9"),
            (-100, "-10"),
            (-124, "-12"),
            (-125, "-13"),
            (-599, "-60"),
        ] {
            assert_eq!(shown(tenths), text, "{tenths}");
        }
    }
}
