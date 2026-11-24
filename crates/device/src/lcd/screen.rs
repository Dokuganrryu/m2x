use dsp::{DspMode, PeqAtRate, SampleRate};

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

impl DspBadge {
    const fn mode(self) -> DspMode {
        match self {
            Self::Bypass | Self::Raw => DspMode::Bypass,
            Self::On | Self::Clarity | Self::Warm | Self::Custom => DspMode::On,
        }
    }

    #[cfg(feature = "sound2")]
    // WHY: A badge readable across the desk: a filled box in the preset's colour with its name in the background colour, orange for warm, green for clarity, white for bit-perfect flat
    const fn label(self) -> Option<(&'static [u8], Rgb565)> {
        match self {
            Self::Bypass | Self::On => None,
            Self::Clarity => Some((b"EQ1", ACTIVE)),
            Self::Warm => Some((b"EQ2", METER_LIT_ABOVE_KNEE)),
            Self::Raw => Some((b"RAW", TEXT)),
            Self::Custom => Some((b"USR", DIM)),
        }
    }
}

use crate::boot::RUNNING;
use crate::breadcrumb::Breadcrumb;
use crate::fault::FaultCode;
use crate::lcd::glyphs::{
    GLYPH_ADVANCE, GLYPH_HEIGHT, GLYPH_WIDTH, GlyphRows, Icon, character, has_glyph,
};
pub use crate::lcd::meter::MeterFrame;
use crate::lcd::meter::{METER_CHANNELS, MeterDisplay, MeterLevel};
use crate::lcd::panel::{PANEL_COLUMNS, RamWindow, SpiByteCount};
use crate::lcd::wire::PanelWord;
use crate::phantom::PhantomPower;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb565(u16);

impl Rgb565 {
    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u16 {
        self.0
    }
}

pub const BACKGROUND: Rgb565 = Rgb565(0x0841);
pub const FAULT_BACKGROUND: Rgb565 = Rgb565(0x8000);
const TEXT: Rgb565 = Rgb565(0xFFFF);
const DIM: Rgb565 = Rgb565(0x4208);
const ACTIVE: Rgb565 = Rgb565(0x07E0);
const PHANTOM_ON: Rgb565 = Rgb565(0xF800);
const METER_LIT: Rgb565 = Rgb565(0x07E0);
const METER_LIT_ABOVE_KNEE: Rgb565 = Rgb565(0xFD20);
const METER_UNLIT: Rgb565 = Rgb565(0x2104);
const CLIP_LIT: Rgb565 = Rgb565(0xF800);
const CLIP_UNLIT: Rgb565 = Rgb565(0x2104);
const FAULT_TEXT: Rgb565 = Rgb565(0xFFFF);

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Screen {
    pub rate: SampleRate,
    pub clock: ClockLock,
    pub playback: StreamActivity,
    pub capture: StreamActivity,
    pub phantom: [PhantomPower; 2],
    pub meters: MeterFrame,
    pub dsp: DspBadge,
    pub breadcrumb: Breadcrumb,
    pub previous_boot: Option<Breadcrumb>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    Rate,
    Stream,
    Phantom,
    Meters,
    Legend,
    Dsp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionSet(u8);

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

    const fn pixel_count(self) -> usize {
        self.width as usize * self.height as usize
    }
}

const RATE_AREA: SeenRect = SeenRect {
    x: 0,
    y: 0,
    width: 64,
    height: 11,
};
const STREAM_AREA: SeenRect = SeenRect {
    x: 112,
    y: 0,
    width: 48,
    height: 11,
};
const PHANTOM_AREA: SeenRect = SeenRect {
    x: 2,
    y: 40,
    width: 40,
    height: 20,
};
const METERS_AREA: SeenRect = SeenRect {
    x: 50,
    y: 14,
    width: 58,
    height: 88,
};
const LEGEND_AREA: SeenRect = SeenRect {
    x: 50,
    y: 104,
    width: 58,
    height: 9,
};
const DSP_AREA: SeenRect = SeenRect {
    x: 0,
    y: 116,
    width: 160,
    height: 12,
};
const FAULT_AREA: SeenRect = SeenRect {
    x: 44,
    y: 29,
    width: 72,
    height: 70,
};

