use core::sync::atomic::{AtomicBool, Ordering};

use device::phantom::Milliseconds;
use device::supervisor::WATCHDOG_TIMEOUT;

use super::fault_policy::{FAULT_POLICY, FaultPolicy};
use super::map::{
    SYSCON_AHBCLKCTRL0_WWDT, SYSCON_AHBCLKCTRLSET0, SYSCON_CLOCK_CTRL,
    SYSCON_CLOCK_CTRL_FRO1MHZ_CLK_ENA, SYSCON_WDTCLKDIV, SYSCON_WDTCLKDIV_DIVIDE_BY_ONE,
    SYSCON_WDTCLKDIV_RESET, WWDT_FEED, WWDT_FEED_FIRST, WWDT_FEED_SECOND, WWDT_MOD, WWDT_MOD_WDEN,
    WWDT_MOD_WDPROTECT, WWDT_MOD_WDRESET, WWDT_TC, WWDT_WARNINT, WWDT_WARNINT_NONE, WWDT_WINDOW,
    WWDT_WINDOW_NONE,
};
use super::mmio::with_interrupts_masked;

const WATCHDOG_COUNTS_PER_MILLISECOND: u32 = 1_000_000 / 4 / 1_000;
const SHORTEST_TIMEOUT_COUNT: u32 = 0xFF;
const LONGEST_TIMEOUT_COUNT: u32 = 0xFF_FFFF;

const fn timeout_count(timeout: Milliseconds) -> u32 {
    let count = timeout.0 * WATCHDOG_COUNTS_PER_MILLISECOND;
    assert!(count >= SHORTEST_TIMEOUT_COUNT && count <= LONGEST_TIMEOUT_COUNT);
    count
}

const WATCHDOG_TIMEOUT_COUNT: u32 = timeout_count(WATCHDOG_TIMEOUT);

static CLAIMED: AtomicBool = AtomicBool::new(false);

// WHY: On hardware, an image that never touched the WWDT stayed attached for over 94 s, so no watchdog runs at the hand-over, and the hand-over leaves the WWDT clock, MOD and FEED alone
const TOUCHES_THE_WWDT: bool = !cfg!(feature = "milestone4");

// WHY: UM11126 rev 1.8 section 30.6.1, under WDPROTECT a TC write or a feed before TV is below WARNINT and WINDOW is a feed error, so the bring-up policy leaves a locked watchdog as the bootloader set it
enum Inherited {
    Stopped,
    Running,
    Locked,
}

fn inherited() -> Inherited {
    let mode = WWDT_MOD.read();
    if mode & WWDT_MOD_WDEN == 0 {
        Inherited::Stopped
    } else if mode & WWDT_MOD_WDPROTECT == 0 {
        Inherited::Running
    } else {
        Inherited::Locked
    }
}

// WHY: UM11126 rev 1.8 section 30.6.6, a feed while TV is above WINDOW is a watchdog event, and section 30.6.2, a feed loads TC
fn open_an_inherited_window() {
    match inherited() {
        Inherited::Running => with_interrupts_masked(|| {
            WWDT_WINDOW.write(WWDT_WINDOW_NONE);
            WWDT_TC.write(LONGEST_TIMEOUT_COUNT);
            WWDT_FEED.write(WWDT_FEED_FIRST);
            WWDT_FEED.write(WWDT_FEED_SECOND);
        }),
        Inherited::Stopped => {}
        Inherited::Locked => {}
    }
}

fn feed() {
    if !TOUCHES_THE_WWDT {
        return;
    }
    match FAULT_POLICY {
        FaultPolicy::BringUp => match inherited() {
            Inherited::Stopped | Inherited::Running => feed_now(),
            Inherited::Locked => {}
        },
        FaultPolicy::Release => feed_now(),
    }
}

fn feed_now() {
    with_interrupts_masked(|| {
        WWDT_FEED.write(WWDT_FEED_FIRST);
        WWDT_FEED.write(WWDT_FEED_SECOND);
    });
}

pub fn feed_while_parked() {
    feed();
}

pub struct Watchdog(());

impl Watchdog {
    pub fn claim() -> Option<Self> {
        if CLAIMED.swap(true, Ordering::AcqRel) {
            return None;
        }
        if !TOUCHES_THE_WWDT {
            return Some(Self(()));
        }
        SYSCON_AHBCLKCTRLSET0.write(SYSCON_AHBCLKCTRL0_WWDT);
        match FAULT_POLICY {
            FaultPolicy::BringUp => open_an_inherited_window(),
            FaultPolicy::Release => {}
        }
        Some(Self(()))
    }

    pub fn feed(&mut self) {
        feed();
    }

    pub fn arm(&mut self) {
        if !TOUCHES_THE_WWDT {
            return;
        }
        SYSCON_CLOCK_CTRL.set(SYSCON_CLOCK_CTRL_FRO1MHZ_CLK_ENA);
        SYSCON_WDTCLKDIV.write(SYSCON_WDTCLKDIV_RESET);
        SYSCON_WDTCLKDIV.write(SYSCON_WDTCLKDIV_DIVIDE_BY_ONE);
        with_interrupts_masked(|| {
            WWDT_TC.write(WATCHDOG_TIMEOUT_COUNT);
            WWDT_MOD.write(WWDT_MOD_WDEN | WWDT_MOD_WDRESET);
            WWDT_WINDOW.write(WWDT_WINDOW_NONE);
            WWDT_WARNINT.write(WWDT_WARNINT_NONE);
            WWDT_FEED.write(WWDT_FEED_FIRST);
            WWDT_FEED.write(WWDT_FEED_SECOND);
        });
    }
}
