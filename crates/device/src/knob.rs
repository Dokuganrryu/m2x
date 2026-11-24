use crate::input::Input;

const KNOB_COUNT: usize = 5;
const CACHE_SHIFT: u32 = 8;
const LOW_BYTE_ABOVE_WHICH_ONE_BELOW_IS_HELD: u16 = 0x80;
const LOW_BYTE_MASK: u16 = 0xFF;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Knob {
    PreampTwo,
    Main,
    MonitorMix,
    PreampOne,
    Headphone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KnobChannel {
    Channel0A,
    Channel1A,
    Channel2A,
    Channel3A,
    Channel0B,
}

impl Knob {
    pub const ALL: [Knob; KNOB_COUNT] = [
        Knob::PreampTwo,
        Knob::Main,
        Knob::MonitorMix,
        Knob::PreampOne,
        Knob::Headphone,
    ];

    pub const fn channel(self) -> KnobChannel {
        match self {
            Knob::PreampTwo => KnobChannel::Channel0A,
            Knob::Main => KnobChannel::Channel1A,
            Knob::MonitorMix => KnobChannel::Channel2A,
            Knob::PreampOne => KnobChannel::Channel3A,
            Knob::Headphone => KnobChannel::Channel0B,
        }
    }

    pub const fn from_result_tag(tag: u8) -> Option<Knob> {
        match tag {
            1 => Some(Knob::PreampTwo),
            2 => Some(Knob::Main),
            3 => Some(Knob::MonitorMix),
            4 => Some(Knob::PreampOne),
            5 => Some(Knob::Headphone),
            0 | 6..=u8::MAX => None,
        }
    }

    pub const fn preamp(input: Input) -> Knob {
        match input {
            Input::One => Knob::PreampOne,
            Input::Two => Knob::PreampTwo,
        }
    }

    const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KnobReading(pub u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KnobReadings(pub [KnobReading; KNOB_COUNT]);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct KnobPosition(u8);

impl KnobPosition {
    pub const LOWEST: Self = Self(0);
    pub const HIGHEST: Self = Self(240);

    pub const fn new(value: u8) -> Option<Self> {
        if value <= Self::HIGHEST.0 {
            Some(Self(value))
        } else {
            None
        }
    }

    pub const fn value(self) -> u8 {
        self.0
    }

    pub(crate) const fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) const fn mirrored(self) -> Self {
        Self(Self::HIGHEST.0 - self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct KnobCache(u8);

impl KnobCache {
    const fn of(reading: KnobReading) -> Self {
        Self((reading.0 >> CACHE_SHIFT) as u8)
    }

    const fn after(self, reading: KnobReading) -> Self {
        let fresh = Self::of(reading);
        let one_below = fresh.0.checked_add(1);
        let held = matches!(one_below, Some(above) if above == self.0)
            && reading.0 & LOW_BYTE_MASK > LOW_BYTE_ABOVE_WHICH_ONE_BELOW_IS_HELD;
        if held { self } else { fresh }
    }

    const fn position(self) -> KnobPosition {
        if self.0 > KnobPosition::HIGHEST.0 {
            KnobPosition::HIGHEST
        } else {
            KnobPosition(self.0)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Knobs([KnobCache; KNOB_COUNT]);

impl Knobs {
    pub fn first_scan(readings: KnobReadings) -> Self {
        Self(readings.0.map(KnobCache::of))
    }

    pub fn after_scan(self, readings: KnobReadings) -> Self {
        let mut caches = self.0;
        for (cache, reading) in caches.iter_mut().zip(readings.0) {
            *cache = cache.after(reading);
        }
        Self(caches)
    }

    pub const fn position(self, knob: Knob) -> KnobPosition {
        self.0[knob.index()].position()
    }
}
