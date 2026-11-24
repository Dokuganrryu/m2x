use device::cycles::down_count_fall;

const FACTORY_PERIOD: u32 = 100_001;
const PERIOD_AT_150_MHZ: u32 = 150_000;

#[test]
fn the_factory_period_against_a_150_mhz_reload_counts_billions_of_cycles_at_one_wrap() {
    let (last, current) = (1_000u32, 149_000u32);
    let stale = last.wrapping_add(FACTORY_PERIOD).wrapping_sub(current);
    assert!(
        stale > 4_000_000_000,
        "the silicon fault: {stale} cycles in one sample"
    );
}

#[test]
fn a_wrap_counts_the_cycles_to_zero_plus_the_cycles_down_from_the_live_reload() {
    assert_eq!(down_count_fall(1_000, 149_000, PERIOD_AT_150_MHZ), 2_000);
    assert_eq!(down_count_fall(0, 149_999, PERIOD_AT_150_MHZ), 1);
    assert_eq!(down_count_fall(5_000, 4_000, PERIOD_AT_150_MHZ), 1_000);
    assert_eq!(down_count_fall(500, 99_000, FACTORY_PERIOD), 1_501);
}

#[test]
fn one_sample_never_counts_more_than_one_period_whatever_the_reload() {
    for period in [FACTORY_PERIOD, PERIOD_AT_150_MHZ, 96_001] {
        for last in [0, 1, 50_000, period - 1] {
            for current in [0, 1, 70_000, period - 1, period, u32::MAX] {
                assert!(
                    down_count_fall(last, current, period) <= period,
                    "{last} {current} {period}"
                );
            }
        }
    }
}

#[test]
fn a_100_ms_wait_at_150_mhz_lasts_100_ms_across_every_wrap() {
    let mut elapsed = 0u32;
    let mut last = 149_999u32;
    let mut cycles = 0u64;
    while elapsed < 15_000_000 {
        cycles += 997;
        let current = 149_999 - (cycles % u64::from(PERIOD_AT_150_MHZ)) as u32;
        elapsed += down_count_fall(last, current, PERIOD_AT_150_MHZ);
        last = current;
    }
    assert!(
        (14_999_000..=15_001_000).contains(&cycles),
        "{cycles} cycles"
    );
}
