use crate::lcd::panel::{Hertz, Microseconds};
use crate::pll::{CycleBudget, TIMEOUT_BUDGET_CLOCK};

pub const FLEXCOMM_CLOCK: Hertz = Hertz(24_000_000);
pub const CLOCK_DIVISOR: u32 = 15;
pub const SCK: Hertz = Hertz(FLEXCOMM_CLOCK.0 / CLOCK_DIVISOR);
pub const WORD_FRAME_BITS: u32 = 16;
pub const SHIFT_BYTE_FRAME_BITS: u32 = 8;
pub const PAD_FRAME_BITS: u32 = 8;
pub const TRANSFER_BITS: u32 = WORD_FRAME_BITS + SHIFT_BYTE_FRAME_BITS + PAD_FRAME_BITS;

const MICROSECONDS_PER_SECOND: u32 = 1_000_000;
const MILLISECONDS_PER_SECOND: u32 = 1_000;

pub const LONGEST_TRANSFER: Microseconds =
    Microseconds(TRANSFER_BITS * MICROSECONDS_PER_SECOND / SCK.0);
pub const ACKNOWLEDGE_DEADLINE: CycleBudget =
    CycleBudget(TIMEOUT_BUDGET_CLOCK.0 / MILLISECONDS_PER_SECOND);

const _: () = assert!(
    ACKNOWLEDGE_DEADLINE.0 as u64 * MICROSECONDS_PER_SECOND as u64
        >= LONGEST_TRANSFER.0 as u64 * TIMEOUT_BUDGET_CLOCK.0 as u64,
    "the acknowledge deadline covers the longest transfer at the clock its cycles are priced at"
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pll::{
        CycleCount, Observation, Readiness, SLOWEST_CORE_CLOCK, Sample, TIMEOUT_BUDGET_CLOCK,
        await_readiness,
    };

    struct FakeBus {
        latches_after: Option<u32>,
        samples: u32,
    }

    impl FakeBus {
        const fn new(latches_after: Option<u32>) -> Self {
            Self {
                latches_after,
                samples: 0,
            }
        }

        fn observe(&mut self, clock: &mut FakeClock) -> Observation {
            self.samples += 1;
            let sample = match self.latches_after {
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

        fn advance(&mut self) -> CycleCount {
            self.cycles = self.cycles.wrapping_add(self.step);
            CycleCount(self.cycles)
        }
    }

    #[test]
    fn a_word_the_bus_latches_inside_the_deadline_is_reached() {
        let mut bus = FakeBus::new(Some(3));
        let mut clock = FakeClock::stepping_by(1_000);
        let readiness = await_readiness(ACKNOWLEDGE_DEADLINE, || bus.observe(&mut clock), || {});
        assert_eq!(readiness, Readiness::Reached);
        assert_eq!(bus.samples, 3);
    }

    #[test]
    fn a_bus_that_never_latches_times_out_at_the_deadline() {
        let mut bus = FakeBus::new(None);
        let mut clock = FakeClock::stepping_by(1_000);
        let readiness = await_readiness(ACKNOWLEDGE_DEADLINE, || bus.observe(&mut clock), || {});
        assert_eq!(readiness, Readiness::TimedOut);
        assert_eq!(bus.samples, 1 + ACKNOWLEDGE_DEADLINE.0 / 1_000);
        assert_eq!(clock.cycles, ACKNOWLEDGE_DEADLINE.0 + 1_000);
    }

    #[test]
    fn the_deadline_is_one_millisecond_at_the_pricing_clock_and_covers_the_longest_transfer_at_both()
     {
        assert_eq!(SCK, Hertz(1_600_000));
        assert_eq!(LONGEST_TRANSFER, Microseconds(20));
        let microseconds_at =
            |clock: Hertz| u64::from(ACKNOWLEDGE_DEADLINE.0) * 1_000_000 / u64::from(clock.0);
        assert_eq!(microseconds_at(TIMEOUT_BUDGET_CLOCK), 1_000);
        assert_eq!(microseconds_at(SLOWEST_CORE_CLOCK), 12_500);
        for clock in [TIMEOUT_BUDGET_CLOCK, SLOWEST_CORE_CLOCK] {
            assert!(
                microseconds_at(clock) >= u64::from(LONGEST_TRANSFER.0),
                "the deadline covers the longest transfer at {} Hz",
                clock.0
            );
        }
        assert_eq!(
            TRANSFER_BITS,
            WORD_FRAME_BITS + SHIFT_BYTE_FRAME_BITS + PAD_FRAME_BITS
        );
    }
}
