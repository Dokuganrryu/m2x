use device::core_clock::{
    CORE_150_MHZ_TICK_CYCLES, Corner, Nmpa, NmpaWords, PLL1_NDEC_FOR_150_MHZ, PmcWords,
    SYSTICK_RELOAD_AT_150_MHZ, VoltageSource, plan_for_150_mhz, voltage_reached,
};

const LIVE_UNIT: PmcWords = PmcWords {
    dcdc0: 0x0108_5E7D,
    dcdc1: 0x01B0_44C8,
    ldopmu: 0x010E_F718,
};

const fn vout(dcdc0: u32) -> u32 {
    (dcdc0 >> 17) & 0xF
}

const fn vadj(ldopmu: u32) -> u32 {
    ldopmu & 0x1F
}

const fn boost(ldopmu: u32) -> u32 {
    (ldopmu >> 10) & 0x1F
}

fn ringo(hz: u32) -> u32 {
    (hz << 1) | 1
}

fn nmpa(dcdc: [u32; 2], ringo: [u32; 2]) -> Nmpa {
    Nmpa::Read(NmpaWords {
        dcdc_high: dcdc,
        ringo,
    })
}

#[test]
fn an_unreadable_nmpa_plans_nxps_slow_corner_row_and_leaves_the_dcdc_trims() {
    let plan = plan_for_150_mhz(Nmpa::Unreadable, LIVE_UNIT);
    assert_eq!(plan.corner, Corner::Slow);
    assert_eq!(plan.source, VoltageSource::NmpaUnreadable);
    assert_eq!(plan.millivolts, 1200);
    assert_eq!(vout(plan.writes.dcdc0), 10);
    assert_eq!(vadj(plan.writes.ldopmu), 30);
    assert_eq!(boost(plan.writes.ldopmu), 31);
    assert_eq!(plan.writes.dcdc1, LIVE_UNIT.dcdc1);
    assert_eq!(
        plan.writes.dcdc0 & !(0xF << 17),
        LIVE_UNIT.dcdc0 & !(0xF << 17)
    );
    assert_eq!(
        plan.writes.ldopmu & !(0x1F | 0x1F << 10),
        LIVE_UNIT.ldopmu & !(0x1F | 0x1F << 10)
    );
}

#[test]
fn nmpa_words_with_no_valid_bit_count_as_unreadable_and_plan_the_slow_row() {
    let plan = plan_for_150_mhz(nmpa([0, 0], [0, 0]), LIVE_UNIT);
    assert_eq!(plan.corner, Corner::Slow);
    assert_eq!(plan.source, VoltageSource::NmpaInvalid);
    assert_eq!(plan.writes.dcdc1, LIVE_UNIT.dcdc1);
}

#[test]
fn the_corner_follows_nxps_ring_oscillator_limits_from_the_slower_ring() {
    for (hz, corner, millivolts) in [
        (16_900_000, Corner::Slow, 1200),
        (16_900_001, Corner::Nominal, 1150),
        (19_300_000, Corner::Nominal, 1150),
        (21_700_000, Corner::Nominal, 1150),
        (21_700_001, Corner::Fast, 1050),
    ] {
        let plan = plan_for_150_mhz(nmpa([0, 0], [ringo(hz), ringo(25_000_000)]), LIVE_UNIT);
        assert_eq!((plan.corner, plan.millivolts), (corner, millivolts), "{hz}");
    }
    let one_invalid = plan_for_150_mhz(nmpa([0, 0], [ringo(25_000_000), 0]), LIVE_UNIT);
    assert_eq!(
        one_invalid.corner,
        Corner::Nominal,
        "NXP counts an invalid ring as 19.3 MHz"
    );
}

#[test]
fn a_valid_high_profile_trim_is_written_with_the_vout_never_below_the_corner_or_now() {
    let trim0 = 0x0123_4567;
    let trim1 = 0x0089_ABCD;
    let plan = plan_for_150_mhz(
        nmpa([(trim0 << 1) | 1, trim1], [ringo(19_300_000); 2]),
        LIVE_UNIT,
    );
    assert_eq!(plan.writes.dcdc1, trim1);
    assert_eq!(plan.writes.dcdc0 & !(0xF << 17), trim0 & !(0xF << 17));
    assert_eq!(vout(plan.writes.dcdc0), 8);
    assert_eq!(plan.source, VoltageSource::Nmpa);
}

#[test]
fn no_field_is_ever_planned_below_what_the_pmc_holds_now() {
    let fast = plan_for_150_mhz(nmpa([0, 0], [ringo(23_000_000); 2]), LIVE_UNIT);
    assert_eq!(fast.corner, Corner::Fast);
    assert_eq!(
        vout(fast.writes.dcdc0),
        vout(LIVE_UNIT.dcdc0),
        "FFF's 1.05 V is what runs now"
    );
    assert_eq!(
        vadj(fast.writes.ldopmu),
        24,
        "NXP's FFF VADJ 20 would lower 24"
    );
    assert_eq!(
        boost(fast.writes.ldopmu),
        29,
        "NXP's FFF BOOST 25 would lower 29"
    );
    for hz in [10_000_000, 19_300_000, 30_000_000] {
        for now in [
            LIVE_UNIT,
            PmcWords {
                dcdc0: 0x0014_0000 | 1,
                dcdc1: 0,
                ldopmu: 31 | 31 << 10,
            },
        ] {
            let plan = plan_for_150_mhz(nmpa([3, 0], [ringo(hz); 2]), now);
            assert!(vout(plan.writes.dcdc0) >= vout(now.dcdc0));
            assert!(vadj(plan.writes.ldopmu) >= vadj(now.ldopmu));
            assert!(boost(plan.writes.ldopmu) >= boost(now.ldopmu));
        }
    }
}

#[test]
fn the_voltage_counts_as_reached_only_when_every_planned_field_reads_back_at_least_as_high() {
    let plan = plan_for_150_mhz(Nmpa::Unreadable, LIVE_UNIT);
    assert!(voltage_reached(&plan, Some(plan.writes)));
    assert!(
        !voltage_reached(&plan, None),
        "an unreadable PMC never raises the clock"
    );
    assert!(
        !voltage_reached(&plan, Some(LIVE_UNIT)),
        "a write the PMC ignored"
    );
    let mut low_boost = plan.writes;
    low_boost.ldopmu = (low_boost.ldopmu & !(0x1F << 10)) | 30 << 10;
    assert!(!voltage_reached(&plan, Some(low_boost)));
}

#[test]
fn pll1_at_150_mhz_is_m25_n2_p1_from_the_24_mhz_crystal_with_the_cco_in_range() {
    assert_eq!(PLL1_NDEC_FOR_150_MHZ, 2);
    let cco = 24_000_000 / PLL1_NDEC_FOR_150_MHZ * 25;
    assert_eq!(cco, 300_000_000);
    assert!((275_000_000..=550_000_000).contains(&cco));
    assert_eq!(cco / 2, 150_000_000);
    assert_eq!(SYSTICK_RELOAD_AT_150_MHZ + 1, 150_000);
    assert_eq!(CORE_150_MHZ_TICK_CYCLES, 18_750);
}
