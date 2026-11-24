use dsp::RateFamily;

use crate::lcd::panel::{Hertz, Microseconds};
use crate::phantom::Milliseconds;
use crate::supervisor;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Predivider(pub u32);

pub const INFERRED_CLOCK_INPUT: Hertz = Hertz(24_000_000);
pub const CORE_PLL_PREDIVIDER: Predivider = Predivider(3);
pub const AUDIO_PLL_PREDIVIDER_FOR_48K_FAMILY: Predivider = Predivider(125);
pub const AUDIO_PLL_PREDIVIDER_FOR_44K1_FAMILY: Predivider = Predivider(125);

pub const LOWEST_LOCK_DETECTOR_REFERENCE: Hertz = Hertz(100_000);
pub const HIGHEST_LOCK_DETECTOR_REFERENCE: Hertz = Hertz(20_000_000);
pub const ERRATA_FIXED_WAIT: Microseconds = Microseconds(500);
pub const ERRATA_REFERENCE_PERIODS: u64 = 400;

const MICROSECONDS_PER_SECOND: u64 = 1_000_000;
const MICROSECONDS_PER_MILLISECOND: u32 = 1_000;

pub const CORE_PLL_STABILISATION: Milliseconds =
    stabilisation_wait(reference_clock(CORE_PLL_PREDIVIDER));
pub const AUDIO_PLL_STABILISATION_FOR_48K_FAMILY: Milliseconds =
    stabilisation_wait(reference_clock(AUDIO_PLL_PREDIVIDER_FOR_48K_FAMILY));
pub const AUDIO_PLL_STABILISATION_FOR_44K1_FAMILY: Milliseconds =
    stabilisation_wait(reference_clock(AUDIO_PLL_PREDIVIDER_FOR_44K1_FAMILY));

pub const fn audio_pll_predivider(family: RateFamily) -> Predivider {
    match family {
        RateFamily::FortyEightKilohertz => AUDIO_PLL_PREDIVIDER_FOR_48K_FAMILY,
        RateFamily::FortyFourPointOneKilohertz => AUDIO_PLL_PREDIVIDER_FOR_44K1_FAMILY,
    }
}

pub const fn audio_pll_stabilisation(family: RateFamily) -> Milliseconds {
    match family {
        RateFamily::FortyEightKilohertz => AUDIO_PLL_STABILISATION_FOR_48K_FAMILY,
        RateFamily::FortyFourPointOneKilohertz => AUDIO_PLL_STABILISATION_FOR_44K1_FAMILY,
    }
}

pub const fn reference_clock(predivider: Predivider) -> Hertz {
    Hertz(INFERRED_CLOCK_INPUT.0 / predivider.0)
}

pub const fn stabilisation_time(reference: Hertz) -> Microseconds {
    assert!(
        reference.0 >= LOWEST_LOCK_DETECTOR_REFERENCE.0
            && reference.0 < HIGHEST_LOCK_DETECTOR_REFERENCE.0
    );
    let reference_periods =
        (ERRATA_REFERENCE_PERIODS * MICROSECONDS_PER_SECOND).div_ceil(reference.0 as u64);
    Microseconds(ERRATA_FIXED_WAIT.0 + reference_periods as u32)
}

