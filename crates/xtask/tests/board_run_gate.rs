use device::fault::{FaultCode, I2cPeer};
use xtask::board_run::{
    Access, AfterFault, AnalogWords, Check, Expectation, ExpectedEnding, FenceProbe, MainClock,
    Nanoseconds, ParkFunction, RateChange, Recorded, Silence, Trace, judge,
};

const BOOT_TO_THE_REFUSED_TRANSPORT: &str = "\
0 behavior idle-skip guess | a poll jumps time
1000000 flash-read 0x38200
1000000 flash-read 0x38204
1000000 breadcrumb 0x1000001
1000000 breadcrumb-magic 0x4d324243
1999999 write IOCON.P0_28 0x100
1999999 pin P1_26 low
1999999 pin P1_22 low
1999999 pin P1_10 low
1999999 write NVIC+0x80 0xffffffff
1999999 write DMA0.ENABLECLR0 0x50820
1999999 write CTIMER0.TCR 0x0
1999999 write FC7.CFG1 0x0
1999999 write FC6.CFG1 0x0
1999999 write FC2.CFG1 0x0
1999999 write FC0.CFG1 0x0
1999999 write USB1.DEVCMDSTAT 0x200
1999999 mpu enable ctrl=0x1 regions 0x20000000-0x2002ffe1
2000000 write PMC.RTCOSC32K 0x0
2000000 write PMC.PDRUNCFGCLR0 0x1411c0
2000000 write ANACTRL.XO32M_CTRL 0x1800000
2050000 read ANACTRL.XO32M_STATUS
2100000 write SYSCON.PLL1CTRL 0x201cf0
3400000 read SYSCON.PLL1STAT
3401000 clock main 100000000
3500000 write SYSCON.AHBCLKCTRLSET2 0x80000f0
3500000 write USBHSH+0x50 0x10000
3500000 write USBPHY+0x38 0x80000000
3500000 write USBPHY.PLL_SIC 0xa03040
3500000 write USBPHY.CTRL_SET 0x180000
3500000 read USBPHY.PLL_SIC
3500000 write USB1.EPLISTSTART 0x40100000
3500001 usb connect
9333416 write FC1.CFG 0x35
9416750 pin P1_4 low
9431750 preamp-word bits=24 value=0x000000
9500083 pin P1_3 low
9500083 pin P1_29 low
9500083 pin P1_22 high
51083833 pin P1_26 high
92750916 pin P1_10 high
926107583 preamp-word bits=24 value=0x000000
926108000 write FC3.CFG 0x1
926150000 write SYSCON.AHBCLKCTRLSET1 0x8000
926150000 write SYSCON.AHBCLKCTRLSET2 0x8000000
926150000 write ANACTRL.FRO192M_CTRL 0x40000000
926150000 write SYSCON.FROHFDIV 0x3
926150000 write SYSCON.FCCLKSEL4 0x3
926150000 write FC4+0xff8 0x2
934426000 write SYSCON.PLL0SSCG1 0x10200000
934426000 read SYSCON.PLL0STAT
937426030 write SYSCON.MCLKCLKSEL 0x1
937426030 write SYSCON.MCLKIO 0x1
937536030 i2c write 0x49 0x10,0xff ack
938516040 i2c write 0x49 0x11,0xff ack
939516050 i2c write 0x49 0x12,0xff ack
940516060 i2c write 0x49 0x13,0xff ack
941516070 i2c write 0x49 0x14,0xff ack
942526080 i2c write 0x49 0x15,0xff ack
943516090 i2c write 0x49 0x16,0xff ack
944516100 i2c write 0x49 0x17,0xff ack
950426160 pin P1_3 high
950426160 pin P1_29 high
1050457160 i2c address 0x13 ack
1052457180 i2c address 0x4f nack-address
1053437180 write CTIMER0.TCR 0x0
1053437180 write DMA0.ENABLECLR0 0x50820
1053437180 write FC7.CFG1 0x0
1053437180 write FC6.CFG1 0x0
1053437180 write FC2.CFG1 0x0
1053437180 write FC0.CFG1 0x0
1053437190 write DMA0.ENABLECLR0 0x820
1053437190 breadcrumb 0xa8b040a
1053452190 preamp-word bits=24 value=0x000000
1501431670 end time-limit
1501431670 mpu-probe 0x40034000 write fault
1501431670 mpu-probe 0x40033ffc write allowed
";

const FEED_EVERY_FORTY_MILLISECONDS_THEN_EVERY_FIVE: (u64, u64, u64, u64) =
    (40_000_000, 926_000_000, 5_000_000, 1_501_000_000);

fn with_feeds(boot: &str) -> String {
    let (slow, slow_until, fast, fast_until) = FEED_EVERY_FORTY_MILLISECONDS_THEN_EVERY_FIVE;
    let mut text = boot.to_string() + "1999999 write WWDT.TC 0xffffff\n";
    let mut at = 1_999_999 + slow;
    while at < slow_until {
        text.push_str(&format!("{at} watchdog feed\n"));
        at += slow;
    }
    while at < fast_until {
        text.push_str(&format!("{at} watchdog feed\n"));
        at += fast;
    }
    text
}

const FENCE: [FenceProbe; 2] = [
    FenceProbe {
        address: 0x4003_4000,
        access: Access::Write,
        allowed: false,
    },
    FenceProbe {
        address: 0x4003_3FFC,
        access: Access::Write,
        allowed: true,
    },
];

fn refused_transport() -> Expectation {
    Expectation {
        ending: ExpectedEnding::LoopRunsOn,
        record: Some(Recorded::raised(FaultCode::RefusedCommand, 10, 4)),
        quiet_after: Some(Nanoseconds::from_milliseconds(1_100)),
        fence: &FENCE,
        planted_store: None,
        after_fault: AfterFault::None,
        silence: Silence::WordAndTransmitDma,
        analog_words: AnalogWords::Latched,
        rate_change: RateChange::Ordered,
        main_clock: MainClock::MaySwitch,
    }
}

const PARK_FUNCTIONS: [ParkFunction; 2] = [
    ParkFunction {
        name: "board::firmware::fault::enter",
        start: 0x2000_1000,
        end: 0x2000_1400,
    },
    ParkFunction {
        name: "board::firmware::fault::park",
        start: 0x2000_2000,
        end: 0x2000_2040,
    },
];

fn failures(text: &str, expectation: &Expectation) -> Vec<(Check, String)> {
    let trace = Trace::parse(text).expect("the trace reads");
    judge(&trace, expectation, &PARK_FUNCTIONS)
        .into_iter()
        .filter_map(|verdict| verdict.failure.map(|failure| (verdict.check, failure)))
        .collect()
}

fn failed_checks(text: &str, expectation: &Expectation) -> Vec<Check> {
    failures(text, expectation)
        .into_iter()
        .map(|(check, _)| check)
        .collect()
}

fn changed(from: &str, to: &str) -> String {
    let boot = with_feeds(BOOT_TO_THE_REFUSED_TRANSPORT);
    assert!(boot.contains(from), "the fixture holds `{from}`");
    boot.replacen(from, to, 1)
}

fn added(line: &str) -> String {
    with_feeds(BOOT_TO_THE_REFUSED_TRANSPORT) + line + "\n"
}

fn without_the_pll0_lock_read() -> String {
    let boot = with_feeds(BOOT_TO_THE_REFUSED_TRANSPORT);
    assert!(boot.contains("934426000 read SYSCON.PLL0STAT"));
    boot.replacen("934426000 read SYSCON.PLL0STAT\n", "", 1)
}

fn moved_after((from, to): (&str, &str), after: &str) -> String {
    let boot = with_feeds(BOOT_TO_THE_REFUSED_TRANSPORT);
    assert!(boot.contains(from), "the fixture holds `{from}`");
    assert!(boot.contains(after), "the fixture holds `{after}`");
    boot.replacen(&format!("{from}\n"), "", 1)
        .replacen(after, &format!("{after}\n{to}"), 1)
}

#[test]
fn the_boot_the_image_reaches_today_passes_every_check() {
    assert_eq!(
        failures(
            &with_feeds(BOOT_TO_THE_REFUSED_TRANSPORT),
            &refused_transport()
        ),
        vec![]
    );
}

#[test]
fn every_check_is_judged_and_named() {
    let trace = Trace::parse(&with_feeds(BOOT_TO_THE_REFUSED_TRANSPORT)).expect("reads");
    let verdicts = judge(&trace, &refused_transport(), &PARK_FUNCTIONS);
    assert_eq!(verdicts.len(), Check::ALL.len());
    for verdict in verdicts {
        assert!(!verdict.check.name().is_empty());
    }
}

