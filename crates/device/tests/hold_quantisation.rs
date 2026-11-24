use device::phantom::Milliseconds;
use device::steps::{Countdown, count_down, later, reached};

const MICROS_PER_TICK: u32 = 1000;

fn first_tick_after(start_micros: u32) -> u32 {
    (start_micros / MICROS_PER_TICK + 1) * MICROS_PER_TICK
}

fn waited_hold_ends_at(start_micros: u32, hold: Milliseconds) -> u32 {
    let mut waited = Milliseconds(0);
    let mut now = first_tick_after(start_micros);
    loop {
        waited = later(waited, Milliseconds(1));
        if reached(waited, hold) {
            return now;
        }
        now += MICROS_PER_TICK;
    }
}

fn counted_hold_ends_at(start_micros: u32, hold: Milliseconds) -> u32 {
    let mut remaining = hold;
    let mut now = first_tick_after(start_micros);
    loop {
        match count_down(remaining, Milliseconds(1)) {
            Countdown::Over => return now,
            Countdown::Left(left) => remaining = left,
        }
        now += MICROS_PER_TICK;
    }
}

#[test]
fn a_hold_of_n_milliseconds_lasts_at_least_n_milliseconds_from_any_point_in_a_free_running_tick() {
    for hold in 0..40 {
        for phase in (0..MICROS_PER_TICK).step_by(7) {
            let start = 5 * MICROS_PER_TICK + phase;
            for ends in [
                waited_hold_ends_at(start, Milliseconds(hold)),
                counted_hold_ends_at(start, Milliseconds(hold)),
            ] {
                assert!(
                    ends - start >= hold * MICROS_PER_TICK,
                    "hold {hold} ms from phase {phase} us ended after {} us",
                    ends - start
                );
                assert!(
                    ends - start <= (hold + 1) * MICROS_PER_TICK,
                    "hold {hold} ms from phase {phase} us ran {} us",
                    ends - start
                );
            }
        }
    }
}

#[test]
fn a_hold_ends_on_the_tick_after_n_whole_ticks_when_ticks_arrive_in_bursts() {
    assert_eq!(
        count_down(Milliseconds(3), Milliseconds(3)),
        Countdown::Left(Milliseconds(0))
    );
    assert_eq!(
        count_down(Milliseconds(3), Milliseconds(4)),
        Countdown::Over
    );
    assert!(!reached(
        later(Milliseconds(1), Milliseconds(2)),
        Milliseconds(3)
    ));
    assert!(reached(
        later(Milliseconds(2), Milliseconds(2)),
        Milliseconds(3)
    ));
}
