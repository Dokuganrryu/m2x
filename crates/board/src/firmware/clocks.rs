use device::boot::ClockInput;
use device::console::{ClockFacts, Lock, MainClock};
use device::pll::{self, CLOCK_READY_WAIT, Observation, Readiness, Sample, WATCHDOG_FEED_WAIT};
use dsp::RateFamily;

use super::clock_gates::{self, BusClock};
use super::dwt::CycleClock;
use super::map::{
    ANACTRL_FRO192M_CTRL, ANACTRL_FRO192M_CTRL_ENA_96MHZCLK, ANACTRL_XO32M_CTRL,
    ANACTRL_XO32M_CTRL_ENABLE_PLL_USB_OUT, ANACTRL_XO32M_CTRL_ENABLE_SYSTEM_CLK_OUT,
    ANACTRL_XO32M_STATUS, ANACTRL_XO32M_STATUS_XO_READY, AUDIO_CLOCK_DIV_44_1_KHZ_FAMILY,
    AUDIO_CLOCK_DIV_48_KHZ_FAMILY, IOCON_PIO1_31, IOCON_PIO1_31_FACTORY_MCLK,
    PMC_PDRUNCFG0_PDEN_FRO32K, PMC_PDRUNCFG0_PDEN_LDOUSBHS, PMC_PDRUNCFG0_PDEN_LDOXO32M,
    PMC_PDRUNCFG0_PDEN_PLL0, PMC_PDRUNCFG0_PDEN_PLL0_SSCG, PMC_PDRUNCFG0_PDEN_PLL1,
    PMC_PDRUNCFG0_PDEN_USBHSPHY, PMC_PDRUNCFG0_PDEN_XTAL32K, PMC_PDRUNCFG0_PDEN_XTAL32M,
    PMC_PDRUNCFGCLR0, PMC_RTCOSC32K, PMC_RTCOSC32K_FACTORY_WORD, SYSCON_AHBCLKDIV,
    SYSCON_AHBCLKDIV_DIVIDE_BY_ONE, SYSCON_CLOCK_CTRL, SYSCON_CLOCK_CTRL_CLKIN_ENA,
    SYSCON_CLOCK_CTRL_XO_CAL_CLK_ENA, SYSCON_FMCCR, SYSCON_FMCCR_FLASHTIM,
    SYSCON_FMCCR_FLASHTIM_UP_TO_100_MHZ, SYSCON_FROHFDIV, SYSCON_FROHFDIV_DIV,
    SYSCON_FROHFDIV_FACTORY_DIVIDE_BY_FOUR, SYSCON_MAINCLKSELA, SYSCON_MAINCLKSELA_SEL_CLKIN,
    SYSCON_MAINCLKSELA_SEL_FRO_12MHZ, SYSCON_MAINCLKSELA_SEL_FRO_96MHZ, SYSCON_MAINCLKSELB,
    SYSCON_MAINCLKSELB_SEL_MAIN_A, SYSCON_MAINCLKSELB_SEL_PLL1, SYSCON_MCLKCLKSEL,
    SYSCON_MCLKCLKSEL_SEL_PLL0, SYSCON_MCLKDIV, SYSCON_MCLKDIV_HALT, SYSCON_MCLKIO,
    SYSCON_MCLKIO_MCLKIO, SYSCON_PLL0CLKDIV, SYSCON_PLL0CLKDIV_HALT, SYSCON_PLL0CLKSEL,
    SYSCON_PLL0CLKSEL_SEL_CLKIN, SYSCON_PLL0CTRL, SYSCON_PLL0CTRL_FACTORY_WORD_FROM_RESET,
    SYSCON_PLL0NDEC, SYSCON_PLL0NDEC_FACTORY_N, SYSCON_PLL0PDEC, SYSCON_PLL0PDEC_FACTORY_P,
    SYSCON_PLL0SSCG1, SYSCON_PLL0SSCG1_44_1_KHZ_FAMILY_M_2352,
    SYSCON_PLL0SSCG1_48_KHZ_FAMILY_M_2048, SYSCON_PLL0STAT, SYSCON_PLL0STAT_LOCK,
    SYSCON_PLL1CLKSEL, SYSCON_PLL1CLKSEL_SEL_CLKIN, SYSCON_PLL1CTRL,
    SYSCON_PLL1CTRL_FACTORY_WORD_FROM_RESET, SYSCON_PLL1MDEC, SYSCON_PLL1MDEC_FACTORY_M,
    SYSCON_PLL1NDEC, SYSCON_PLL1NDEC_FACTORY_N, SYSCON_PLL1PDEC, SYSCON_PLL1PDEC_FACTORY_P,
    SYSCON_PLL1STAT, SYSCON_PLL1STAT_LOCK, USBPHY_PLL_SIC, USBPHY_PLL_SIC_PLL_LOCK,
};
use super::mmio::Register;
use super::watchdog::Watchdog;