const METER_BAR_WIDTH: u8 = 12;
const METER_BAR_LEFT_EDGES: [u8; 4] = [0, 14, 32, 46];
const CLIP_BOX_HEIGHT: u8 = 8;
const METER_BAR_TOP: u8 = CLIP_BOX_HEIGHT + 2;
const METER_BAR_PIXELS: u8 = MeterLevel::TOP.steps() + 2;

const FAULT_WORD_SCALE: u8 = 2;
const FAULT_NUMBER_SCALE: u8 = 4;
const FAULT_STAGE_SCALE: u8 = 2;

pub const MAX_REGION_PIXELS: usize = largest_area_pixels();

const ALL_AREAS: [SeenRect; 7] = [
    RATE_AREA,
    STREAM_AREA,
    PHANTOM_AREA,
    METERS_AREA,
    LEGEND_AREA,
    DSP_AREA,
    FAULT_AREA,
];

const fn largest_area_pixels() -> usize {
    let mut largest = 0;
    let mut index = 0;
    while index < ALL_AREAS.len() {
        let pixels = ALL_AREAS[index].pixel_count();
        if pixels > largest {
            largest = pixels;
        }
        index += 1;
    }
    largest
}

const PHANTOM_TEXT: &[u8] = b"48V";
const BYPASS_TEXT: &[u8] = b"BYPASS";
const PEQ_ON_TEXT: &[u8] = b"PEQ ON";
const PEQ_OFF_AT_THIS_RATE_TEXT: &[u8] = b"PEQ OFF AT THIS RATE";
const BIT_PERFECT_TEXT: &[u8] = b"BIT PERFECT";
const BADGE_CHARACTERS: u8 = 3;
const BADGE_PADDING: u8 = 2;
const BADGE_WIDTH: u8 = BADGE_CHARACTERS * GLYPH_ADVANCE + 2 * BADGE_PADDING - 1;
const BADGE_HEIGHT: u8 = GLYPH_HEIGHT + 2 * BADGE_PADDING;
const BADGE_GAP: u8 = 4;
const FAULT_WORD: &[u8] = b"FAULT";
const NUMBER: u8 = b'#';
const NUMBER_DIGITS: usize = 2;
const LARGEST_NUMBER: u8 = 99;
const BOOTING_AFTER_A_RECORD: &[u8] = b"S#  P#/# C#";
const BOOTING_AFTER_NO_RECORD: &[u8] = b"S#  P--";
const BOOTING_WITH_A_CODE: &[u8] = b"S#  C#@#";
const RUNNING_WITH_A_CODE: &[u8] = b"C#@#";
const FAULT_STAGE: &[u8] = b"@#";
const LEGEND_TEXT: [&[u8]; 4] = [b"1", b"2", b"L", b"R"];
const INPUT_NAMES: [&[u8]; 2] = [b"1", b"2"];

const fn every_byte_has_a_glyph(text: &[u8]) -> bool {
    let mut index = 0;
    while index < text.len() {
        if !has_glyph(text[index]) {
            return false;
        }
        index += 1;
    }
    true
}