#[test]
fn a_rail_raised_before_the_all_off_word_fails_the_first_word_check() {
    let text = changed(
        "9431750 preamp-word bits=24 value=0x000000",
        "9600000 preamp-word bits=24 value=0x000000",
    );
    assert!(failed_checks(&text, &refused_transport()).contains(&Check::AllOffWordFirst));
}

#[test]
fn a_first_word_that_is_not_all_off_fails_the_first_word_check() {
    let text = changed(
        "9431750 preamp-word bits=24 value=0x000000",
        "9431750 preamp-word bits=24 value=0x000500",
    );
    assert!(failed_checks(&text, &refused_transport()).contains(&Check::AllOffWordFirst));
}

#[test]
fn a_phantom_bit_in_any_latched_word_fails_the_phantom_check() {
    let text = changed(
        "926107583 preamp-word bits=24 value=0x000000",
        "926107583 preamp-word bits=24 value=0x400000",
    );
    assert!(failed_checks(&text, &refused_transport()).contains(&Check::PhantomNeverRaised));
}

#[test]
fn the_phantom_pin_driven_high_fails_the_phantom_check() {
    let text = added("1200000000 pin P1_4 high");
    assert!(failed_checks(&text, &refused_transport()).contains(&Check::PhantomNeverRaised));
}

#[test]
fn a_word_cut_short_fails_the_whole_word_check() {
    let text = changed(
        "1053452190 preamp-word bits=24 value=0x000000",
        "1053452190 preamp-word bits=13 value=0x000000",
    );
    assert!(failed_checks(&text, &refused_transport()).contains(&Check::PreampWordsWhole));
}

#[test]
fn outputs_connected_before_every_volume_register_is_attenuated_fails_the_attenuation_check() {
    let text = changed(
        "944516100 i2c write 0x49 0x17,0xff ack",
        "944516100 pin P0_28 high",
    );
    assert!(failed_checks(&text, &refused_transport()).contains(&Check::DacAttenuatedBeforeOutput));
}

#[test]
fn outputs_connected_before_the_master_clock_fails_the_attenuation_check() {
    let text = changed(
        "937426030 write SYSCON.MCLKIO 0x1",
        "937426030 pin P0_28 high",
    );
    assert!(failed_checks(&text, &refused_transport()).contains(&Check::DacAttenuatedBeforeOutput));
}

#[test]
fn i2s_enabled_after_every_volume_register_is_attenuated_passes_the_attenuation_check() {
    let text = added("944600000 write FC7.CFG1 0x1f0231");
    assert!(
        !failed_checks(&text, &refused_transport()).contains(&Check::DacAttenuatedBeforeOutput)
    );
}

#[test]
fn a_volume_write_nacked_does_not_count_as_attenuated() {
    let text = changed(
        "944516100 i2c write 0x49 0x17,0xff ack",
        "944516100 i2c write 0x49 0x17,0xff nack-data@1",
    ) + "945000000 pin P0_28 high\n";
    assert!(failed_checks(&text, &refused_transport()).contains(&Check::DacAttenuatedBeforeOutput));
}

const OUTPUTS_UP_THEN_RAMPED_THEN_DOWN_AFTER_A_HOST_RATE_REQUEST: &str = "\
1000000000 pin P0_28 high
1000100000 i2c write 0x49 0x12,0x40 ack
1000150000 handover-write 0x20000000 0xbb80
1000200000 pin P0_28 low
";

#[test]
fn outputs_reconnected_louder_than_before_they_fell_fail_the_attenuation_check() {
    let text = added(OUTPUTS_UP_THEN_RAMPED_THEN_DOWN_AFTER_A_HOST_RATE_REQUEST)
        + "1000300000 i2c write 0x49 0x12,0x20 ack\n\
           1000400000 pin P0_28 high";
    let failures = failures(&text, &refused_transport());
    let reason = failures
        .iter()
        .find(|(check, _)| *check == Check::DacAttenuatedBeforeOutput)
        .map(|(_, reason)| reason.as_str());
    assert!(
        reason
            .is_some_and(|reason| reason.contains("1000.400 ms")
                && reason.contains("(18, Some(32), Some(64))")),
        "{reason:?}"
    );
}

#[test]
fn outputs_reconnected_with_a_register_unacknowledged_since_boot_fail_the_attenuation_check() {
    let text = added(OUTPUTS_UP_THEN_RAMPED_THEN_DOWN_AFTER_A_HOST_RATE_REQUEST)
        + "1000300000 i2c write 0x49 0x16,0x00 nack-data@1\n\
           1000400000 pin P0_28 high";
    assert!(failed_checks(&text, &refused_transport()).contains(&Check::DacAttenuatedBeforeOutput));
}

#[test]
fn outputs_reconnected_at_full_attenuation_or_at_most_as_loud_as_before_pass_the_attenuation_check()
{
    for after_the_fall in [
        "1000300000 i2c write 0x49 0x12,0xff ack\n",
        "1000300000 i2c write 0x49 0x12,0x40 ack\n",
        "1000300000 i2c write 0x49 0x12,0x60 ack\n",
        "",
    ] {
        let text = added(OUTPUTS_UP_THEN_RAMPED_THEN_DOWN_AFTER_A_HOST_RATE_REQUEST)
            + after_the_fall
            + "1000400000 pin P0_28 high";
        let failures = failures(&text, &refused_transport());
        assert!(
            failures
                .iter()
                .all(|(check, _)| *check != Check::DacAttenuatedBeforeOutput),
            "{after_the_fall:?}: {failures:?}"
        );
    }
}

#[test]
fn a_master_trim_louder_than_minus_one_decibel_fails_the_trim_check() {
    let text = added(
        "1000000000 i2c write 0x49 0x18,0xff ack\n\
         1000100000 i2c write 0x49 0x19,0xff ack\n\
         1000200000 i2c write 0x49 0x1a,0xff ack\n\
         1000300000 i2c write 0x49 0x1b,0x7f ack",
    );
    assert!(
        failed_checks(&text, &refused_transport()).contains(&Check::MasterTrimAtMostMinusOneDb)
    );
}

#[test]
fn the_factory_minus_one_decibel_trim_passes_the_trim_check() {
    let text = added(
        "1000000000 i2c write 0x49 0x18,0xbf ack\n\
         1000100000 i2c write 0x49 0x19,0x82 ack\n\
         1000200000 i2c write 0x49 0x1a,0x14 ack\n\
         1000300000 i2c write 0x49 0x1b,0x72 ack",
    );
    assert!(
        !failed_checks(&text, &refused_transport()).contains(&Check::MasterTrimAtMostMinusOneDb)
    );
}

#[test]
fn a_write_to_dac_register_zero_or_sixty_two_fails_even_when_nacked() {
    for register in ["0x00", "0x3e"] {
        let text = added(&format!(
            "1000000000 i2c write 0x49 {register},0x00 nack-address"
        ));
        assert!(
            failed_checks(&text, &refused_transport())
                .contains(&Check::DacRegistersZeroAndSixtyTwoUntouched),
            "register {register}"
        );
    }
}

#[test]
fn any_flash_controller_access_or_factory_write_or_stray_flash_read_fails_the_flash_check() {
    for line in [
        "1000000000 flash-controller read 0x40034010",
        "1000000000 flash-write 0x38400 0x0",
        "1000000000 secure-alias read 0x50034000",
        "1000000000 flash-read 0x76000",
    ] {
        assert!(
            failed_checks(&added(line), &refused_transport()).contains(&Check::FlashUntouched),
            "{line}"
        );
    }
}

#[test]
fn an_access_the_fence_faults_fails_the_fence_check() {
    let text = added("1000000000 mpu-fault 0x20030010 write no-region");
    assert!(failed_checks(&text, &refused_transport()).contains(&Check::FenceNeverTripped));
}

#[test]
fn a_probe_the_fence_answers_wrongly_fails_the_fence_programming_check() {
    let text = changed(
        "mpu-probe 0x40034000 write fault",
        "mpu-probe 0x40034000 write allowed",
    );
    assert_eq!(
        failed_checks(&text, &refused_transport()),
        vec![Check::FenceProgrammed]
    );
}

#[test]
fn a_main_clock_switch_with_no_read_of_the_pll_lock_bit_fails_the_lock_check() {
    let text = changed("3400000 read SYSCON.PLL1STAT", "");
    assert!(failed_checks(&text, &refused_transport()).contains(&Check::PllLockReadBeforeClockUse));
}