const CRYSTAL_AND_USB_PHY_POWER: u32 = PMC_PDRUNCFG0_PDEN_XTAL32M
    | PMC_PDRUNCFG0_PDEN_LDOXO32M
    | PMC_PDRUNCFG0_PDEN_FRO32K
    | PMC_PDRUNCFG0_PDEN_XTAL32K
    | PMC_PDRUNCFG0_PDEN_USBHSPHY
    | PMC_PDRUNCFG0_PDEN_LDOUSBHS;
const _: () = assert!(CRYSTAL_AND_USB_PHY_POWER == 0x0014_11C0);

struct Pll0Family {
    multiplier: u32,
    audio_clock_div: u32,
}

const fn pll0_family(family: RateFamily) -> Pll0Family {
    match family {
        RateFamily::FortyEightKilohertz => Pll0Family {
            multiplier: SYSCON_PLL0SSCG1_48_KHZ_FAMILY_M_2048,
            audio_clock_div: AUDIO_CLOCK_DIV_48_KHZ_FAMILY,
        },
        RateFamily::FortyFourPointOneKilohertz => Pll0Family {
            multiplier: SYSCON_PLL0SSCG1_44_1_KHZ_FAMILY_M_2352,
            audio_clock_div: AUDIO_CLOCK_DIV_44_1_KHZ_FAMILY,
        },
    }
}

fn halt_then_divide(divider: Register, halt: u32, div: u32) {
    divider.set(halt);
    divider.write(div);
}

fn observe(status: Register, ready: u32, clock: &mut CycleClock) -> Observation {
    let sample = if status.read() & ready == 0 {
        Sample::NotReady
    } else {
        Sample::Ready
    };
    Observation {
        sample,
        at: clock.now(),
    }
}

fn wait_until_ready(watchdog: &mut Watchdog, status: Register, ready: u32) -> Readiness {
    let mut clock = CycleClock::start();
    let mut fed_at = clock.now();
    pll::await_readiness(
        CLOCK_READY_WAIT,
        || {
            let observation = observe(status, ready, &mut clock);
            if WATCHDOG_FEED_WAIT.elapsed_by(fed_at, observation.at) {
                watchdog.feed();
                fed_at = observation.at;
            }
            observation
        },
        || {},
    )
}

fn require_lock(watchdog: &mut Watchdog, status: Register, lock: u32) -> Readiness {
    wait_until_ready(watchdog, status, lock)
}

pub fn start_clock_input() {
    clock_gates::enable(&[BusClock::AnalogControl]);
    #[cfg(feature = "milestone2")]
    keep_the_core_fast_enough_for_usb();
    PMC_RTCOSC32K.write(PMC_RTCOSC32K_FACTORY_WORD);
    PMC_PDRUNCFGCLR0.write(CRYSTAL_AND_USB_PHY_POWER);
    ANACTRL_XO32M_CTRL
        .set(ANACTRL_XO32M_CTRL_ENABLE_SYSTEM_CLK_OUT | ANACTRL_XO32M_CTRL_ENABLE_PLL_USB_OUT);
    SYSCON_CLOCK_CTRL.set(SYSCON_CLOCK_CTRL_CLKIN_ENA | SYSCON_CLOCK_CTRL_XO_CAL_CLK_ENA);
}