const _: () = {
    assert!(every_byte_has_a_glyph(PHANTOM_TEXT));
    assert!(every_byte_has_a_glyph(BYPASS_TEXT));
    assert!(every_byte_has_a_glyph(PEQ_ON_TEXT));
    assert!(every_byte_has_a_glyph(PEQ_OFF_AT_THIS_RATE_TEXT));
    assert!(every_byte_has_a_glyph(BIT_PERFECT_TEXT));
    assert!(every_byte_has_a_glyph(b"COSVITRAWUSR"));
    assert!(BADGE_HEIGHT <= DSP_AREA.height);
    assert!(
        TEXT_INSET + BADGE_WIDTH + BADGE_GAP + text_width(PEQ_OFF_AT_THIS_RATE_TEXT, 1)
            <= DSP_AREA.width
    );
    assert!(every_byte_has_a_glyph(FAULT_WORD));
    let mut rate = 0;
    while rate < SampleRate::ALL.len() {
        assert!(every_byte_has_a_glyph(rate_text(SampleRate::ALL[rate])));
        rate += 1;
    }
    assert!(text_width(PEQ_OFF_AT_THIS_RATE_TEXT, 1) <= DSP_AREA.width);
    assert!(text_width(FAULT_WORD, FAULT_WORD_SCALE) <= FAULT_AREA.width);
    assert!(!has_glyph(NUMBER));
    let patterns = [
        BOOTING_AFTER_A_RECORD,
        BOOTING_AFTER_NO_RECORD,
        BOOTING_WITH_A_CODE,
        RUNNING_WITH_A_CODE,
    ];
    let mut pattern = 0;
    while pattern < patterns.len() {
        assert!(every_byte_has_a_glyph_or_is_a_number(patterns[pattern]));
        assert!(numbers_in(patterns[pattern]) <= MOST_NUMBERS);
        assert!(TEXT_INSET + pattern_width(patterns[pattern], 1) <= DSP_AREA.width);
        pattern += 1;
    }
    assert!(every_byte_has_a_glyph_or_is_a_number(FAULT_STAGE));
    assert!(pattern_width(FAULT_STAGE, FAULT_STAGE_SCALE) <= FAULT_AREA.width);
    assert!(FAULT_STAGE_TOP + GLYPH_HEIGHT * FAULT_STAGE_SCALE <= FAULT_AREA.height);
};

const fn numbers_in(pattern: &[u8]) -> usize {
    let mut numbers = 0;
    let mut index = 0;
    while index < pattern.len() {
        if pattern[index] == NUMBER {
            numbers += 1;
        }
        index += 1;
    }
    numbers
}

const fn every_byte_has_a_glyph_or_is_a_number(pattern: &[u8]) -> bool {
    let mut index = 0;
    while index < pattern.len() {
        if pattern[index] != NUMBER && !has_glyph(pattern[index]) {
            return false;
        }
        index += 1;
    }
    true
}

const fn text_width(text: &[u8], scale: u8) -> u8 {
    (text.len() as u8 * GLYPH_ADVANCE - 1) * scale
}

const fn pattern_width(pattern: &[u8], scale: u8) -> u8 {
    let characters = pattern.len() + numbers_in(pattern) * (NUMBER_DIGITS - 1);
    (characters as u8 * GLYPH_ADVANCE - 1) * scale
}

const MOST_NUMBERS: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq)]
struct TextLine<'a> {
    pattern: &'a [u8],
    numbers: [u8; MOST_NUMBERS],
}

impl Screen {
    #[cfg(feature = "sound2")]
    fn badge(&self) -> Option<(&'static [u8], Rgb565)> {
        self.dsp.label().filter(|_| self.runs_clean())
    }

    // WHY: Only sound2 shows a preset, so the other images keep their old screen code and size
    #[cfg(not(feature = "sound2"))]
    fn badge(&self) -> Option<(&'static [u8], Rgb565)> {
        None
    }

    #[cfg(feature = "sound2")]
    const fn runs_clean(&self) -> bool {
        self.breadcrumb.latest == RUNNING.stage && self.breadcrumb.code == 0
    }