#[test]
fn a_read_of_the_pll_lock_bit_after_the_switch_at_the_same_timestamp_fails_the_lock_check() {
    let text = moved_after(
        (
            "3400000 read SYSCON.PLL1STAT",
            "3401000 read SYSCON.PLL1STAT",
        ),
        "3401000 clock main 100000000",
    );
    assert!(failed_checks(&text, &refused_transport()).contains(&Check::PllLockReadBeforeClockUse));
}

#[test]
fn selecting_pll0_for_mclk_with_no_read_of_its_lock_bit_fails_the_lock_check() {
    let text = without_the_pll0_lock_read() + "937426031 write SYSCON.MCLKCLKSEL 0x1\n";
    assert!(failed_checks(&text, &refused_transport()).contains(&Check::PllLockReadBeforeClockUse));
}

#[test]
fn the_images_pll0_selection_after_its_lock_read_passes_the_lock_check() {
    let text = with_feeds(BOOT_TO_THE_REFUSED_TRANSPORT);
    assert!(text.contains("934426000 read SYSCON.PLL0STAT"));
    assert!(text.contains("937426030 write SYSCON.MCLKCLKSEL 0x1"));
    assert!(
        !failed_checks(&text, &refused_transport()).contains(&Check::PllLockReadBeforeClockUse)
    );
}

#[test]
fn a_read_of_the_pll0_lock_bit_after_the_mclk_selection_fails_the_lock_check() {
    let text = moved_after(
        (
            "934426000 read SYSCON.PLL0STAT",
            "937426030 read SYSCON.PLL0STAT",
        ),
        "937426030 write SYSCON.MCLKCLKSEL 0x1",
    );
    assert!(failed_checks(&text, &refused_transport()).contains(&Check::PllLockReadBeforeClockUse));
}

#[test]
fn an_armed_watchdog_fails_the_never_enabled_check() {
    let text = added("9500000 watchdog arm tc=25000 mod=0x3");
    assert_eq!(
        failed_checks(&text, &refused_transport()),
        vec![Check::WatchdogNeverEnabled]
    );
}

#[test]
fn a_mode_write_that_sets_the_enable_bit_fails_the_never_enabled_check() {
    let enabling = added("9500000 write WWDT.MOD 0x3");
    assert_eq!(
        failed_checks(&enabling, &refused_transport()),
        vec![Check::WatchdogNeverEnabled]
    );
    let not_enabling = added("9500000 write WWDT.MOD 0x2");
    assert_eq!(failed_checks(&not_enabling, &refused_transport()), vec![]);
}

#[test]
fn a_quiesce_step_after_the_fence_is_raised_fails_the_boot_order_check() {
    let text = changed(
        "1999999 mpu enable ctrl=0x1 regions 0x20000000-0x2002ffe1",
        "1999998 mpu enable ctrl=0x1 regions 0x20000000-0x2002ffe1",
    );
    assert_eq!(
        failed_checks(&text, &refused_transport()),
        vec![Check::BootOrderD9]
    );
}

fn boot_order_failure(text: &str) -> Option<String> {
    failures(text, &refused_transport())
        .into_iter()
        .find(|(check, _)| *check == Check::BootOrderD9)
        .map(|(_, failure)| failure)
}

#[test]
fn a_quiesce_that_never_drives_a_rail_pin_low_fails_the_boot_order_check() {
    let text = changed(
        "1999999 pin P1_22 low
",
        "",
    );
    assert_eq!(
        boot_order_failure(&text).as_deref(),
        Some(
            "the first P1_22 level is high at 9.500 ms, and the quiesce drives P1_26, P1_22 and P1_10 low, in that order, after the P0_28 pad write and by the fence"
        )
    );
}

#[test]
fn rail_pins_driven_low_in_the_order_the_rails_raise_them_fail_the_boot_order_check() {
    let text = changed(
        "1999999 pin P1_26 low
1999999 pin P1_22 low
",
        "1999999 pin P1_22 low
1999999 pin P1_26 low
",
    );
    assert_eq!(
        boot_order_failure(&text).as_deref(),
        Some(
            "P1_22 low at 2.000 ms comes before P1_26 low, and the quiesce drives P1_26, P1_22 and P1_10 low, in that order, after the P0_28 pad write and by the fence"
        )
    );
}

#[test]
fn rail_pins_driven_low_before_the_p0_28_pad_write_fail_the_boot_order_check() {
    let text = moved_after(
        (
            "1999999 write IOCON.P0_28 0x100",
            "1999999 write IOCON.P0_28 0x100",
        ),
        "1999999 pin P1_10 low",
    );
    assert_eq!(
        boot_order_failure(&text).as_deref(),
        Some(
            "P1_26 low at 2.000 ms comes before the P0_28 pad write, and the quiesce drives P1_26, P1_22 and P1_10 low, in that order, after the P0_28 pad write and by the fence"
        )
    );
}

#[test]
fn a_rail_pin_driven_low_after_the_fence_fails_the_boot_order_check() {
    let text = changed(
        "1999999 pin P1_10 low
",
        "",
    )
    .replacen(
        "2000000 write PMC.RTCOSC32K 0x0
",
        "2000000 pin P1_10 low
2000000 write PMC.RTCOSC32K 0x0
",
        1,
    );
    assert_eq!(
        boot_order_failure(&text).as_deref(),
        Some(
            "P1_10 low at 2.000 ms comes after the fence at 2.000 ms, and the quiesce drives P1_26, P1_22 and P1_10 low, in that order, after the P0_28 pad write and by the fence"
        )
    );
}

#[test]
fn a_feed_gap_at_the_slow_edge_of_the_watchdog_clock_fails_the_starvation_check() {
    let boot = with_feeds(BOOT_TO_THE_REFUSED_TRANSPORT);
    let text: String = boot
        .lines()
        .filter(|line| {
            !line.ends_with("watchdog feed")
                || !(1_200_000_000..1_290_000_000)
                    .contains(&line.split(' ').next().unwrap().parse::<u64>().unwrap())
        })
        .map(|line| format!("{line}\n"))
        .collect();
    assert_eq!(
        failed_checks(&text, &refused_transport()),
        vec![Check::WatchdogNeverStarved]
    );
}

#[test]
fn a_feed_before_the_claim_sets_the_longest_timeout_fails_the_timeout_check() {
    let boot = with_feeds(BOOT_TO_THE_REFUSED_TRANSPORT);
    let written = "1999999 write WWDT.TC 0xffffff";
    assert!(boot.contains(written), "the fixture holds `{written}`");
    let never_written = boot.replacen(&format!("{written}\n"), "", 1);
    let shorter = boot.replacen(written, "1999999 write WWDT.TC 0x2000", 1);
    let after_the_first_feed = moved_after(
        (written, "41999999 write WWDT.TC 0xffffff"),
        "41999999 watchdog feed",
    );
    let shortened_after = boot.replacen(
        written,
        &format!("{written}\n1999999 write WWDT.TC 0x2000"),
        1,
    );
    for text in [
        never_written,
        shorter,
        after_the_first_feed,
        shortened_after,
    ] {
        assert_eq!(
            failed_checks(&text, &refused_transport()),
            vec![Check::WatchdogLongestTimeoutBeforeFirstFeed]
        );
    }
}

#[test]
fn a_usb_connect_after_the_rails_rise_fails_the_usb_check() {
    let text = changed("3500001 usb connect", "9600000 usb connect");
    let reason = failures(&text, &refused_transport())
        .into_iter()
        .find(|(check, _)| *check == Check::UsbConnectsBeforeTheRails)
        .map(|(_, reason)| reason);
    assert!(
        reason
            .as_deref()
            .is_some_and(|reason| reason.contains("after the first rail rose")),
        "{reason:?}"
    );
}

#[test]
fn rails_that_rise_with_usb_never_connected_fail_the_usb_check_unless_the_core_pll_failed() {
    let text = changed("3500001 usb connect\n", "");
    let reason = failures(&text, &refused_transport())
        .into_iter()
        .find(|(check, _)| *check == Check::UsbConnectsBeforeTheRails)
        .map(|(_, reason)| reason);
    assert!(
        reason
            .as_deref()
            .is_some_and(|reason| reason.contains("USB never connected")),
        "{reason:?}"
    );
    let core_pll_failed = Expectation {
        after_fault: AfterFault::PanelOnly,
        ..refused_transport()
    };
    assert!(!failed_checks(&text, &core_pll_failed).contains(&Check::UsbConnectsBeforeTheRails));
}