// WHY: UM11126 rev 1.8 sections on the main clock muxes: they are glitch-free but the clock switched to must already run, so the 96 MHz output is enabled first; the default core voltage covers 100 MHz and below, and the flash access time is set for the new clock before the clock rises
#[cfg(feature = "milestone2")]
fn keep_the_core_fast_enough_for_usb() {
    use device::pll::{CoreClockForUsb, MainClockSelectors, RaiseTo96Mhz, core_clock_for_usb};
    let handed_over = MainClockSelectors {
        a: SYSCON_MAINCLKSELA.read(),
        b: SYSCON_MAINCLKSELB.read(),
    };
    let CoreClockForUsb::RaiseTo96Mhz(raise) =
        core_clock_for_usb(handed_over, SYSCON_AHBCLKDIV.read())
    else {
        return;
    };
    set_the_flash_access_time_for_100_mhz();
    match raise {
        RaiseTo96Mhz::SwitchToFro96 => {
            ANACTRL_FRO192M_CTRL.set(ANACTRL_FRO192M_CTRL_ENA_96MHZCLK);
            SYSCON_MAINCLKSELA.write(SYSCON_MAINCLKSELA_SEL_FRO_96MHZ);
        }
        RaiseTo96Mhz::DivideFro96ByOne => SYSCON_AHBCLKDIV.write(SYSCON_AHBCLKDIV_DIVIDE_BY_ONE),
    }
}

// WHY: UM11126 rev 1.8 Table 113, the flash access time is set for the new clock before the clock rises; FLASHTIM 4, five clocks, is good to 55 MHz only
fn set_the_flash_access_time_for_100_mhz() {
    SYSCON_FMCCR
        .write(SYSCON_FMCCR.read() & !SYSCON_FMCCR_FLASHTIM | SYSCON_FMCCR_FLASHTIM_UP_TO_100_MHZ);
}

pub fn clock_input(watchdog: &mut Watchdog) -> ClockInput {
    match wait_until_ready(
        watchdog,
        ANACTRL_XO32M_STATUS,
        ANACTRL_XO32M_STATUS_XO_READY,
    ) {
        Readiness::Reached => ClockInput::Ready,
        Readiness::TimedOut => ClockInput::NotReady,
    }
}

pub fn usb_phy_pll_lock(watchdog: &mut Watchdog) -> Readiness {
    require_lock(watchdog, USBPHY_PLL_SIC, USBPHY_PLL_SIC_PLL_LOCK)
}

pub fn start_serial_bus_clock_source() {
    clock_gates::enable(&[BusClock::AnalogControl]);
    ANACTRL_FRO192M_CTRL.set(ANACTRL_FRO192M_CTRL_ENA_96MHZCLK);
    SYSCON_FROHFDIV.write(SYSCON_FROHFDIV_FACTORY_DIVIDE_BY_FOUR);
}

pub fn start_core_pll() {
    PMC_PDRUNCFGCLR0.write(PMC_PDRUNCFG0_PDEN_PLL1);
    SYSCON_PLL1CLKSEL.write(SYSCON_PLL1CLKSEL_SEL_CLKIN);
    SYSCON_PLL1CTRL.write(SYSCON_PLL1CTRL_FACTORY_WORD_FROM_RESET);
    SYSCON_PLL1MDEC.write(SYSCON_PLL1MDEC_FACTORY_M);
    SYSCON_PLL1NDEC.write(SYSCON_PLL1NDEC_FACTORY_N);
    SYSCON_PLL1PDEC.write(SYSCON_PLL1PDEC_FACTORY_P);
}