    #[inline(never)]
    const fn text_line(&self) -> TextLine<'static> {
        let Breadcrumb {
            place,
            code,
            latest,
            ..
        } = self.breadcrumb;
        let (pattern, numbers) = match (latest == RUNNING.stage, code, self.previous_boot) {
            (true, 0, _) => (dsp_text(self.dsp, self.rate), [0; MOST_NUMBERS]),
            (true, _, _) => (RUNNING_WITH_A_CODE, [code, place.stage, 0, 0]),
            (false, 0, Some(previous)) => (
                BOOTING_AFTER_A_RECORD,
                [latest, previous.place.stage, previous.latest, previous.code],
            ),
            (false, 0, None) => (BOOTING_AFTER_NO_RECORD, [latest, 0, 0, 0]),
            (false, _, _) => (BOOTING_WITH_A_CODE, [latest, code, place.stage, 0]),
        };
        TextLine { pattern, numbers }
    }
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

const fn dsp_text(dsp: DspBadge, rate: SampleRate) -> &'static [u8] {
    if matches!(dsp, DspBadge::Raw) {
        return BIT_PERFECT_TEXT;
    }
    match (dsp.mode(), PeqAtRate::at(rate)) {
        (DspMode::Bypass, PeqAtRate::Runs | PeqAtRate::OffAtThisRate) => BYPASS_TEXT,
        (DspMode::On, PeqAtRate::Runs) => PEQ_ON_TEXT,
        (DspMode::On, PeqAtRate::OffAtThisRate) => PEQ_OFF_AT_THIS_RATE_TEXT,
    }
}

impl Region {
    pub const ALL: [Region; 6] = [
        Region::Rate,
        Region::Stream,
        Region::Phantom,
        Region::Meters,
        Region::Legend,
        Region::Dsp,
    ];

    const fn area(self) -> SeenRect {
        match self {
            Region::Rate => RATE_AREA,
            Region::Stream => STREAM_AREA,
            Region::Phantom => PHANTOM_AREA,
            Region::Meters => METERS_AREA,
            Region::Legend => LEGEND_AREA,
            Region::Dsp => DSP_AREA,
        }
    }

    pub const fn window(self) -> RamWindow {
        self.area().ram_window()
    }

    const fn bit(self) -> u8 {
        1 << self.position()
    }

    const fn position(self) -> usize {
        match self {
            Region::Rate => 0,
            Region::Stream => 1,
            Region::Phantom => 2,
            Region::Meters => 3,
            Region::Legend => 4,
            Region::Dsp => 5,
        }
    }

    pub const fn following(self) -> Region {
        Region::ALL[(self.position() + 1) % Region::ALL.len()]
    }

    fn differs(self, shown: &Screen, wanted: &Screen) -> bool {
        match self {
            Region::Rate => shown.rate != wanted.rate || shown.clock != wanted.clock,
            Region::Stream => shown.playback != wanted.playback || shown.capture != wanted.capture,
            Region::Phantom => shown.phantom != wanted.phantom,
            Region::Meters => !shown.meters.looks_like(wanted.meters),
            Region::Legend => false,
            Region::Dsp => {
                shown.text_line() != wanted.text_line() || shown.badge() != wanted.badge()
            }
        }
    }
}

pub const FAULT_WINDOW: RamWindow = FAULT_AREA.ram_window();
pub const METER_FRAME_SPI_BYTES: SpiByteCount = METERS_AREA.ram_window().transfer_bytes();
pub const LARGEST_REGION_SPI_BYTES: SpiByteCount = largest_region_transfer();
pub const FULL_SCREEN_CLEAR_SPI_BYTES: SpiByteCount = RamWindow::FULL.transfer_bytes();

const fn largest_region_transfer() -> SpiByteCount {
    let mut largest = 0;
    let mut index = 0;
    while index < ALL_AREAS.len() {
        let bytes = ALL_AREAS[index].ram_window().transfer_bytes().0;
        if bytes > largest {
            largest = bytes;
        }
        index += 1;
    }
    SpiByteCount(largest)
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

impl Screen {
    pub fn regions_changed_from(&self, shown: &Screen) -> RegionSet {
        Region::ALL
            .into_iter()
            .filter(|region| region.differs(shown, self))
            .fold(RegionSet::EMPTY, RegionSet::with)
    }
}

pub struct RegionPixels([PanelWord; MAX_REGION_PIXELS]);

impl RegionPixels {
    pub const fn new() -> Self {
        Self([PanelWord::UNSENT; MAX_REGION_PIXELS])
    }
}

impl Default for RegionPixels {
    fn default() -> Self {
        Self::new()
    }
}

struct Canvas<'a> {
    area: SeenRect,
    pixels: &'a mut [PanelWord],
}