#[test]
fn a_usb_connect_before_the_core_pll_starts_fails_the_boot_order_check() {
    let text = changed("3500001 usb connect", "2050000 usb connect");
    assert!(failed_checks(&text, &refused_transport()).contains(&Check::BootOrderD9));
}

#[test]
fn a_usb_connect_with_no_list_start_write_since_the_last_connect_fails_the_list_start_check() {
    let never = added("1200000000 usb connect");
    assert!(
        failed_checks(&never, &refused_transport()).contains(&Check::UsbListStartSetBeforeConnect)
    );
    let once = added(
        "1200000000 write USB1.EPLISTSTART 0x40100000\n\
         1200000100 usb connect\n\
         1300000000 usb disconnect\n\
         1400000000 usb connect",
    );
    assert!(
        failed_checks(&once, &refused_transport()).contains(&Check::UsbListStartSetBeforeConnect)
    );
}

#[test]
fn a_list_start_away_from_the_usb_ram_list_fails_the_list_start_check() {
    let text = added("1200000000 write USB1.EPLISTSTART 0x40100100\n1200000100 usb connect");
    assert!(
        failed_checks(&text, &refused_transport()).contains(&Check::UsbListStartSetBeforeConnect)
    );
}

#[test]
fn a_wrong_list_start_written_after_the_last_connect_fails_the_list_start_check() {
    let text = added(
        "1200000000 write USB1.EPLISTSTART 0x40100000\n\
         1200000100 usb connect\n\
         1300000000 write USB1.EPLISTSTART 0x40100100",
    );
    assert!(
        failed_checks(&text, &refused_transport()).contains(&Check::UsbListStartSetBeforeConnect)
    );
}

#[test]
fn every_connect_after_its_own_list_start_write_passes_the_list_start_check() {
    let text = added(
        "1200000000 write USB1.EPLISTSTART 0x40100000\n\
         1200000100 usb connect\n\
         1300000000 usb disconnect\n\
         1400000000 write USB1.EPLISTSTART 0x40100000\n\
         1400000100 usb connect",
    );
    assert!(
        !failed_checks(&text, &refused_transport()).contains(&Check::UsbListStartSetBeforeConnect)
    );
}

fn power_check_failure(text: &str) -> Option<String> {
    failures(text, &refused_transport())
        .into_iter()
        .find(|(check, _)| *check == Check::UsbPhyPoweredBeforeTheClockWaits)
        .map(|(_, reason)| reason)
}

#[test]
fn a_usb_power_word_short_of_any_factory_bit_fails_the_power_check() {
    for short in [0x1410c0, 0x1401c0, 0x1011c0, 0x0411c0, 0x141180, 0x141140] {
        let text = changed(
            "2000000 write PMC.PDRUNCFGCLR0 0x1411c0",
            &format!("2000000 write PMC.PDRUNCFGCLR0 {short:#x}"),
        );
        let reason = power_check_failure(&text);
        assert!(
            reason
                .as_deref()
                .is_some_and(|reason| reason.contains("0x1411c0 never happened")),
            "{short:#x}: {reason:?}"
        );
    }
}

#[test]
fn usb_power_after_the_crystal_ready_read_fails_the_power_check() {
    for (line, later) in [
        (
            "2000000 write PMC.PDRUNCFGCLR0 0x1411c0",
            "2060000 write PMC.PDRUNCFGCLR0 0x1411c0",
        ),
        (
            "2000000 write ANACTRL.XO32M_CTRL 0x1800000",
            "2060000 write ANACTRL.XO32M_CTRL 0x1800000",
        ),
    ] {
        let text = moved_after((line, later), "2050000 read ANACTRL.XO32M_STATUS");
        assert!(power_check_failure(&text).is_some(), "{line}");
    }
}

#[test]
fn a_crystal_output_to_the_phy_pll_never_enabled_fails_the_power_check() {
    let text = changed(
        "2000000 write ANACTRL.XO32M_CTRL 0x1800000",
        "2000000 write ANACTRL.XO32M_CTRL 0x1000000",
    );
    assert!(power_check_failure(&text).is_some());
}

#[test]
fn a_32_khz_source_register_inherited_or_written_out_of_the_factory_order_fails_the_power_check() {
    let never = changed("2000000 write PMC.RTCOSC32K 0x0\n", "");
    assert!(power_check_failure(&never).is_some());
    let nonzero = changed(
        "2000000 write PMC.RTCOSC32K 0x0",
        "2000000 write PMC.RTCOSC32K 0x1",
    );
    assert!(power_check_failure(&nonzero).is_some());
    let after_the_power_word = moved_after(
        (
            "2000000 write PMC.RTCOSC32K 0x0",
            "2000000 write PMC.RTCOSC32K 0x0",
        ),
        "2000000 write PMC.PDRUNCFGCLR0 0x1411c0",
    );
    assert!(power_check_failure(&after_the_power_word).is_some());
    let after_the_phy = moved_after(
        (
            "2000000 write PMC.RTCOSC32K 0x0",
            "3500000 write PMC.RTCOSC32K 0x0",
        ),
        "3500000 write USBPHY+0x38 0x80000000",
    );
    assert!(power_check_failure(&after_the_phy).is_some());
}

#[test]
fn a_usb_phy_write_before_the_core_pll_lock_read_fails_the_power_check() {
    let text = moved_after(
        (
            "3500000 write USBPHY+0x38 0x80000000",
            "2100000 write USBPHY+0x38 0x80000000",
        ),
        "2100000 write SYSCON.PLL1CTRL 0x201cf0",
    );
    let reason = power_check_failure(&text);
    assert!(
        reason
            .as_deref()
            .is_some_and(|reason| reason.contains("comes before the core PLL lock read")),
        "{reason:?}"
    );
}

#[test]
fn a_boot_that_never_writes_the_usb_phy_passes_the_power_check() {
    let text = changed("3500000 write USBPHY+0x38 0x80000000\n", "")
        .replacen("3500000 write USBPHY.PLL_SIC 0xa03040\n", "", 1)
        .replacen("3500000 write USBPHY.CTRL_SET 0x180000\n", "", 1)
        .replacen("2000000 write PMC.RTCOSC32K 0x0\n", "", 1)
        .replacen("2000000 write PMC.PDRUNCFGCLR0 0x1411c0\n", "", 1);
    assert_eq!(power_check_failure(&text), None);
}

fn lock_check_failure(text: &str) -> Option<String> {
    failures(text, &refused_transport())
        .into_iter()
        .find(|(check, _)| *check == Check::UsbPhyPllLockReadBeforeConnect)
        .map(|(_, reason)| reason)
}

#[test]
fn a_connect_with_no_phy_pll_lock_read_fails_the_lock_read_check() {
    let text = changed("3500000 read USBPHY.PLL_SIC\n", "");
    let reason = lock_check_failure(&text);
    assert!(
        reason
            .as_deref()
            .is_some_and(|reason| reason.contains("no USBPHY.PLL_SIC lock read")),
        "{reason:?}"
    );
}

#[test]
fn a_phy_pll_lock_read_before_the_phy_is_ungated_fails_the_lock_read_check() {
    let text = moved_after(
        ("3500000 read USBPHY.PLL_SIC", "3500000 read USBPHY.PLL_SIC"),
        "3500000 write USBPHY.PLL_SIC 0xa03040",
    );
    let reason = lock_check_failure(&text);
    assert!(
        reason
            .as_deref()
            .is_some_and(|reason| reason.contains("with the PHY still gated")),
        "{reason:?}"
    );
}

#[test]
fn a_phy_pll_lock_read_after_the_connect_fails_the_lock_read_check() {
    let text = moved_after(
        ("3500000 read USBPHY.PLL_SIC", "3500002 read USBPHY.PLL_SIC"),
        "3500001 usb connect",
    );
    assert!(lock_check_failure(&text).is_some());
}

#[test]
fn a_phy_pll_lock_read_with_no_connect_after_it_fails_the_lock_read_check() {
    let text = changed("3500001 usb connect\n", "");
    let reason = lock_check_failure(&text);
    assert!(
        reason
            .as_deref()
            .is_some_and(|reason| reason.contains("USB never connected after it")),
        "{reason:?}"
    );
}