pub fn switch_pll0_family(family: RateFamily, watchdog: &mut Watchdog) -> Readiness {
    let settings = pll0_family(family);
    PMC_PDRUNCFGCLR0.write(PMC_PDRUNCFG0_PDEN_PLL0 | PMC_PDRUNCFG0_PDEN_PLL0_SSCG);
    SYSCON_PLL0CLKSEL.write(SYSCON_PLL0CLKSEL_SEL_CLKIN);
    SYSCON_PLL0CTRL.write(SYSCON_PLL0CTRL_FACTORY_WORD_FROM_RESET);
    SYSCON_PLL0NDEC.write(SYSCON_PLL0NDEC_FACTORY_N);
    SYSCON_PLL0PDEC.write(SYSCON_PLL0PDEC_FACTORY_P);
    SYSCON_PLL0SSCG1.write(settings.multiplier);
    halt_then_divide(
        SYSCON_MCLKDIV,
        SYSCON_MCLKDIV_HALT,
        settings.audio_clock_div,
    );
    halt_then_divide(
        SYSCON_PLL0CLKDIV,
        SYSCON_PLL0CLKDIV_HALT,
        settings.audio_clock_div,
    );
    let readiness = require_lock(watchdog, SYSCON_PLL0STAT, SYSCON_PLL0STAT_LOCK);
    if readiness == Readiness::TimedOut {
        SYSCON_MCLKDIV.write(SYSCON_MCLKDIV_HALT);
    }
    readiness
}

#[cfg(not(feature = "sound3"))]
pub fn select_core_pll_as_main_clock(watchdog: &mut Watchdog) -> Readiness {
    select_pll1(watchdog)
}

// WHY: NXP's HIGH voltage row goes in before the clock rises, then PLL1 is relocked with N 2 while it is not the main clock; a failed raise keeps the 100 MHz clock and reports CoreClockHeld instead of stopping the boot
#[cfg(feature = "sound3")]
pub fn select_core_pll_as_main_clock(watchdog: &mut Watchdog) -> Readiness {
    use device::breadcrumb::{At, Weight};
    use device::core_clock::{CoreClock, HeldAt100, PLL1_NDEC_FOR_150_MHZ};
    use device::fault::FaultCode;
    // WHY: From milestone2 on the image raises no MPU fence (boot.rs QUIESCE_OF_THE_BUILD); that holds because main copies the factory page and the NMPA words before the clock stage and nothing reads flash afterwards
    let held = match super::power::raise_voltage_for_150_mhz(super::factory::nmpa()) {
        None => HeldAt100::VoltageNotConfirmed,
        Some(_) => {
            relock_pll1(PLL1_NDEC_FOR_150_MHZ);
            if select_pll1(watchdog) == Readiness::Reached {
                speed::running_at(CoreClock::Pll150);
                return Readiness::Reached;
            }
            relock_pll1(SYSCON_PLL1NDEC_FACTORY_N);
            HeldAt100::Pll150Unlocked
        }
    };
    let readiness = select_pll1(watchdog);
    if readiness == Readiness::Reached {
        speed::running_at(CoreClock::Pll100(held));
        super::fault::record(FaultCode::CoreClockHeld, At::Here, Weight::Noted);
    }
    readiness
}

// WHY: PLL1's dividers are written while it is powered down, so the relock powers it down, writes N and powers it up, then holds 1 ms (above the 0.53 ms errata settle at a 12 MHz reference, with margin after the voltage change) before LOCK is read
#[cfg(feature = "sound3")]
fn relock_pll1(divider: u32) {
    use super::map::PMC_PDRUNCFGSET0;
    PMC_PDRUNCFGSET0.write(PMC_PDRUNCFG0_PDEN_PLL1);
    SYSCON_PLL1NDEC.write(divider);
    PMC_PDRUNCFGCLR0.write(PMC_PDRUNCFG0_PDEN_PLL1);
    let mut clock = CycleClock::start();
    let started = clock.now();
    while !SETTLE.elapsed_by(started, clock.now()) {}
}

#[cfg(feature = "sound3")]
const SETTLE: device::pll::CycleBudget =
    device::pll::CycleBudget(pll::TIMEOUT_BUDGET_CLOCK.0 / 1_000);

#[cfg(feature = "sound3")]
pub mod speed {
    use core::sync::atomic::{AtomicU8, Ordering};

    use device::core_clock::{CoreClock, HeldAt100, SYSTICK_RELOAD_AT_150_MHZ};

