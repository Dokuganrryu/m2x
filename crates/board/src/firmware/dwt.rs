use device::pll::CycleCount;

#[cfg(feature = "milestone4")]
use super::map::SYST_CVR;
#[cfg(all(feature = "milestone4", not(feature = "sound3")))]
use super::map::SYST_RVR_FACTORY_ONE_MILLISECOND;
#[cfg(not(feature = "milestone4"))]
use super::map::{
    DEMCR, DEMCR_TRCENA, DWT_CTRL, DWT_CTRL_CYCCNTENA, DWT_CTRL_NOCYCCNT, DWT_CYCCNT,
};

#[cfg(not(feature = "milestone4"))]
pub fn start_the_cycle_counter() {
    DEMCR.set(DEMCR_TRCENA);
    if DWT_CTRL.read() & DWT_CTRL_NOCYCCNT == 0 {
        DWT_CYCCNT.write(0);
        DWT_CTRL.set(DWT_CTRL_CYCCNTENA);
    }
}

#[cfg(not(feature = "milestone4"))]
pub fn ensure_the_cycle_counter_runs() {
    if DWT_CTRL.read() & DWT_CTRL_CYCCNTENA == 0 {
        start_the_cycle_counter();
    }
}

#[cfg(not(feature = "milestone4"))]
#[inline(always)]
pub fn cycles() -> u32 {
    DWT_CYCCNT.read()
}

#[cfg(not(feature = "milestone4"))]
pub struct CycleClock(());

#[cfg(not(feature = "milestone4"))]
impl CycleClock {
    pub fn start() -> Self {
        Self(())
    }

    #[inline(always)]
    pub fn now(&mut self) -> CycleCount {
        CycleCount(cycles())
    }
}

// WHY: Armv8-M, DWT_CYCCNT stops when Non-secure non-invasive debug is not allowed, and a frozen counter would end each wait after its 64 frozen samples, so waits are timed on the SysTick the executor starts
#[cfg(feature = "milestone4")]
pub fn start_the_cycle_counter() {}

#[cfg(feature = "milestone4")]
pub fn ensure_the_cycle_counter_runs() {}

// WHY: Milestone4 measures no interrupt spans, so the console's tick and usb lines read 0
#[cfg(all(feature = "milestone4", not(feature = "sound1")))]
#[inline(always)]
pub fn cycles() -> u32 {
    0
}

// WHY: The executor's SysTick runs at the core clock in Non-secure state; CMSIS, a CVR read leaves COUNTFLAG alone, so the executor's millisecond is untouched; the tick and USB interrupts are timed on it
#[cfg(feature = "sound1")]
#[inline(always)]
pub fn cycles() -> u32 {
    SYST_CVR.read()
}

#[cfg(all(feature = "sound1", not(feature = "sound3")))]
#[inline(always)]
pub fn span(entry: u32, exit: u32) -> u32 {
    device::cycles::down_count_span(entry, exit, SYSTICK_PERIOD)
}

// WHY: The 150 MHz build reloads SysTick with 149,999, so a span takes the period from the reload the counter really runs
#[cfg(feature = "sound3")]
#[inline(always)]
pub fn span(entry: u32, exit: u32) -> u32 {
    device::cycles::down_count_span(entry, exit, super::map::SYST_RVR.read() + 1)
}

#[cfg(not(feature = "sound1"))]
#[inline(always)]
pub fn span(entry: u32, exit: u32) -> u32 {
    exit.wrapping_sub(entry)
}

#[cfg(all(feature = "milestone4", not(feature = "sound3")))]
const SYSTICK_PERIOD: u32 = SYST_RVR_FACTORY_ONE_MILLISECOND + 1;

#[cfg(feature = "milestone4")]
pub struct CycleClock {
    last: u32,
    elapsed: u32,
}

// WHY: CMSIS SysTick, CVR counts core cycles down from RVR and a CVR read leaves COUNTFLAG to the executor's millisecond; a wait samples far more often than once a wrap, so summing the falls between samples counts every cycle
#[cfg(feature = "milestone4")]
impl CycleClock {
    pub fn start() -> Self {
        Self {
            last: SYST_CVR.read(),
            elapsed: 0,
        }
    }

    #[cfg(not(feature = "sound3"))]
    pub fn now(&mut self) -> CycleCount {
        let current = SYST_CVR.read();
        let fell = if current <= self.last {
            self.last - current
        } else {
            self.last + SYSTICK_PERIOD - current
        };
        self.elapsed = self.elapsed.wrapping_add(fell);
        self.last = current;
        CycleCount(self.elapsed)
    }

    // WHY: SysTick reloads 149,999 at 150 MHz, so a fixed 100,001 period turned the first wrap of the PLL0 lock wait into billions of cycles; the period is read from the live reload
    #[cfg(feature = "sound3")]
    pub fn now(&mut self) -> CycleCount {
        let current = SYST_CVR.read();
        let fell =
            device::cycles::down_count_fall(self.last, current, super::map::SYST_RVR.read() + 1);
        self.elapsed = self.elapsed.wrapping_add(fell);
        self.last = current;
        CycleCount(self.elapsed)
    }
}