impl<'a> Canvas<'a> {
    fn filled(area: SeenRect, store: &'a mut RegionPixels, colour: Rgb565) -> Self {
        let pixels = &mut store.0[..area.pixel_count()];
        pixels.fill(PanelWord::pixel(colour));
        Self { area, pixels }
    }

    fn put(&mut self, x: u8, y: u8, colour: Rgb565) {
        if x < self.area.width && y < self.area.height {
            let ram_index =
                x as usize * self.area.height as usize + (self.area.height - 1 - y) as usize;
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
        self.line(
            x,
            y,
            TextLine {
                pattern: text,
                numbers: [0; MOST_NUMBERS],
            },
            scale,
            colour,
        );
    }

    #[inline(never)]
    fn line(&mut self, x: u8, y: u8, line: TextLine<'_>, scale: u8, colour: Rgb565) {
        let mut left = x;
        let mut pattern = line.pattern.iter();
        let mut numbers = line.numbers.into_iter();
        let mut ones = None;
        loop {
            let byte = if let Some(ones) = ones.take() {
                ones
            } else {
                match pattern.next() {
                    None => break,
                    Some(&NUMBER) => {
                        let number = numbers.next().unwrap_or(0).min(LARGEST_NUMBER);
                        ones = Some(b'0' + number % 10);
                        b'0' + number / 10
                    }
                    Some(&byte) => byte,
                }
            };
            self.glyph(left, y, character(byte), scale, colour);
            left = left.saturating_add(GLYPH_ADVANCE * scale);
        }
    }

    fn into_transfer(self) -> &'a [PanelWord] {
        if let Some(last) = self.pixels.last_mut() {
            *last = last.ending();
        }
        self.pixels
    }
}

const fn activity_colour(activity: StreamActivity) -> Rgb565 {
    match activity {
        StreamActivity::Idle => DIM,
        StreamActivity::Active => ACTIVE,
    }
}

const fn phantom_colour(power: PhantomPower) -> Rgb565 {
    match power {
        PhantomPower::Off => DIM,
        PhantomPower::On => PHANTOM_ON,
    }
}

const fn meter_pixel_colour(step: u8, display: MeterDisplay) -> Rgb565 {
    if step > display.level().steps() {
        METER_UNLIT
    } else if step > MeterLevel::FACTORY_KNEE.steps() {
        METER_LIT_ABOVE_KNEE
    } else {
        METER_LIT
    }
}

const fn clip_colour(display: MeterDisplay) -> Rgb565 {
    if display.clip_lit() {
        CLIP_LIT
    } else {
        CLIP_UNLIT
    }
}

fn draw_meter(canvas: &mut Canvas<'_>, left: u8, display: MeterDisplay) {
    canvas.fill(
        left,
        0,
        METER_BAR_WIDTH,
        CLIP_BOX_HEIGHT,
        clip_colour(display),
    );
    let bottom = METER_BAR_TOP + METER_BAR_PIXELS - 1;
    for step in 0..METER_BAR_PIXELS {
        canvas.fill(
            left,
            bottom - step,
            METER_BAR_WIDTH,
            1,
            meter_pixel_colour(step, display),
        );
    }
}

const TEXT_INSET: u8 = 2;
const LINE_PITCH: u8 = 10;

