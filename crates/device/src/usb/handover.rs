#![deny(clippy::wildcard_enum_match_arm)]

use core::sync::atomic::{AtomicU32, Ordering};

use dsp::SampleRate;

use super::request::DataTerminal;
use super::service::Notice;
use crate::outputs::OutputList;
use crate::rate_change::{self, ClockReport, ClockValidity};
use crate::supervisor;

pub const MOST_NOTICES_PER_PASS: usize = 7;

pub type PassNotices = OutputList<Notice, MOST_NOTICES_PER_PASS>;

const LEVEL_MASK: u32 = 1;
const EDGE_STEP: u32 = 2;
const EDGE_COUNT_MASK: u32 = u32::MAX >> 1;
const RATE_CODE_MASK: u32 = 0b111;
const GENERATION_STEP: u32 = 0b1000;
const INVALID_BIT: u32 = 0b1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Level {
    Low,
    High,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum News {
    Quiet,
    Settled(Level),
    Crossed(Level),
}

#[derive(Debug)]
struct LevelWord(AtomicU32);

impl LevelWord {
    const fn new() -> Self {
        Self(AtomicU32::new(0))
    }

    fn set(&self, level: Level) {
        let word = self.0.load(Ordering::Relaxed);
        let edges = (word & !LEVEL_MASK).wrapping_add(EDGE_STEP);
        self.0.store(edges | level_bit(level), Ordering::Release);
    }

    fn news(&self, seen: &mut u32) -> News {
        let now = self.0.load(Ordering::Acquire);
        let edges = (now >> 1).wrapping_sub(*seen >> 1) & EDGE_COUNT_MASK;
        *seen = now;
        let level = level_of(now);
        match edges {
            0 => News::Quiet,
            1 => News::Settled(level),
            _ => News::Crossed(level),
        }
    }
}

const fn level_bit(level: Level) -> u32 {
    match level {
        Level::Low => 0,
        Level::High => LEVEL_MASK,
    }
}

const fn level_of(word: u32) -> Level {
    if word & LEVEL_MASK == 0 {
        Level::Low
    } else {
        Level::High
    }
}

const fn opposite(level: Level) -> Level {
    match level {
        Level::Low => Level::High,
        Level::High => Level::Low,
    }
}

const fn rate_code(rate: SampleRate) -> u32 {
    match rate {
        SampleRate::Hz44100 => 0,
        SampleRate::Hz48000 => 1,
        SampleRate::Hz88200 => 2,
        SampleRate::Hz96000 => 3,
        SampleRate::Hz176400 => 4,
        SampleRate::Hz192000 => 5,
    }
}

const fn rate_of(word: u32) -> SampleRate {
    match word & RATE_CODE_MASK {
        0 => SampleRate::Hz44100,
        1 => SampleRate::Hz48000,
        2 => SampleRate::Hz88200,
        3 => SampleRate::Hz96000,
        4 => SampleRate::Hz176400,
        _ => SampleRate::Hz192000,
    }
}

const fn usb_code(usb: supervisor::Usb) -> u32 {
    match usb {
        supervisor::Usb::Silent => 0,
        supervisor::Usb::Serving => 1,
    }
}

const fn usb_of(word: u32) -> supervisor::Usb {
    if word == 0 {
        supervisor::Usb::Silent
    } else {
        supervisor::Usb::Serving
    }
}

#[derive(Debug)]
#[repr(C)]
pub struct UsbNotices {
    rate: AtomicU32,
    silence: LevelWord,
    phantom: LevelWord,
    terminal: LevelWord,
    bus: AtomicU32,
}

impl UsbNotices {
    pub const fn new() -> Self {
        Self {
            rate: AtomicU32::new(0),
            silence: LevelWord::new(),
            phantom: LevelWord::new(),
            terminal: LevelWord::new(),
            bus: AtomicU32::new(0),
        }
    }

    pub fn publish(&self, notice: Notice) {
        match notice {
            Notice::RateChange(event) => self.request(event),
            Notice::FadeOutputsToSilence => self.silence.set(Level::High),
            Notice::ReleaseOutputSilence => self.silence.set(Level::Low),
            Notice::SuspendPhantom => self.phantom.set(Level::High),
            Notice::ResumePhantom => self.phantom.set(Level::Low),
            Notice::DataTerminal(DataTerminal::Present) => self.terminal.set(Level::High),
            Notice::DataTerminal(DataTerminal::Absent) => self.terminal.set(Level::Low),
        }
    }

    pub fn bus(&self, usb: supervisor::Usb) {
        self.bus.store(usb_code(usb), Ordering::Release);
    }

    fn request(&self, event: rate_change::Event) {
        match event {
            rate_change::Event::SetCur(rate) => {
                let word = self.rate.load(Ordering::Relaxed);
                let generation = (word & !RATE_CODE_MASK).wrapping_add(GENERATION_STEP);
                self.rate
                    .store(generation | rate_code(rate), Ordering::Release);
            }
            rate_change::Event::Ticked
            | rate_change::Event::Elapsed(_)
            | rate_change::Event::I2cAcked
            | rate_change::Event::I2cNacked
            | rate_change::Event::PllLockTimedOut
            | rate_change::Event::StepDone => {}
        }
    }
}

pub const fn rate_request_word(generation: u32, rate: SampleRate) -> u32 {
    generation.wrapping_mul(GENERATION_STEP) | rate_code(rate)
}

impl Default for UsbNotices {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoticesSeen {
    rate: u32,
    silence: u32,
    phantom: u32,
    terminal: u32,
    bus: u32,
}

impl NoticesSeen {
    pub const START: Self = Self {
        rate: 0,
        silence: 0,
        phantom: 0,
        terminal: 0,
        bus: 0,
    };

    pub fn take(&mut self, from: &UsbNotices) -> PassNotices {
        self.drain(from, true)
    }

    pub fn drain(&mut self, from: &UsbNotices, running: bool) -> PassNotices {
        let mut notices = PassNotices::NONE;
        push_news(
            &mut notices,
            running,
            from.silence.news(&mut self.silence),
            [Notice::ReleaseOutputSilence, Notice::FadeOutputsToSilence],
        );
        push_news(
            &mut notices,
            running,
            from.phantom.news(&mut self.phantom),
            [Notice::ResumePhantom, Notice::SuspendPhantom],
        );
        push_news(
            &mut notices,
            running,
            from.terminal.news(&mut self.terminal),
            [
                Notice::DataTerminal(DataTerminal::Absent),
                Notice::DataTerminal(DataTerminal::Present),
            ],
        );
        let rate = from.rate.load(Ordering::Acquire);
        if (rate ^ self.rate) & !RATE_CODE_MASK != 0 {
            pass(
                &mut notices,
                running,
                Notice::RateChange(rate_change::Event::SetCur(rate_of(rate))),
            );
        }
        self.rate = rate;
        notices
    }

    pub fn bus_changed(&mut self, from: &UsbNotices) -> Option<supervisor::Usb> {
        let bus = from.bus.load(Ordering::Acquire);
        let changed = (bus != self.bus).then_some(usb_of(bus));
        self.bus = bus;
        changed
    }
}

const fn serves_the_console(notice: Notice) -> bool {
    match notice {
        Notice::DataTerminal(_) => true,
        Notice::RateChange(_)
        | Notice::FadeOutputsToSilence
        | Notice::ReleaseOutputSilence
        | Notice::SuspendPhantom
        | Notice::ResumePhantom => false,
    }
}

fn pass(notices: &mut PassNotices, running: bool, notice: Notice) {
    if running || serves_the_console(notice) {
        notices.push(notice);
    }
}

fn push_news(notices: &mut PassNotices, running: bool, news: News, [low, high]: [Notice; 2]) {
    let notice_at = |level| match level {
        Level::Low => low,
        Level::High => high,
    };
    match news {
        News::Quiet => {}
        News::Settled(level) => pass(notices, running, notice_at(level)),
        News::Crossed(level) => {
            pass(notices, running, notice_at(opposite(level)));
            pass(notices, running, notice_at(level));
        }
    }
}

#[derive(Debug)]
pub struct ClockReports(AtomicU32);

impl ClockReports {
    pub const fn at(rate: SampleRate) -> Self {
        Self(AtomicU32::new(rate_code(rate)))
    }

    pub fn publish(&self, report: ClockReport) {
        let validity = match report.validity {
            ClockValidity::Valid => 0,
            ClockValidity::Invalid => INVALID_BIT,
        };
        self.0
            .store(rate_code(report.rate) | validity, Ordering::Release);
    }

    pub fn latest(&self) -> ClockReport {
        let word = self.0.load(Ordering::Acquire);
        ClockReport {
            rate: rate_of(word),
            validity: if word & INVALID_BIT == 0 {
                ClockValidity::Valid
            } else {
                ClockValidity::Invalid
            },
        }
    }
}