#[test]
fn a_second_connect_needs_its_own_phy_pll_lock_read() {
    let without = added(
        "1200000000 write USB1.EPLISTSTART 0x40100000\n\
         1200000000 write USBPHY.CTRL_SET 0x180000\n\
         1200000100 usb connect",
    );
    assert!(lock_check_failure(&without).is_some());
    let with = added(
        "1200000000 write USB1.EPLISTSTART 0x40100000\n\
         1200000000 write USBPHY.CTRL_SET 0x180000\n\
         1200000000 read USBPHY.PLL_SIC\n\
         1200000100 usb connect",
    );
    assert_eq!(lock_check_failure(&with), None);
}

#[test]
fn a_boot_that_never_attaches_passes_the_lock_read_check() {
    let text =
        changed("3500000 read USBPHY.PLL_SIC\n", "").replacen("3500001 usb connect\n", "", 1);
    assert_eq!(lock_check_failure(&text), None);
}

fn lcd_clock_failure(text: &str) -> Option<String> {
    failures(text, &refused_transport())
        .into_iter()
        .find(|(check, _)| *check == Check::LcdStartsItsOwnClockSource)
        .map(|(_, failure)| failure)
}

const SOURCE_BEFORE_THE_GATE: &str = "926108000 write ANACTRL.FRO192M_CTRL 0x40000000
926108000 write SYSCON.FROHFDIV 0x3
926150000 write SYSCON.AHBCLKCTRLSET1 0x8000
";

#[test]
fn an_lcd_set_up_that_leaves_the_clock_source_to_an_earlier_stage_fails_the_lcd_clock_check() {
    let text = changed(
        "926150000 write ANACTRL.FRO192M_CTRL 0x40000000
",
        "",
    )
    .replacen(
        "926150000 write SYSCON.FROHFDIV 0x3
",
        "",
        1,
    )
    .replacen(
        "926150000 write SYSCON.AHBCLKCTRLSET1 0x8000
",
        SOURCE_BEFORE_THE_GATE,
        1,
    );
    assert_eq!(
        lcd_clock_failure(&text).as_deref(),
        Some(
            "SYSCON.FCCLKSEL4 is written at 926.150 ms with no ANACTRL.FRO192M_CTRL write that sets bit 30 since the FC4 clock-gate write at 926.150 ms"
        )
    );
}

#[test]
fn a_clock_source_started_after_fcclksel4_fails_the_lcd_clock_check() {
    let text = moved_after(
        (
            "926150000 write SYSCON.FCCLKSEL4 0x3",
            "926150000 write SYSCON.FCCLKSEL4 0x3",
        ),
        "926150000 write SYSCON.AHBCLKCTRLSET1 0x8000",
    );
    assert_eq!(
        lcd_clock_failure(&text).as_deref(),
        Some(
            "SYSCON.FCCLKSEL4 is written at 926.150 ms with no ANACTRL.FRO192M_CTRL write that sets bit 30 since the FC4 clock-gate write at 926.150 ms"
        )
    );
}

#[test]
fn a_divider_left_halted_or_at_another_ratio_fails_the_lcd_clock_check() {
    for word in ["0x40000003", "0x1"] {
        let text = changed(
            "926150000 write SYSCON.FROHFDIV 0x3
",
            &format!(
                "926150000 write SYSCON.FROHFDIV {word}
"
            ),
        );
        assert_eq!(
            lcd_clock_failure(&text).as_deref(),
            Some(
                "SYSCON.FCCLKSEL4 is written at 926.150 ms with no SYSCON.FROHFDIV write of 3 after the ANACTRL.FRO192M_CTRL write since the FC4 clock-gate write at 926.150 ms"
            ),
            "{word}"
        );
    }
}

#[test]
fn a_divider_released_before_the_source_is_on_fails_the_lcd_clock_check() {
    let text = moved_after(
        (
            "926150000 write ANACTRL.FRO192M_CTRL 0x40000000",
            "926150000 write ANACTRL.FRO192M_CTRL 0x40000000",
        ),
        "926150000 write SYSCON.FROHFDIV 0x3",
    );
    assert!(lcd_clock_failure(&text).is_some());
}

#[test]
fn fcclksel4_with_no_fc4_clock_gate_write_before_it_fails_the_lcd_clock_check() {
    let text = changed(
        "926150000 write SYSCON.AHBCLKCTRLSET1 0x8000
",
        "",
    );
    assert_eq!(
        lcd_clock_failure(&text).as_deref(),
        Some("SYSCON.FCCLKSEL4 is written at 926.150 ms with no FC4 clock-gate write before it")
    );
}

#[test]
fn a_boot_that_never_selects_the_fc4_clock_passes_the_lcd_clock_check() {
    let text = changed(
        "926150000 write SYSCON.FCCLKSEL4 0x3
",
        "",
    );
    assert_eq!(lcd_clock_failure(&text), None);
    assert_eq!(
        lcd_clock_failure(&with_feeds(BOOT_TO_THE_REFUSED_TRANSPORT)),
        None
    );
}

#[test]
fn the_audio_pll_started_before_the_rails_fails_the_boot_order_check() {
    let text = changed(
        "934426000 write SYSCON.PLL0SSCG1 0x10200000",
        "9450000 write SYSCON.PLL0SSCG1 0x10200000",
    );
    assert!(failed_checks(&text, &refused_transport()).contains(&Check::BootOrderD9));
}

#[test]
fn the_screen_started_before_the_rails_fails_the_boot_order_check() {
    let text = changed(
        "926150000 write FC4+0xff8 0x2",
        "9450000 write FC4+0xff8 0x2",
    );
    assert_eq!(
        failed_checks(&text, &refused_transport()),
        vec![Check::BootOrderD9]
    );
}

#[test]
fn the_core_pll_started_after_the_rails_fails_the_boot_order_check() {
    let text = changed(
        "2100000 write SYSCON.PLL1CTRL 0x201cf0",
        "9600000 write SYSCON.PLL1CTRL 0x201cf0",
    );
    assert!(failed_checks(&text, &refused_transport()).contains(&Check::BootOrderD9));
}

#[test]
fn the_master_clock_or_the_audio_pll_before_the_first_all_off_word_fails_the_first_word_check() {
    for (from, to) in [
        (
            "937426030 write SYSCON.MCLKIO 0x1",
            "9400000 write SYSCON.MCLKIO 0x1",
        ),
        (
            "934426000 write SYSCON.PLL0SSCG1 0x10200000",
            "9400000 write SYSCON.PLL0SSCG1 0x10200000",
        ),
    ] {
        let text = changed(from, to);
        assert!(
            failed_checks(&text, &refused_transport()).contains(&Check::AllOffWordFirst),
            "{to}"
        );
    }
}

#[test]
fn a_rail_stagger_shorter_than_the_spec_fails_the_boot_order_check() {
    let text = changed("51083833 pin P1_26 high", "12000000 pin P1_26 high");
    assert_eq!(
        failed_checks(&text, &refused_transport()),
        vec![Check::BootOrderD9]
    );
}

#[test]
fn a_fault_code_other_than_the_expected_one_fails_the_end_state_check() {
    let text = changed(
        "1053437190 breadcrumb 0xa8b040a",
        "1053437190 breadcrumb 0xa86040a",
    );
    assert_eq!(
        failed_checks(&text, &refused_transport()),
        vec![Check::EndState]
    );
}

const RAISED_11_AT_10_4: &str = "1053437190 breadcrumb 0xa8b040a";

#[test]
fn the_breadcrumb_must_hold_the_scenarios_code_weight_and_place_in_the_end_state_check() {
    let wrong_records = [
        ("another stage", "1053437190 breadcrumb 0xa8b0409"),
        ("another step", "1053437190 breadcrumb 0xa8b050a"),
        (
            "the code noted, not raised",
            "1053437190 breadcrumb 0xa0b040a",
        ),
        ("no code at all", "1053437190 breadcrumb 0xa00040a"),
    ];
    for (what, record) in wrong_records {
        assert!(
            failed_checks(&changed(RAISED_11_AT_10_4, record), &refused_transport())
                .contains(&Check::EndState),
            "{what}"
        );
    }
    let later_latest_place = changed(RAISED_11_AT_10_4, "1053437190 breadcrumb 0x118b040a");
    assert_eq!(
        failed_checks(&later_latest_place, &refused_transport()),
        vec![],
        "the latest place is not part of the record"
    );
}