pub fn render<'a>(region: Region, screen: &Screen, store: &'a mut RegionPixels) -> &'a [PanelWord] {
    let mut canvas = Canvas::filled(region.area(), store, BACKGROUND);
    match region {
        Region::Rate => {
            let (text_colour, lock) = match screen.clock {
                ClockLock::Locked => (TEXT, Some(Icon::Lock)),
                ClockLock::Changing => (DIM, None),
            };
            canvas.text(
                TEXT_INSET,
                TEXT_INSET,
                rate_text(screen.rate),
                1,
                text_colour,
            );
            if let Some(icon) = lock {
                canvas.glyph(46, TEXT_INSET, icon.rows(), 1, TEXT);
            }
        }
        Region::Stream => {
            canvas.glyph(
                8,
                TEXT_INSET,
                Icon::Play.rows(),
                1,
                activity_colour(screen.playback),
            );
            canvas.glyph(
                28,
                TEXT_INSET,
                Icon::Record.rows(),
                1,
                activity_colour(screen.capture),
            );
        }
        Region::Phantom => {
            for (line, (name, power)) in INPUT_NAMES.iter().zip(screen.phantom).enumerate() {
                let y = 1 + line as u8 * LINE_PITCH;
                canvas.text(0, y, name, 1, TEXT);
                canvas.text(2 * GLYPH_ADVANCE, y, PHANTOM_TEXT, 1, phantom_colour(power));
            }
        }
        Region::Meters => {
            for (left, channel) in METER_BAR_LEFT_EDGES.into_iter().zip(METER_CHANNELS) {
                draw_meter(&mut canvas, left, screen.meters.channel(channel));
            }
        }
        Region::Legend => {
            for (left, label) in METER_BAR_LEFT_EDGES.into_iter().zip(LEGEND_TEXT) {
                canvas.text(left + (METER_BAR_WIDTH - GLYPH_WIDTH) / 2, 1, label, 1, DIM);
            }
        }
        Region::Dsp => match screen.badge() {
            Some((label, colour)) => {
                canvas.fill(TEXT_INSET, 0, BADGE_WIDTH, BADGE_HEIGHT, colour);
                canvas.text(
                    TEXT_INSET + BADGE_PADDING,
                    BADGE_PADDING,
                    label,
                    1,
                    BACKGROUND,
                );
                canvas.line(
                    TEXT_INSET + BADGE_WIDTH + BADGE_GAP,
                    3,
                    screen.text_line(),
                    1,
                    TEXT,
                );
            }
            None => {
                canvas.line(TEXT_INSET, 3, screen.text_line(), 1, TEXT);
            }
        },
    }
    canvas.into_transfer()
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

const FAULT_NUMBER_TOP: u8 = 2 + GLYPH_HEIGHT * FAULT_WORD_SCALE + 6;
const FAULT_STAGE_TOP: u8 = FAULT_NUMBER_TOP + GLYPH_HEIGHT * FAULT_NUMBER_SCALE + 4;

pub fn render_fault(code: FaultCode, stage: u8, store: &mut RegionPixels) -> &[PanelWord] {
    let mut canvas = Canvas::filled(FAULT_AREA, store, FAULT_BACKGROUND);
    let word_width = text_width(FAULT_WORD, FAULT_WORD_SCALE);
    canvas.text(
        (FAULT_AREA.width - word_width) / 2,
        2,
        FAULT_WORD,
        FAULT_WORD_SCALE,
        FAULT_TEXT,
    );
    let mut digits = [0u8; 3];
    let number = number_text(code.number(), &mut digits);
    let number_width = text_width(number, FAULT_NUMBER_SCALE);
    canvas.text(
        FAULT_AREA.width.saturating_sub(number_width) / 2,
        FAULT_NUMBER_TOP,
        number,
        FAULT_NUMBER_SCALE,
        FAULT_TEXT,
    );
    canvas.line(
        (FAULT_AREA.width - pattern_width(FAULT_STAGE, FAULT_STAGE_SCALE)) / 2,
        FAULT_STAGE_TOP,
        TextLine {
            pattern: FAULT_STAGE,
            numbers: [stage, 0, 0, 0],
        },
        FAULT_STAGE_SCALE,
        FAULT_TEXT,
    );
    canvas.into_transfer()
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