    use super::super::map::SYST_RVR;

    static RUNNING: AtomicU8 = AtomicU8::new(0);

    pub fn running_at(clock: CoreClock) {
        if clock == CoreClock::Pll150 {
            SYST_RVR.write(SYSTICK_RELOAD_AT_150_MHZ);
            dsp::lift_bands_at_twelve_frames();
        }
        RUNNING.store(
            match clock {
                CoreClock::Pll150 => 1,
                CoreClock::Pll100(HeldAt100::NotYetRaised) => 2,
                CoreClock::Pll100(HeldAt100::VoltageNotConfirmed) => 3,
                CoreClock::Pll100(HeldAt100::Pll150Unlocked) => 4,
                CoreClock::Fro96 => 5,
            },
            Ordering::Relaxed,
        );
    }

    pub fn clock() -> CoreClock {
        match RUNNING.load(Ordering::Relaxed) {
            1 => CoreClock::Pll150,
            3 => CoreClock::Pll100(HeldAt100::VoltageNotConfirmed),
            4 => CoreClock::Pll100(HeldAt100::Pll150Unlocked),
            5 => CoreClock::Fro96,
            _ => CoreClock::Pll100(HeldAt100::NotYetRaised),
        }
    }
}

fn select_pll1(watchdog: &mut Watchdog) -> Readiness {
    if require_lock(watchdog, SYSCON_PLL1STAT, SYSCON_PLL1STAT_LOCK) == Readiness::TimedOut {
        return Readiness::TimedOut;
    }
    set_the_flash_access_time_for_100_mhz();
    SYSCON_AHBCLKDIV.write(SYSCON_AHBCLKDIV_DIVIDE_BY_ONE);
    SYSCON_MAINCLKSELB.write(SYSCON_MAINCLKSELB_SEL_PLL1);
    SYSCON_MAINCLKSELA.write(SYSCON_MAINCLKSELA_SEL_CLKIN);
    // WHY: FC4 runs the screen from FRO_HF_DIV (FCCLKSEL4 3), and start_serial_bus_clock_source set the divide before the screen started; rewriting the divider under a live transfer gains nothing, so it is written only when it differs
    if SYSCON_FROHFDIV.read() & SYSCON_FROHFDIV_DIV != SYSCON_FROHFDIV_FACTORY_DIVIDE_BY_FOUR {
        SYSCON_FROHFDIV.write(SYSCON_FROHFDIV_FACTORY_DIVIDE_BY_FOUR);
    }
    Readiness::Reached
}

pub fn facts() -> ClockFacts {
    let locked = |status: Register, lock: u32| {
        if status.read() & lock == 0 {
            Lock::Unlocked
        } else {
            Lock::Locked
        }
    };
    ClockFacts {
        main: match (SYSCON_MAINCLKSELB.read(), SYSCON_MAINCLKSELA.read()) {
            (SYSCON_MAINCLKSELB_SEL_PLL1, _) => MainClock::Pll1,
            (SYSCON_MAINCLKSELB_SEL_MAIN_A, SYSCON_MAINCLKSELA_SEL_FRO_12MHZ) => MainClock::Fro12,
            (SYSCON_MAINCLKSELB_SEL_MAIN_A, SYSCON_MAINCLKSELA_SEL_FRO_96MHZ) => MainClock::Fro96,
            _ => MainClock::Other,
        },
        core_pll: locked(SYSCON_PLL1STAT, SYSCON_PLL1STAT_LOCK),
        audio_pll: locked(SYSCON_PLL0STAT, SYSCON_PLL0STAT_LOCK),
        #[cfg(feature = "sound3")]
        core: speed::clock(),
    }
}

pub fn enable_master_clock_output() {
    clock_gates::enable(&[BusClock::Iocon]);
    SYSCON_MCLKCLKSEL.write(SYSCON_MCLKCLKSEL_SEL_PLL0);
    SYSCON_MCLKIO.write(SYSCON_MCLKIO_MCLKIO);
    IOCON_PIO1_31.write(IOCON_PIO1_31_FACTORY_MCLK);
}