#[test]
fn a_noted_code_is_no_raised_fault_and_a_raised_one_replaces_it_in_the_end_state_check() {
    let dac = FaultCode::Unanswered(I2cPeer::Dac);
    let noted_then_raised = changed(
        RAISED_11_AT_10_4,
        "1053437185 breadcrumb 0x8030008
1053437190 breadcrumb 0xa8b040a",
    );
    assert_eq!(
        failed_checks(&noted_then_raised, &refused_transport()),
        vec![]
    );
    let expecting_the_note = Expectation {
        record: Some(Recorded::noted(dac, 8, 0)),
        ..refused_transport()
    };
    assert!(failed_checks(&noted_then_raised, &expecting_the_note).contains(&Check::EndState));
    let only_noted = changed(RAISED_11_AT_10_4, "1053437190 breadcrumb 0x8030008");
    assert!(!failed_checks(&only_noted, &expecting_the_note).contains(&Check::EndState));
    assert!(failed_checks(&only_noted, &refused_transport()).contains(&Check::EndState));
    let nothing_expected = Expectation {
        record: None,
        ..refused_transport()
    };
    assert!(failed_checks(&only_noted, &nothing_expected).contains(&Check::EndState));
}

#[test]
fn a_boot_that_does_not_start_its_breadcrumb_fails_the_end_state_check() {
    let magic = "1000000 breadcrumb-magic 0x4d324243\n";
    let fresh = "1000000 breadcrumb 0x1000001\n";
    let broken_starts = [
        ("no magic word", changed(magic, "")),
        (
            "another magic word",
            changed(magic, "1000000 breadcrumb-magic 0x4d324242\n"),
        ),
        ("no fresh record", changed(fresh, "")),
        (
            "a stale record kept",
            changed(fresh, "1000000 breadcrumb 0xa8b040a\n"),
        ),
        (
            "the magic word before the fresh record",
            changed(
                "1000000 breadcrumb 0x1000001\n1000000 breadcrumb-magic 0x4d324243\n",
                "1000000 breadcrumb-magic 0x4d324243\n1000000 breadcrumb 0x1000001\n",
            ),
        ),
        (
            "the magic word stored twice",
            changed(
                magic,
                "1000000 breadcrumb-magic 0x4d324243\n1000001 breadcrumb-magic 0x4d324243\n",
            ),
        ),
    ];
    for (what, text) in broken_starts {
        assert_eq!(
            failed_checks(&text, &refused_transport()),
            vec![Check::EndState],
            "{what}"
        );
    }
}

#[test]
fn a_fault_without_the_all_off_word_after_it_fails_the_end_state_check() {
    let text = changed("1053452190 preamp-word bits=24 value=0x000000\n", "");
    assert!(failed_checks(&text, &refused_transport()).contains(&Check::EndState));
}

#[test]
fn a_reset_where_the_loop_should_run_on_fails_the_end_state_check() {
    let text = changed("1501431670 end time-limit", "1501431670 end reset-request");
    assert!(failed_checks(&text, &refused_transport()).contains(&Check::EndState));
}

#[test]
fn bus_traffic_after_the_boot_settles_fails_the_rest_check() {
    let text = added("1300000000 i2c address 0x39 ack");
    assert!(failed_checks(&text, &refused_transport()).contains(&Check::QuietAtRest));
}

#[test]
fn a_starved_watchdog_fails_the_starvation_check() {
    let text = "\
1999999 mpu enable ctrl=0x1 regions 0x20000000-0x2002ffe1
1999999 write IOCON.P0_28 0x100
1999999 pin P1_26 low
1999999 pin P1_22 low
1999999 pin P1_10 low
1999999 write NVIC+0x80 0xffffffff
1999999 write DMA0.ENABLECLR0 0x50820
1999999 write CTIMER0.TCR 0x0
1999999 write FC7.CFG1 0x0
1999999 write FC6.CFG1 0x0
1999999 write FC2.CFG1 0x0
1999999 write FC0.CFG1 0x0
1999999 write USB1.DEVCMDSTAT 0x200
1999999 watchdog arm tc=25000 mod=0x3
9431750 preamp-word bits=24 value=0x000000
50000000 watchdog feed
150000000 watchdog starved
150000000 end watchdog-reset
";
    let expect_running = Expectation {
        ending: ExpectedEnding::LoopRunsOn,
        record: None,
        quiet_after: None,
        fence: &[],
        planted_store: None,
        after_fault: AfterFault::None,
        silence: Silence::NotRequired,
        analog_words: AnalogWords::Latched,
        rate_change: RateChange::Ordered,
        main_clock: MainClock::MaySwitch,
    };
    let failed = failed_checks(text, &expect_running);
    assert!(failed.contains(&Check::WatchdogNeverStarved));
    assert!(failed.contains(&Check::EndState));
}

#[test]
fn the_fault_routine_must_silence_before_it_stores_the_code_and_parks() {
    let good = "\
1000000 breadcrumb 0x1000001
1000000 breadcrumb-magic 0x4d324243
1999999 write DMA0.ENABLECLR0 0x50820
2000000 write DMA0.ENABLECLR0 0x820
2014999 preamp-word bits=24 value=0x000000
2083333 breadcrumb 0x18a0c01
3000000000 end time-limit
3000000000 stopped-at 0x20001234
";
    let fence_refused = Expectation {
        ending: ExpectedEnding::Parked,
        record: Some(Recorded::raised(FaultCode::FenceUnavailable, 1, 12)),
        quiet_after: None,
        fence: &[],
        planted_store: None,
        after_fault: AfterFault::None,
        silence: Silence::WordAndTransmitDma,
        analog_words: AnalogWords::Latched,
        rate_change: RateChange::Ordered,
        main_clock: MainClock::MaySwitch,
    };
    assert_eq!(failures(good, &fence_refused), vec![]);
    let code_before_the_word = good.replace(
        "2083333 breadcrumb 0x18a0c01",
        "2010000 breadcrumb 0x18a0c01",
    );
    assert_eq!(
        failed_checks(&code_before_the_word, &fence_refused),
        vec![Check::EndState]
    );
    let reset = good.replace(
        "3000000000 end time-limit",
        "2083333 reset-request\n2083333 end reset-request",
    );
    assert_eq!(failed_checks(&reset, &fence_refused), vec![Check::EndState]);
    let busy_after_the_code = good.replace(
        "3000000000 end time-limit",
        "2500000 pin P1_4 low\n3000000000 end time-limit",
    );
    assert_eq!(
        failed_checks(&busy_after_the_code, &fence_refused),
        vec![Check::EndState]
    );
}

#[test]
fn a_parked_image_takes_no_interrupt_after_the_code_and_stops_in_the_park() {
    let parked = "1000000 breadcrumb 0x1000001
1000000 breadcrumb-magic 0x4d324243
1999999 write DMA0.ENABLECLR0 0x50820
2000000 write DMA0.ENABLECLR0 0x820
2014999 preamp-word bits=24 value=0x000000
2083333 breadcrumb 0x10880010
2083333 irq 10 delivered 7 at-breadcrumb
3000000000 end time-limit
3000000000 irq 10 delivered 7
3000000000 stopped-at 0x20001234
";
    let after_running = Expectation {
        ending: ExpectedEnding::Parked,
        record: Some(Recorded::raised(FaultCode::HardFault, 16, 0)),
        quiet_after: None,
        fence: &[],
        planted_store: None,
        after_fault: AfterFault::None,
        silence: Silence::WordAndTransmitDma,
        analog_words: AnalogWords::Latched,
        rate_change: RateChange::Ordered,
        main_clock: MainClock::MaySwitch,
    };
    assert_eq!(failures(parked, &after_running), vec![]);
    let ticks_after_the_code = parked.replace(
        "3000000000 irq 10 delivered 7
",
        "3000000000 irq 10 delivered 9
",
    );
    assert_eq!(
        failed_checks(&ticks_after_the_code, &after_running),
        vec![Check::EndState]
    );
    let first_usb_frame_after_the_code = parked.replace(
        "3000000000 stopped-at",
        "3000000000 irq 47 delivered 1
3000000000 stopped-at",
    );
    assert_eq!(
        failed_checks(&first_usb_frame_after_the_code, &after_running),
        vec![Check::EndState]
    );
    let stopped_in_the_park_kept_out_of_line =
        parked.replace("stopped-at 0x20001234", "stopped-at 0x2000203e");
    assert_eq!(
        failures(&stopped_in_the_park_kept_out_of_line, &after_running),
        vec![]
    );
    let stopped_in_an_interrupt = parked.replace("stopped-at 0x20001234", "stopped-at 0x10000000");
    assert_eq!(
        failures(&stopped_in_an_interrupt, &after_running),
        vec![(
            Check::EndState,
            "the run stopped at 0x10000000, outside board::firmware::fault::enter (0x20001000..0x20001400), board::firmware::fault::park (0x20002000..0x20002040) where the park loop runs".to_string()
        )]
    );
    for just_past_a_park_function in ["0x20001400", "0x20002040"] {
        let stopped_just_past_the_park = parked.replace(
            "stopped-at 0x20001234",
            &format!("stopped-at {just_past_a_park_function}"),
        );
        assert_eq!(
            failed_checks(&stopped_just_past_the_park, &after_running),
            vec![Check::EndState]
        );
    }
    let no_stop_address = parked.replace(
        "3000000000 stopped-at 0x20001234
",
        "",
    );
    assert_eq!(
        failed_checks(&no_stop_address, &after_running),
        vec![Check::EndState]
    );
}