const fn stabilisation_wait(reference: Hertz) -> Milliseconds {
    Milliseconds(
        stabilisation_time(reference)
            .0
            .div_ceil(MICROSECONDS_PER_MILLISECOND),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CycleCount(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CycleBudget(pub u32);

impl CycleBudget {
    pub fn elapsed_by(self, started_at: CycleCount, now: CycleCount) -> bool {
        now.0.wrapping_sub(started_at.0) >= self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readiness {
    Reached,
    TimedOut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sample {
    Ready,
    NotReady,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Observation {
    pub sample: Sample,
    pub at: CycleCount,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wait {
    Starting(CycleBudget),
    Waiting {
        started_at: CycleCount,
        last_observed: CycleCount,
        budget: CycleBudget,
        frozen_samples: u32,
    },
    Settled(Readiness),
}

pub const FROZEN_SAMPLE_LIMIT: u32 = 64;

impl Wait {
    pub const fn starting(budget: CycleBudget) -> Self {
        Wait::Starting(budget)
    }
}

pub fn transition(wait: Wait, observation: Observation) -> (Wait, Option<Readiness>) {
    match (wait, observation.sample) {
        (Wait::Starting(_), Sample::Ready) => {
            (Wait::Settled(Readiness::Reached), Some(Readiness::Reached))
        }
        (Wait::Starting(budget), Sample::NotReady) => (
            Wait::Waiting {
                started_at: observation.at,
                last_observed: observation.at,
                budget,
                frozen_samples: 0,
            },
            None,
        ),
        (Wait::Waiting { .. }, Sample::Ready) => {
            (Wait::Settled(Readiness::Reached), Some(Readiness::Reached))
        }
        (
            Wait::Waiting {
                started_at,
                last_observed,
                budget,
                frozen_samples,
            },
            Sample::NotReady,
        ) => {
            let frozen_samples = if observation.at == last_observed {
                frozen_samples + 1
            } else {
                0
            };
            if budget.elapsed_by(started_at, observation.at)
                || frozen_samples >= FROZEN_SAMPLE_LIMIT
            {
                (
                    Wait::Settled(Readiness::TimedOut),
                    Some(Readiness::TimedOut),
                )
            } else {
                (
                    Wait::Waiting {
                        started_at,
                        last_observed: observation.at,
                        budget,
                        frozen_samples,
                    },
                    None,
                )
            }
        }
        (Wait::Settled(readiness), Sample::Ready | Sample::NotReady) => {
            (Wait::Settled(readiness), Some(readiness))
        }
    }
}

pub fn await_readiness(
    budget: CycleBudget,
    mut observe: impl FnMut() -> Observation,
    mut before_each_sample: impl FnMut(),
) -> Readiness {
    let mut wait = Wait::starting(budget);
    loop {
        before_each_sample();
        let (next, outcome) = transition(wait, observe());
        wait = next;
        if let Some(readiness) = outcome {
            return readiness;
        }
    }
}

pub const TIMEOUT_BUDGET_CLOCK: Hertz = Hertz(150_000_000);
pub const SLOWEST_CORE_CLOCK: Hertz = Hertz(12_000_000);
pub const SLOWEST_BOOTLOADER_CORE_CLOCK: Hertz = Hertz(1_000_000);

const fn wait_cycles(milliseconds: Milliseconds, clock: Hertz) -> CycleBudget {
    CycleBudget(
        (clock.0 as u64 * milliseconds.0 as u64 / MICROSECONDS_PER_MILLISECOND as u64) as u32,
    )
}

pub const CLOCK_READY_WAIT: CycleBudget = wait_cycles(Milliseconds(100), TIMEOUT_BUDGET_CLOCK);
pub const SHORTEST_WATCHDOG_TIMEOUT: Milliseconds =
    Milliseconds(supervisor::WATCHDOG_TIMEOUT.0 / 4);
pub const WATCHDOG_FEED_WAIT: CycleBudget = wait_cycles(
    Milliseconds(SHORTEST_WATCHDOG_TIMEOUT.0 / 2),
    SLOWEST_BOOTLOADER_CORE_CLOCK,
);

#[cfg(feature = "milestone2")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MainClockSelectors {
    pub a: u32,
    pub b: u32,
}

#[cfg(feature = "milestone2")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RaiseTo96Mhz {
    SwitchToFro96,
    DivideFro96ByOne,
}

#[cfg(feature = "milestone2")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoreClockForUsb {
    RaiseTo96Mhz(RaiseTo96Mhz),
    KeepAsHandedOver,
}

#[cfg(feature = "milestone2")]
const SELECTOR_FIELD: u32 = 0b111;
#[cfg(feature = "milestone2")]
const MAIN_CLOCK_A: u32 = 0;
#[cfg(feature = "milestone2")]
const FRO_12_MHZ: u32 = 0;
#[cfg(feature = "milestone2")]
const FRO_96_MHZ: u32 = 3;
#[cfg(feature = "milestone2")]
const AHB_DIVIDER_FIELD: u32 = 0xFF;
#[cfg(feature = "milestone2")]
const FRO_96_MHZ_CLOCK: Hertz = Hertz(96_000_000);
#[cfg(feature = "milestone2")]
const USB1_LOWEST_CORE_CLOCK: Hertz = Hertz(60_000_000);

// WHY: UM11126 rev 1.8 section 44.3, the USB1 device wants a CPU clock of 60 MHz or more; Tables 72 and 73, SEL is bits 2:0, 0 in both is the 12 MHz FRO, the reset value, and A 3 is the 96 MHz FRO; Table 102, the core runs at the main clock over AHBCLKDIV bits 7:0 plus one
#[cfg(feature = "milestone2")]
pub const fn core_clock_for_usb(
    handed_over: MainClockSelectors,
    ahb_clock_div: u32,
) -> CoreClockForUsb {
    match (
        handed_over.b & SELECTOR_FIELD,
        handed_over.a & SELECTOR_FIELD,
    ) {
        (MAIN_CLOCK_A, FRO_12_MHZ) => CoreClockForUsb::RaiseTo96Mhz(RaiseTo96Mhz::SwitchToFro96),
        (MAIN_CLOCK_A, FRO_96_MHZ)
            if FRO_96_MHZ_CLOCK.0 / ((ahb_clock_div & AHB_DIVIDER_FIELD) + 1)
                < USB1_LOWEST_CORE_CLOCK.0 =>
        {
            CoreClockForUsb::RaiseTo96Mhz(RaiseTo96Mhz::DivideFro96ByOne)
        }
        _ => CoreClockForUsb::KeepAsHandedOver,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeStatus {
        ready_at_sample: Option<u32>,
        samples: u32,
    }

    impl FakeStatus {
        const fn new(ready_at_sample: Option<u32>) -> Self {
            Self {
                ready_at_sample,
                samples: 0,
            }
        }

        fn observe(&mut self, clock: &mut FakeClock) -> Observation {
            self.samples += 1;
            let sample = match self.ready_at_sample {
                Some(at) if self.samples >= at => Sample::Ready,
                Some(_) | None => Sample::NotReady,
            };
            Observation {
                sample,
                at: clock.advance(),
            }
        }
    }

    struct FakeClock {
        cycles: u32,
        step: u32,
    }

    impl FakeClock {
        const fn stepping_by(step: u32) -> Self {
            Self { cycles: 0, step }
        }

        const fn starting_at(cycles: u32, step: u32) -> Self {
            Self { cycles, step }
        }

        fn advance(&mut self) -> CycleCount {
            self.cycles = self.cycles.wrapping_add(self.step);
            CycleCount(self.cycles)
        }
    }

    #[test]
    fn a_counter_that_never_advances_ends_the_wait_as_a_timeout() {
        let mut samples = 0u32;
        let readiness = await_readiness(
            CLOCK_READY_WAIT,
            || {
                samples += 1;
                assert!(samples < 1_000, "the wait did not end on a frozen counter");
                Observation {
                    sample: Sample::NotReady,
                    at: CycleCount(0xFFFF_FF00),
                }
            },
            || {},
        );
        assert_eq!(readiness, Readiness::TimedOut);
        assert_eq!(samples, 1 + FROZEN_SAMPLE_LIMIT);
    }

    #[test]
    fn a_lock_that_arrives_on_a_frozen_counter_is_reached() {
        let mut status = FakeStatus::new(Some(3));
        let mut clock = FakeClock::stepping_by(0);
        let readiness = await_readiness(CLOCK_READY_WAIT, || status.observe(&mut clock), || {});
        assert_eq!(readiness, Readiness::Reached);
        assert_eq!(status.samples, 3);
    }

    #[test]
    fn a_wait_that_straddles_the_thirty_two_bit_wrap_times_out_on_the_budget() {
        let budget = CycleBudget(1_000);
        let started_at = CycleCount(0xFFFF_FF00);
        let waiting = Wait::Waiting {
            started_at,
            last_observed: started_at,
            budget,
            frozen_samples: 0,
        };
        let just_after_the_wrap = Observation {
            sample: Sample::NotReady,
            at: CycleCount(0x0000_0100),
        };
        let at_the_budget = Observation {
            sample: Sample::NotReady,
            at: CycleCount(0x0000_02E8),
        };
        assert!(!budget.elapsed_by(started_at, just_after_the_wrap.at));
        assert!(budget.elapsed_by(started_at, at_the_budget.at));
        assert_eq!(
            transition(waiting, just_after_the_wrap),
            (
                Wait::Waiting {
                    started_at,
                    last_observed: just_after_the_wrap.at,
                    budget,
                    frozen_samples: 0,
                },
                None
            )
        );
        assert_eq!(
            transition(waiting, at_the_budget),
            (
                Wait::Settled(Readiness::TimedOut),
                Some(Readiness::TimedOut)
            )
        );
        let mut clock = FakeClock::starting_at(0xFFFF_FD00, 0x200);
        let mut samples = 0u32;
        let readiness = await_readiness(
            budget,
            || {
                samples += 1;
                assert!(samples < 10, "the wait did not end across the wrap");
                Observation {
                    sample: Sample::NotReady,
                    at: clock.advance(),
                }
            },
            || {},
        );
        assert_eq!(readiness, Readiness::TimedOut);
        assert_eq!(samples, 3);
        assert_eq!(clock.cycles, 0x0000_0300);
    }

    #[test]
    fn a_lock_that_arrives_is_reached_before_the_budget_runs_out() {
        let mut status = FakeStatus::new(Some(3));
        let mut clock = FakeClock::stepping_by(1_000);
        let readiness = await_readiness(CLOCK_READY_WAIT, || status.observe(&mut clock), || {});
        assert_eq!(readiness, Readiness::Reached);
        assert_eq!(status.samples, 3);
    }

    #[test]
    fn a_lock_that_never_arrives_times_out_after_the_budget() {
        let mut status = FakeStatus::new(None);
        let mut clock = FakeClock::stepping_by(1_000);
        let readiness = await_readiness(CLOCK_READY_WAIT, || status.observe(&mut clock), || {});
        assert_eq!(readiness, Readiness::TimedOut);
        assert_eq!(status.samples, 1 + CLOCK_READY_WAIT.0 / 1_000);
        assert_eq!(clock.cycles, CLOCK_READY_WAIT.0 + 1_000);
    }

    #[test]
    fn a_crystal_that_never_reports_ready_times_out_within_the_bounds_the_budget_sets() {
        let mut status = FakeStatus::new(None);
        let mut clock = FakeClock::stepping_by(CLOCK_READY_WAIT.0 / 1_000);
        let readiness = await_readiness(CLOCK_READY_WAIT, || status.observe(&mut clock), || {});
        assert_eq!(readiness, Readiness::TimedOut);
        assert_eq!(status.samples, 1_001);
        assert_eq!(
            u64::from(CLOCK_READY_WAIT.0) * 1_000 / u64::from(TIMEOUT_BUDGET_CLOCK.0),
            100
        );
        assert_eq!(
            u64::from(CLOCK_READY_WAIT.0) * 1_000 / u64::from(SLOWEST_CORE_CLOCK.0),
            1_250
        );
    }

    #[test]
    fn the_watchdog_is_fed_twice_inside_the_shortest_timeout_at_one_megahertz() {
        assert_eq!(SHORTEST_WATCHDOG_TIMEOUT.0, 25);
        assert_eq!(WATCHDOG_FEED_WAIT.0, 12_000);
        let milliseconds_at =
            |clock: Hertz| u64::from(WATCHDOG_FEED_WAIT.0) * 1_000 / u64::from(clock.0);
        assert_eq!(milliseconds_at(SLOWEST_BOOTLOADER_CORE_CLOCK), 12);
        assert!(
            milliseconds_at(SLOWEST_BOOTLOADER_CORE_CLOCK) * 2
                <= u64::from(SHORTEST_WATCHDOG_TIMEOUT.0)
        );
        let microseconds_at =
            |clock: Hertz| u64::from(WATCHDOG_FEED_WAIT.0) * 1_000_000 / u64::from(clock.0);
        assert_eq!(microseconds_at(TIMEOUT_BUDGET_CLOCK), 80);
        assert_eq!(
            u64::from(SHORTEST_WATCHDOG_TIMEOUT.0) * 1_000 / microseconds_at(TIMEOUT_BUDGET_CLOCK),
            312
        );
    }

    #[test]
    fn every_wait_state_and_sample_pair_moves_to_the_state_the_table_names() {
        let budget = CycleBudget(1_000);
        let started_at = CycleCount(500);
        let waiting = Wait::Waiting {
            started_at,
            last_observed: started_at,
            budget,
            frozen_samples: 0,
        };
        let ready = Observation {
            sample: Sample::Ready,
            at: CycleCount(1_499),
        };
        let not_ready = Observation {
            sample: Sample::NotReady,
            at: CycleCount(1_499),
        };
        let at_the_deadline = Observation {
            sample: Sample::NotReady,
            at: CycleCount(1_500),
        };
        let still_waiting = Wait::Waiting {
            started_at,
            last_observed: not_ready.at,
            budget,
            frozen_samples: 0,
        };
        assert_eq!(
            transition(Wait::Starting(budget), ready),
            (Wait::Settled(Readiness::Reached), Some(Readiness::Reached))
        );
        let at_the_start = Observation {
            sample: Sample::NotReady,
            at: started_at,
        };
        assert_eq!(
            transition(Wait::Starting(budget), at_the_start),
            (waiting, None)
        );
        assert_eq!(transition(waiting, not_ready), (still_waiting, None));
        assert_eq!(
            transition(
                Wait::Waiting {
                    started_at,
                    last_observed: started_at,
                    budget,
                    frozen_samples: 1,
                },
                at_the_start
            ),
            (
                Wait::Waiting {
                    started_at,
                    last_observed: started_at,
                    budget,
                    frozen_samples: 2,
                },
                None
            )
        );
        assert_eq!(
            transition(
                Wait::Waiting {
                    started_at,
                    last_observed: started_at,
                    budget,
                    frozen_samples: FROZEN_SAMPLE_LIMIT - 1,
                },
                at_the_start
            ),
            (
                Wait::Settled(Readiness::TimedOut),
                Some(Readiness::TimedOut)
            )
        );
        assert_eq!(
            transition(waiting, at_the_deadline),
            (
                Wait::Settled(Readiness::TimedOut),
                Some(Readiness::TimedOut)
            )
        );
        assert_eq!(
            transition(waiting, ready),
            (Wait::Settled(Readiness::Reached), Some(Readiness::Reached))
        );
        assert_eq!(
            transition(Wait::Settled(Readiness::Reached), not_ready),
            (Wait::Settled(Readiness::Reached), Some(Readiness::Reached))
        );
        assert_eq!(
            transition(Wait::Settled(Readiness::TimedOut), ready),
            (
                Wait::Settled(Readiness::TimedOut),
                Some(Readiness::TimedOut)
            )
        );
    }

    #[test]
    fn no_random_sequence_reports_reached_on_a_sample_that_was_not_ready() {
        let mut seed = 0x1234_5678u32;
        for _ in 0..1_000 {
            let budget = CycleBudget(64);
            let mut clock = FakeClock::stepping_by(1);
            let mut wait = Wait::starting(budget);
            for _ in 0..200 {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let sample = if seed >> 31 == 0 {
                    Sample::Ready
                } else {
                    Sample::NotReady
                };
                let observation = Observation {
                    sample,
                    at: clock.advance(),
                };
                let (next, outcome) = transition(wait, observation);
                if !matches!(wait, Wait::Settled(_)) && outcome == Some(Readiness::Reached) {
                    assert_eq!(sample, Sample::Ready);
                }
                if let Wait::Settled(settled) = wait {
                    assert_eq!(outcome, Some(settled));
                }
                wait = next;
            }
            assert!(matches!(wait, Wait::Settled(_)));
        }
    }
}
