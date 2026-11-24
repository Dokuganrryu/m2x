use core::sync::atomic::{AtomicU32, Ordering};

use dsp::{DspMode, RateFamily};

const FORTY_FOUR_POINT_ONE: u32 = 0;
const FORTY_EIGHT: u32 = 1;
const BYPASS: u32 = 0;
const ON: u32 = 2;

#[repr(C)]
struct Span {
    last: AtomicU32,
    most: AtomicU32,
}

// WHY: Fixed field order, so the emulator reads the spans out of RAM by symbol: four (last, most) pairs indexed family | mode, then family and mode
#[repr(C)]
pub struct CycleSpans {
    spans: [Span; 4],
    family: AtomicU32,
    mode: AtomicU32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CycleSpan {
    pub last: u32,
    pub most: u32,
}

const fn code(family: RateFamily) -> u32 {
    match family {
        RateFamily::FortyFourPointOneKilohertz => FORTY_FOUR_POINT_ONE,
        RateFamily::FortyEightKilohertz => FORTY_EIGHT,
    }
}

const fn mode_code(mode: DspMode) -> u32 {
    match mode {
        DspMode::Bypass => BYPASS,
        DspMode::On => ON,
    }
}

// WHY: CMSIS SysTick, CVR counts down from RVR to 0 and reloads, one period every RVR + 1 cycles; an interrupt span is far shorter than a period, so one reload between entry and exit is the most that can happen
pub const fn down_count_span(entry: u32, exit: u32, period: u32) -> u32 {
    if entry >= exit {
        entry - exit
    } else {
        entry + period - exit
    }
}

// WHY: The cycle clock once subtracted a reading near 149,999 from a fixed 100,001 period and wrapped to billions of cycles at its first SysTick wrap after the 150 MHz reload; counting the wrap from the live reload keeps one sample within one period, so a bounded wait lasts its wall time at any core clock
pub const fn down_count_fall(last: u32, current: u32, period: u32) -> u32 {
    let fell = if current <= last {
        last - current
    } else {
        last.saturating_add(period.saturating_sub(current))
    };
    if fell > period { period } else { fell }
}

impl Span {
    const fn new() -> Self {
        Self {
            last: AtomicU32::new(0),
            most: AtomicU32::new(0),
        }
    }
}

impl CycleSpans {
    pub const fn new(family: RateFamily) -> Self {
        Self {
            spans: [Span::new(), Span::new(), Span::new(), Span::new()],
            family: AtomicU32::new(code(family)),
            mode: AtomicU32::new(BYPASS),
        }
    }

    #[inline(always)]
    fn span(&self, index: u32) -> &Span {
        &self.spans[(index & 3) as usize]
    }

    #[inline(always)]
    pub fn record(&self, entry: u32, exit: u32) {
        self.record_span(exit.wrapping_sub(entry));
    }

    #[inline(always)]
    pub fn record_span(&self, cycles: u32) {
        let index = self.family.load(Ordering::Relaxed) | self.mode.load(Ordering::Relaxed);
        let span = self.span(index);
        span.last.store(cycles, Ordering::Relaxed);
        if cycles > span.most.load(Ordering::Relaxed) {
            span.most.store(cycles, Ordering::Relaxed);
        }
    }

    pub fn follow(&self, family: RateFamily) {
        self.family.store(code(family), Ordering::Relaxed);
    }

    pub fn follow_mode(&self, mode: DspMode) {
        self.mode.store(mode_code(mode), Ordering::Relaxed);
    }

    pub fn read(&self, family: RateFamily) -> CycleSpan {
        self.read_in(family, DspMode::Bypass)
    }

    pub fn read_in(&self, family: RateFamily, mode: DspMode) -> CycleSpan {
        let span = self.span(code(family) | mode_code(mode));
        CycleSpan {
            last: span.last.load(Ordering::Relaxed),
            most: span.most.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_rate_family_keeps_its_own_last_and_largest_span() {
        let spans = CycleSpans::new(RateFamily::FortyEightKilohertz);
        spans.record(100, 1_100);
        spans.record(5_000, 5_400);
        spans.follow(RateFamily::FortyFourPointOneKilohertz);
        spans.record(0, 7);
        assert_eq!(
            spans.read(RateFamily::FortyEightKilohertz),
            CycleSpan {
                last: 400,
                most: 1_000
            }
        );
        assert_eq!(
            spans.read(RateFamily::FortyFourPointOneKilohertz),
            CycleSpan { last: 7, most: 7 }
        );
    }

    #[test]
    fn each_dsp_mode_keeps_its_own_span_within_a_family() {
        let spans = CycleSpans::new(RateFamily::FortyEightKilohertz);
        spans.record_span(2_000);
        spans.follow_mode(DspMode::On);
        spans.record_span(5_000);
        spans.record_span(4_000);
        assert_eq!(
            spans.read_in(RateFamily::FortyEightKilohertz, DspMode::Bypass),
            CycleSpan {
                last: 2_000,
                most: 2_000
            }
        );
        assert_eq!(
            spans.read_in(RateFamily::FortyEightKilohertz, DspMode::On),
            CycleSpan {
                last: 4_000,
                most: 5_000
            }
        );
        assert_eq!(
            spans.read_in(RateFamily::FortyFourPointOneKilohertz, DspMode::On),
            CycleSpan { last: 0, most: 0 }
        );
    }

    #[test]
    fn a_systick_span_is_the_fall_of_the_down_counter_even_across_a_reload() {
        const PERIOD: u32 = 100_001;
        assert_eq!(down_count_span(90_000, 87_500, PERIOD), 2_500);
        assert_eq!(down_count_span(1_000, 98_501, PERIOD), 2_500);
        assert_eq!(down_count_span(500, 500, PERIOD), 0);
    }

    #[test]
    fn a_span_across_the_counter_wrap_is_still_its_true_length() {
        let spans = CycleSpans::new(RateFamily::FortyEightKilohertz);
        spans.record(u32::MAX - 9, 20);
        assert_eq!(spans.read(RateFamily::FortyEightKilohertz).last, 30);
    }
}