#[test]
fn a_parked_image_whose_screen_started_sends_the_fault_screen_after_the_code_and_nothing_else() {
    let parked = "1000000 breadcrumb 0x1000001
1000000 breadcrumb-magic 0x4d324243
1500000 write DMA0+0x4d0 0x11
1999999 write DMA0.ENABLECLR0 0x50820
2000000 write DMA0.ENABLECLR0 0x820
2014999 preamp-word bits=24 value=0x000000
2083333 breadcrumb 0x10880010
2083333 irq 10 delivered 7 at-breadcrumb
2253333 write DMA0+0x4d8 0x61207
2253333 write DMA0.ENABLESET0 0x2000
2253333 write DMA0+0x70 0x2000
3000000000 end time-limit
3000000000 irq 10 delivered 7
3000000000 stopped-at 0x20001234
";
    let after_running = Expectation {
        ending: ExpectedEnding::Parked,
        record: Some(Recorded::raised(FaultCode::HardFault, 16, 0)),
        quiet_after: None,
        fence: &[],
        planted_store: None,
        after_fault: AfterFault::None,
        silence: Silence::WordAndTransmitDma,
        analog_words: AnalogWords::Latched,
        rate_change: RateChange::Ordered,
        main_clock: MainClock::MaySwitch,
    };
    assert_eq!(failures(parked, &after_running), vec![]);
    for (screen_write, other_write) in [
        (
            "write DMA0.ENABLESET0 0x2000",
            "write DMA0.ENABLESET0 0x2820",
        ),
        ("write DMA0+0x70 0x2000", "write DMA0+0x70 0x820"),
        ("write DMA0+0x4d8 0x61207", "write DMA0+0x458 0x61207"),
        ("write DMA0+0x4d8 0x61207", "write FC4+0xe20 0x61207"),
    ] {
        let widened = parked.replace(screen_write, other_write);
        assert_eq!(
            failed_checks(&widened, &after_running),
            vec![Check::EndState],
            "{other_write} after the code"
        );
    }
    let screen_send = "2253333 write DMA0+0x4d8 0x61207
2253333 write DMA0.ENABLESET0 0x2000
2253333 write DMA0+0x70 0x2000
";
    let no_fault_screen = parked.replace(screen_send, "");
    assert_eq!(
        failures(&no_fault_screen, &after_running),
        vec![(
            Check::EndState,
            "the screen's DMA channel was set up at 1.500 ms, before the fault code at 2.083 ms, and no fault screen send follows the code".to_string()
        )]
    );
    let drawn_before_the_safe_state = no_fault_screen.replace(
        "1999999 write DMA0.ENABLECLR0 0x50820
",
        &format!(
            "1999999 write DMA0.ENABLECLR0 0x50820
{}",
            screen_send.replace("2253333", "1999999")
        ),
    );
    assert_eq!(
        failed_checks(&drawn_before_the_safe_state, &after_running),
        vec![Check::EndState]
    );
    let screen_never_started = no_fault_screen.replace(
        "1500000 write DMA0+0x4d0 0x11
",
        "",
    );
    assert_eq!(failures(&screen_never_started, &after_running), vec![]);
}

#[test]
fn a_planted_store_must_trip_the_fence_and_nothing_else_may() {
    let planted = Expectation {
        ending: ExpectedEnding::LoopRunsOn,
        planted_store: Some(0x4003_4000),
        ..refused_transport()
    };
    let unfenced = added("1200000000 planted-store 0x40034000");
    let quiet_unfenced: String =
        unfenced.replace("1200000000 planted-store", "1100000000 planted-store");
    assert!(failed_checks(&quiet_unfenced, &planted).contains(&Check::FenceNeverTripped));
    let fenced = quiet_unfenced.clone()
        + "1100000000 mpu-fault 0x40034000 write no-region
";
    assert!(!failed_checks(&fenced, &planted).contains(&Check::FenceNeverTripped));
    let stray = fenced
        + "1100000000 mpu-fault 0x20030000 write no-region
";
    assert!(failed_checks(&stray, &planted).contains(&Check::FenceNeverTripped));
}

const LCD_START: &str = "926150000 write FC4+0xff8 0x2\n";

fn after_fault_failure(text: &str, after_fault: AfterFault) -> Option<String> {
    let expectation = Expectation {
        after_fault,
        ..refused_transport()
    };
    failures(text, &expectation)
        .into_iter()
        .find(|(check, _)| *check == Check::PanelAfterFault)
        .map(|(_, reason)| reason)
}

#[test]
fn a_usb_connect_on_a_boot_whose_core_pll_failed_fails_the_after_fault_check() {
    let text = with_feeds(BOOT_TO_THE_REFUSED_TRANSPORT);
    let reason = after_fault_failure(&text, AfterFault::PanelOnly);
    assert!(
        reason
            .as_deref()
            .is_some_and(|reason| reason.contains("USB connected")),
        "{reason:?}"
    );
}

#[test]
fn an_lcd_start_on_a_path_that_never_completed_the_rails_fails_the_after_fault_check() {
    let text = with_feeds(BOOT_TO_THE_REFUSED_TRANSPORT);
    let reason = after_fault_failure(&text, AfterFault::UsbOnly);
    assert!(
        reason
            .as_deref()
            .is_some_and(|reason| reason.contains("panel's power")),
        "{reason:?}"
    );
}

#[test]
fn each_ladder_row_passes_the_after_fault_check_with_what_it_leaves_alive() {
    let text = with_feeds(BOOT_TO_THE_REFUSED_TRANSPORT);
    assert_eq!(after_fault_failure(&text, AfterFault::PanelAndUsb), None);
    assert_eq!(after_fault_failure(&text, AfterFault::AudioPlaying), None);
    let dark = text.replacen(LCD_START, "", 1);
    assert_eq!(after_fault_failure(&dark, AfterFault::UsbOnly), None);
    let detached = text.replacen("3500001 usb connect\n", "", 1);
    assert_eq!(after_fault_failure(&detached, AfterFault::PanelOnly), None);
}

#[test]
fn a_screen_that_never_started_fails_the_after_fault_check_when_the_fault_keeps_it() {
    let text = changed(LCD_START, "");
    for after_fault in [
        AfterFault::PanelOnly,
        AfterFault::PanelAndUsb,
        AfterFault::AudioPlaying,
    ] {
        let reason = after_fault_failure(&text, after_fault);
        assert!(
            reason
                .as_deref()
                .is_some_and(|reason| reason.contains("never started")),
            "{after_fault:?}: {reason:?}"
        );
    }
}

#[test]
fn usb_that_never_attached_fails_the_after_fault_check_when_the_fault_keeps_it() {
    let text = changed("3500001 usb connect\n", "");
    for (after_fault, text) in [
        (AfterFault::UsbOnly, text.replacen(LCD_START, "", 1)),
        (AfterFault::PanelAndUsb, text.clone()),
        (AfterFault::AudioPlaying, text.clone()),
    ] {
        let reason = after_fault_failure(&text, after_fault);
        assert!(
            reason
                .as_deref()
                .is_some_and(|reason| reason.contains("USB never attached")),
            "{after_fault:?}: {reason:?}"
        );
    }
}

#[test]
fn usb_detached_after_the_fault_fails_the_after_fault_check_when_the_fault_keeps_it() {
    let text = added("1200000000 usb disconnect");
    for (after_fault, text) in [
        (AfterFault::UsbOnly, text.replacen(LCD_START, "", 1)),
        (AfterFault::PanelAndUsb, text.clone()),
        (AfterFault::AudioPlaying, text.clone()),
    ] {
        let reason = after_fault_failure(&text, after_fault);
        assert!(
            reason
                .as_deref()
                .is_some_and(|reason| reason.contains("detached")),
            "{after_fault:?}: {reason:?}"
        );
    }
}

#[test]
fn a_converter_transfer_after_the_fault_fails_the_quiet_check() {
    let text = added("1200000000 i2c write 0x49 0x10,0xff ack");
    assert!(
        failed_checks(&text, &refused_transport()).contains(&Check::ConverterBusQuietAfterFault)
    );
    let panel = added("1200000000 i2c write 0x39 0x03,0x8d ack");
    assert!(
        !failed_checks(&panel, &refused_transport()).contains(&Check::ConverterBusQuietAfterFault)
    );
}

#[test]
fn a_converter_transfer_before_the_flexcomm_is_configured_fails_the_converter_bus_check() {
    let text = changed(
        "926108000 write FC3.CFG 0x1",
        "926107000 i2c write 0x39 0x03,0x8d ack\n926108000 write FC3.CFG 0x1",
    );
    assert!(
        failed_checks(&text, &refused_transport())
            .contains(&Check::ConverterBusConfiguredBeforeItsFirstTransfer)
    );
}

#[test]
fn a_converter_transfer_with_no_configuration_at_all_fails_the_converter_bus_check() {
    let text = changed("926108000 write FC3.CFG 0x1\n", "");
    assert!(
        failed_checks(&text, &refused_transport())
            .contains(&Check::ConverterBusConfiguredBeforeItsFirstTransfer)
    );
}

fn without_the_announcement_stops() -> String {
    let boot = with_feeds(BOOT_TO_THE_REFUSED_TRANSPORT);
    let mut text = String::new();
    for line in boot.lines() {
        if line.starts_with("1053437180 ") {
            continue;
        }
        text.push_str(line);
        text.push('\n');
    }
    text
}

#[test]
fn a_fault_code_stored_before_the_announcement_stops_fails_the_silence_check() {
    let text = without_the_announcement_stops();
    let reason = failures(&text, &refused_transport())
        .into_iter()
        .find(|(check, _)| *check == Check::SilencedBeforeTheFault)
        .map(|(_, reason)| reason);
    assert!(
        reason
            .as_deref()
            .is_some_and(|reason| reason.contains("did not happen between")),
        "{reason:?}"
    );
}

const RATE_CHANGE_UP_TO_THE_CLOCK: &str = "\
1000000 handover-write 0x20000000 0x8
1000100 i2c write 0x49 0x12,0xff ack
1000200 i2c write 0x49 0x13,0xff ack
1000300 i2c write 0x49 0x14,0xff ack
1000400 i2c write 0x49 0x15,0xff ack
1000500 write CTIMER0.TCR 0x0
1000600 write DMA0.ENABLECLR0 0x50820
1000700 pin P0_28 low
1000800 write SYSCON.PLL0SSCG1 0x10200000
";

fn never_restarts() -> Expectation {
    Expectation {
        rate_change: RateChange::NeverRestarts,
        main_clock: MainClock::MaySwitch,
        ..refused_transport()
    }
}

#[test]
fn a_transport_that_restarts_after_a_failed_clock_change_fails_the_rate_change_check() {
    let text = RATE_CHANGE_UP_TO_THE_CLOCK.to_string() + "1000900 pin P0_28 high\n";
    let reason = failures(&text, &never_restarts())
        .into_iter()
        .find(|(check, _)| *check == Check::HostRateChangeInOrder)
        .map(|(_, reason)| reason);
    assert!(
        reason
            .as_deref()
            .is_some_and(|reason| reason.contains("restarts an output")),
        "{reason:?}"
    );
    assert!(
        !failed_checks(RATE_CHANGE_UP_TO_THE_CLOCK, &never_restarts())
            .contains(&Check::HostRateChangeInOrder)
    );
}

#[test]
fn a_converter_write_after_a_failed_clock_change_fails_the_rate_change_check() {
    let text = RATE_CHANGE_UP_TO_THE_CLOCK.to_string() + "1000900 i2c write 0x49 0x12,0x40 ack\n";
    let reason = failures(&text, &never_restarts())
        .into_iter()
        .find(|(check, _)| *check == Check::HostRateChangeInOrder)
        .map(|(_, reason)| reason);
    assert!(
        reason
            .as_deref()
            .is_some_and(|reason| reason.contains("restarts an output")),
        "{reason:?}"
    );
}

#[test]
fn a_word_that_never_latches_passes_the_first_word_check_only_when_the_scenario_says_so() {
    let text =
        "1999999 write CTIMER0.TCR 0x0\n1999999 breadcrumb 0xa8b040a\n2000000 end time-limit\n";
    let never = Expectation {
        analog_words: AnalogWords::NeverLatched,
        silence: Silence::NotRequired,
        ..refused_transport()
    };
    assert!(!failed_checks(text, &never).contains(&Check::AllOffWordFirst));
    let rail = "1999999 pin P1_22 high\n".to_string() + text;
    let reason = failures(&rail, &never)
        .into_iter()
        .find(|(check, _)| *check == Check::AllOffWordFirst)
        .map(|(_, reason)| reason);
    assert!(
        reason
            .as_deref()
            .is_some_and(|reason| reason.contains("no all-off word was ever latched")),
        "{reason:?}"
    );
    let latched = Expectation {
        analog_words: AnalogWords::Latched,
        ..never
    };
    let reason = failures(text, &latched)
        .into_iter()
        .find(|(check, _)| *check == Check::AllOffWordFirst)
        .map(|(_, reason)| reason);
    assert!(
        reason
            .as_deref()
            .is_some_and(|reason| reason.contains("no preamp word")),
        "{reason:?}"
    );
}

#[test]
fn a_main_clock_switch_after_a_failed_core_pll_fails_the_stay_put_check() {
    let stays = Expectation {
        main_clock: MainClock::MustStayPut,
        ..refused_transport()
    };
    let text = with_feeds(BOOT_TO_THE_REFUSED_TRANSPORT);
    let reason = failures(&text, &stays)
        .into_iter()
        .find(|(check, _)| *check == Check::MainClockStaysPut)
        .map(|(_, reason)| reason);
    assert!(
        reason
            .as_deref()
            .is_some_and(|reason| reason.contains("main clock switched")),
        "{reason:?}"
    );
    let may = Expectation {
        main_clock: MainClock::MaySwitch,
        ..stays
    };
    assert!(!failed_checks(&text, &may).contains(&Check::MainClockStaysPut));
}

fn device_order_failure(text: &str) -> Option<String> {
    failures(text, &refused_transport())
        .into_iter()
        .find(|(check, _)| *check == Check::UsbDeviceRegistersAfterThePhy)
        .map(|(_, failure)| failure)
}

#[test]
fn the_list_start_after_portmode_and_the_phy_passes_the_device_order_check() {
    assert_eq!(device_order_failure(BOOT_TO_THE_REFUSED_TRANSPORT), None);
}

#[test]
fn a_list_start_before_portmode_fails_the_device_order_check() {
    let text = changed(
        "3500000 write USBHSH+0x50 0x10000
",
        "3500000 write USB1.EPLISTSTART 0x40100000
3500000 write USBHSH+0x50 0x10000
",
    );
    assert!(device_order_failure(&text).is_some());
}

#[test]
fn a_device_register_before_the_last_phy_write_fails_the_device_order_check() {
    let text = changed(
        "3500000 write USBPHY.CTRL_SET 0x180000
",
        "3500000 write USB1+0x1c 0x24
3500000 write USBPHY.CTRL_SET 0x180000
",
    );
    assert!(device_order_failure(&text).is_some());
}

#[test]
fn a_connect_with_no_usb_clock_write_since_the_last_one_fails_the_device_order_check() {
    let text = changed(
        "3500000 write SYSCON.AHBCLKCTRLSET2 0x80000f0
",
        "",
    );
    assert!(
        device_order_failure(&text).is_some_and(|reason| reason.contains("no USB clock write"))
    );
}

#[test]
fn a_clocked_i2s_flexcomm_left_running_through_the_quiesce_fails_the_boot_order_check() {
    let clocked = changed(
        "1999999 write FC7.CFG1 0x0\n",
        "1999990 write SYSCON.AHBCLKCTRLSET1 0x40000\n",
    );
    assert!(failed_checks(&clocked, &refused_transport()).contains(&Check::BootOrderD9));
}

#[test]
fn an_unclocked_i2s_flexcomm_needs_no_quiesce_stop() {
    let unclocked = changed("1999999 write FC7.CFG1 0x0\n", "");
    assert!(!failed_checks(&unclocked, &refused_transport()).contains(&Check::BootOrderD9));
}
