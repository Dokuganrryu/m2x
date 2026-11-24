use xtask::hand_over::{
    ADC_ENABLE, CONVERTER_ENABLE, Finding, HELD_LOW, Hazard, HeldPin, PHANTOM, PinAtEntry,
    RegisterNames, extent, judge, pin_at_entry, quiesce_lows, read, starts_something,
};

const QUIESCED: &str = "state primask=1 iser0=0 iser1=0 ispr0=0 ispr1=0 icsr=0 vtor=20000000 syst_csr=0 dma0_ctrl=1 dma0_enabled=0 ctimer0_tcr=0 usb_devcmdstat=80";
const INHERITED_LIVE: &str = "state primask=0 iser0=400 iser1=0 ispr0=400 ispr1=0 icsr=0 vtor=20000000 syst_csr=7 dma0_ctrl=1 dma0_enabled=52820 ctimer0_tcr=1 usb_devcmdstat=10080";

fn findings(trace: &str) -> Vec<Finding> {
    judge(&read(trace).expect("the trace reads")).0
}

#[test]
fn a_data_copy_over_an_inherited_transmit_buffer_while_its_channel_runs_is_reported() {
    let found = findings(&format!(
        "write 4 20000300 4008e280 4 10000000 startup
write 7000 20000160 2002a000 4 0 startup\nreached 20000200 9000\n{QUIESCED}\n"
    ));
    assert!(matches!(
        found.as_slice(),
        [Finding::LiveMemoryWritten { write, range }]
            if write.address == 0x2002_A000 && range.channel.0 == 11
    ));
}

#[test]
fn the_same_write_after_every_channel_is_cleared_is_not_reported() {
    let found = findings(&format!(
        "write 4 20000300 4008e280 4 10000000 startup
write 5 20000300 40082028 4 7fffff startup\nwrite 7000 20000160 2002a000 4 0 startup\nreached 20000200 9000\n{QUIESCED}\n"
    ));
    assert_eq!(found, []);
}

#[test]
fn clearing_only_the_transmit_channels_leaves_the_receive_buffers_live() {
    let found = findings(&format!(
        "write 4 20000300 4008e280 4 10000000 startup
write 5 20000300 40082028 4 820 startup\nwrite 7000 20000160 2002a600 4 0 startup\nreached 20000200 9000\n{QUIESCED}\n"
    ));
    assert!(matches!(
        found.as_slice(),
        [Finding::LiveMemoryWritten { range, .. }] if range.channel.0 == 16
    ));
}

#[test]
fn writes_after_the_entry_is_reached_are_the_images_own() {
    let found = findings(&format!(
        "reached 20000200 9000\n{QUIESCED}\nwrite 9100 20000400 2002a000 4 0 after\n"
    ));
    assert_eq!(found, []);
}

#[test]
fn a_handler_entered_before_the_quiesce_that_returns_with_dma_live_is_reported() {
    let found = findings(&format!(
        "exception 26 20007b94 0\nreturn 26 40\n{INHERITED_LIVE}\nreached 20000200 9000\n{QUIESCED}\n"
    ));
    match found.as_slice() {
        [
            Finding::HandlerLeftHandOverLive {
                number: 26,
                handler: 0x2000_7b94,
                hazards,
            },
        ] => {
            assert!(hazards.contains(&Hazard::InterruptsUnmasked));
            assert!(hazards.contains(&Hazard::DmaChannelsEnabled(0x52820)));
            assert!(hazards.contains(&Hazard::TickTimerCounting));
            assert!(hazards.contains(&Hazard::UsbConnectedWithNoClock));
        }
        other => panic!("unexpected findings {other:?}"),
    }
}

#[test]
fn a_handler_that_returns_with_the_hand_over_quiesced_is_accepted() {
    let found = findings(&format!(
        "exception 15 200089cc 0\nreturn 15 40\n{QUIESCED}\nreached 20000200 9000\n{QUIESCED}\n"
    ));
    assert_eq!(found, []);
}

#[test]
fn a_handler_that_never_returns_is_reported() {
    let found = findings(&format!(
        "exception 15 200001a8 0\nnoreturn 15 200078ac\nreached 20000200 9000\n{QUIESCED}\n"
    ));
    assert!(matches!(
        found.as_slice(),
        [Finding::HandlerDidNotReturn {
            number: 15,
            handler: 0x2000_01a8,
            ..
        }]
    ));
}

#[test]
fn reaching_main_with_the_inherited_state_live_is_reported() {
    let found = findings(&format!("reached 20000200 9000\n{INHERITED_LIVE}\n"));
    assert!(matches!(
        found.as_slice(),
        [Finding::EntryReachedWithHandOverLive { .. }]
    ));
}

#[test]
fn a_usb_controller_left_connected_with_its_device_clock_on_is_accepted_at_the_entry() {
    let found = findings(
        "reached 20000200 9000
state primask=1 iser0=0 iser1=0 ispr0=0 ispr1=0 icsr=0 vtor=20000000 syst_csr=0 dma0_ctrl=1 dma0_enabled=0 ctimer0_tcr=0 usb_devcmdstat=10080 usb1_dev_clock=20
",
    );
    assert_eq!(found, []);
}

#[test]
fn a_usb_controller_left_connected_with_no_device_clock_is_reported_at_the_entry() {
    let found = findings(
        "reached 20000200 9000
state primask=1 iser0=0 iser1=0 ispr0=0 ispr1=0 icsr=0 vtor=20000000 syst_csr=0 dma0_ctrl=1 dma0_enabled=0 ctimer0_tcr=0 usb_devcmdstat=10080
",
    );
    assert!(matches!(
        found.as_slice(),
        [Finding::EntryReachedWithHandOverLive { hazards }]
            if hazards.as_slice() == [Hazard::UsbConnectedWithNoClock]
    ));
}

#[test]
fn a_startup_that_stops_is_reported() {
    let found = findings("stopped 20000300 12 the instruction budget ran out\n");
    assert!(matches!(
        found.as_slice(),
        [Finding::StartupStopped {
            pc: 0x2000_0300,
            ..
        }]
    ));
}

#[test]
fn an_unreadable_emulator_line_is_refused_with_its_line_number() {
    let error = read("reached 20000200 9000\nnonsense here\n").expect_err("garbage refuses");
    assert_eq!(error.line, 2);
}

fn names() -> RegisterNames {
    let rows = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/lpc55s28-svd-registers.txt"),
    )
    .expect("the SVD rows are committed");
    RegisterNames::from_rows(&rows)
}

#[test]
fn a_gpio_word_pin_is_named_by_port_and_pin_from_the_svd_rows() {
    assert_eq!(names().name(0x4008_D0D8), "GPIO W[1].W_[22]");
    assert_eq!(names().name(0x4008_2028), "DMA0 ENABLECLR0");
    assert_eq!(names().name(0x4000_0224), "SYSCON AHBCLKCTRLSET[1]");
    assert_eq!(names().name(0xE000_ED08), "SCB VTOR");
}

#[test]
fn driving_a_rail_pin_high_starts_something_and_driving_a_pin_low_does_not() {
    let names = names();
    assert!(starts_something(names.name(0x4008_D0D8), 1));
    assert!(!starts_something(names.name(0x4008_D090), 0));
}

#[test]
fn a_pll_write_starts_a_clock_and_a_watchdog_feed_does_not() {
    let names = names();
    assert!(starts_something(names.name(0x4000_0560), 0));
    assert!(!starts_something(names.name(0x4000_C008), 0xAA));
}

#[test]
fn a_usb_write_that_keeps_the_device_disconnected_starts_nothing() {
    let names = names();
    assert!(!starts_something(names.name(0x4009_4000), 0x280));
    assert!(starts_something(names.name(0x4009_4000), 0x1_0080));
}

#[test]
fn a_write_to_the_analog_word_bus_starts_something_past_the_quiesce() {
    let names = names();
    assert!(starts_something(names.name(0x4008_7FF8), 2));
    assert!(starts_something(names.name(0x4008_7400), 0x35));
    assert!(starts_something(names.name(0x4008_7E20), 0));
}

#[test]
fn a_ram_write_while_the_inherited_output_connect_pin_is_still_high_is_reported() {
    let found = findings(&format!(
        "write 5 20000300 40082028 4 7fffff startup
write 7000 20000160 20001000 4 0 startup
reached 20000200 9000
{QUIESCED}
"
    ));
    assert!(matches!(
        found.as_slice(),
        [Finding::RamWrittenWithOutputsConnected { write }] if write.address == 0x2000_1000
    ));
}

#[test]
fn a_ram_write_after_the_output_connect_pin_is_cleared_is_accepted() {
    let found = findings(&format!(
        "write 4 20000300 4008e280 4 10000000 startup
write 5 20000300 40082028 4 7fffff startup
write 7000 20000160 20001000 4 0 startup
reached 20000200 9000
{QUIESCED}
"
    ));
    assert_eq!(found, []);
}

#[test]
fn the_cores_own_exception_frame_on_the_inherited_stack_is_not_an_image_ram_write() {
    let found = findings(&format!(
        "write 0 20000130 2002fe60 4 0 frame15
write 4 20000300 4008e280 4 10000000 exception15
write 5 20000300 40082028 4 7fffff exception15
reached 20000200 9000
{QUIESCED}
"
    ));
    assert_eq!(found, []);
}

const TABLE: std::ops::Range<u32> = 0x2000_01a8..0x2000_01cc;

fn phantom(trace: &str) -> PinAtEntry {
    held(trace, PHANTOM)
}

fn held(trace: &str, pin: HeldPin) -> PinAtEntry {
    pin_at_entry(&read(trace).expect("the trace reads"), &TABLE, pin)
}

#[test]
fn a_table_that_selects_the_pad_clears_p1_4_and_then_makes_it_an_output_drives_phantom_low() {
    assert_eq!(
        phantom(
            "write 9 200001b8 40001090 4 100 startup
write 10 200001b8 4008e284 4 10 startup
write 11 200001b8 4008e384 4 10 startup
reached 20000200 9000
"
        ),
        PinAtEntry::DrivenLow
    );
}

#[test]
fn a_table_with_no_phantom_entry_leaves_the_pin_as_the_previous_image_left_it() {
    assert_eq!(
        phantom("write 4 200001b8 4008e280 4 10000000 startup\nreached 20000200 9000\n"),
        PinAtEntry::Inherited
    );
}

#[test]
fn p1_4_made_an_output_before_its_level_is_cleared_is_refused_even_when_the_table_runs_twice() {
    let wrong_order = "write 9 200001b8 40001090 4 100 exception15
write 10 200001b8 4008e384 4 10 exception15
write 11 200001b8 4008e284 4 10 exception15
";
    let again = wrong_order.replace("exception15", "startup");
    for trace in [
        format!(
            "{wrong_order}reached 20000200 9000
"
        ),
        format!(
            "{wrong_order}{again}reached 20000200 9000
"
        ),
    ] {
        assert!(matches!(
            phantom(&trace),
            PinAtEntry::Disturbed(write) if write.index == 10
        ));
    }
}

#[test]
fn the_table_run_a_second_time_keeps_the_phantom_pin_driven_low() {
    let once = "write 9 200001b8 40001090 4 100 exception15
write 10 200001b8 4008e284 4 10 exception15
write 11 200001b8 4008e384 4 10 exception15
";
    let again = once.replace("exception15", "startup");
    assert_eq!(
        phantom(&format!(
            "{once}{again}reached 20000200 9000
"
        )),
        PinAtEntry::DrivenLow
    );
}

#[test]
fn the_same_three_writes_made_after_the_hand_over_table_do_not_count() {
    assert!(matches!(
        phantom(
            "write 900 20003000 40001090 4 100 startup
write 901 20003000 4008e284 4 10 startup
write 902 20003000 4008e384 4 10 startup
reached 20000200 9000
"
        ),
        PinAtEntry::Disturbed(write) if write.index == 902
    ));
    assert_eq!(
        phantom(
            "reached 20000200 9000
write 9001 200001b8 40001090 4 100 after
write 9002 200001b8 4008e284 4 10 after
write 9003 200001b8 4008e384 4 10 after
"
        ),
        PinAtEntry::Inherited
    );
}

#[test]
fn a_pad_word_other_than_the_gpio_output_word_is_refused() {
    assert!(matches!(
        phantom(
            "write 9 200001b8 40001090 4 300 startup
write 10 200001b8 4008e284 4 10 startup
write 11 200001b8 4008e384 4 10 startup
reached 20000200 9000
"
        ),
        PinAtEntry::Disturbed(write) if write.value == 0x300
    ));
}

#[test]
fn a_write_that_raises_p1_4_before_main_undoes_the_table() {
    for raise in [
        "4008e204 4 10",
        "4008d090 4 1",
        "4008c024 1 1",
        "4008e304 4 10",
        "4008e404 4 10",
    ] {
        assert!(
            matches!(
                phantom(&format!(
                    "write 9 200001b8 40001090 4 100 startup
write 10 200001b8 4008e284 4 10 startup
write 11 200001b8 4008e384 4 10 startup
write 500 20003000 {raise} startup
reached 20000200 9000
"
                )),
                PinAtEntry::Disturbed(write) if write.index == 500
            ),
            "{raise}"
        );
    }
}

struct EnableRows {
    pin: HeldPin,
    pad: &'static str,
    bit: &'static str,
    byte: &'static str,
    word: &'static str,
}

const ENABLES: [EnableRows; 2] = [
    EnableRows {
        pin: CONVERTER_ENABLE,
        pad: "4000108c",
        bit: "8",
        byte: "4008c023",
        word: "4008d08c",
    },
    EnableRows {
        pin: ADC_ENABLE,
        pad: "400010f4",
        bit: "20000000",
        byte: "4008c03d",
        word: "4008d0f4",
    },
];

fn three_writes(rows: &EnableRows, pad_word: &str, first: &str, second: &str) -> String {
    format!(
        "write 9 200001b8 {} 4 {pad_word} startup
write 10 200001b8 {first} 4 {} startup
write 11 200001b8 {second} 4 {} startup
",
        rows.pad, rows.bit, rows.bit
    )
}

#[test]
fn the_hand_over_holds_the_phantom_pin_and_both_converter_enables_low() {
    let names: Vec<&str> = HELD_LOW.iter().map(|pin| pin.name).collect();
    assert_eq!(names, ["P1_4", "P1_3", "P1_29"]);
}

#[test]
fn a_table_that_selects_the_pad_clears_an_enable_and_then_makes_it_an_output_drives_it_low() {
    for rows in ENABLES {
        let trace = format!(
            "{}reached 20000200 9000
",
            three_writes(&rows, "100", "4008e284", "4008e384")
        );
        assert_eq!(
            held(&trace, rows.pin),
            PinAtEntry::DrivenLow,
            "{}",
            rows.pin.name
        );
        assert_eq!(
            held(&trace, PHANTOM),
            PinAtEntry::Inherited,
            "{}",
            rows.pin.name
        );
        assert_eq!(
            PinAtEntry::DrivenLow.describe(rows.pin),
            format!(
                "{}, {}, is driven low by the hand-over table: pad word 0x100, clear, direction",
                rows.pin.name, rows.pin.role
            )
        );
    }
}

#[test]
fn an_enable_with_no_table_entry_is_reported_as_left_as_the_previous_image_left_it() {
    for rows in ENABLES {
        let state = held(
            "reached 20000200 9000
",
            rows.pin,
        );
        assert_eq!(state, PinAtEntry::Inherited);
        assert!(
            state.describe(rows.pin).contains(&format!(
                "writes no pad word 0x100 to IOCON PIO{}",
                &rows.pin.name[1..]
            )),
            "{}",
            state.describe(rows.pin)
        );
    }
}

#[test]
fn an_enable_made_an_output_before_its_level_is_cleared_is_refused() {
    for rows in ENABLES {
        let trace = format!(
            "{}reached 20000200 9000
",
            three_writes(&rows, "100", "4008e384", "4008e284")
        );
        assert!(
            matches!(held(&trace, rows.pin), PinAtEntry::Disturbed(write) if write.index == 10),
            "{}",
            rows.pin.name
        );
    }
}

#[test]
fn an_enable_pad_word_other_than_the_gpio_output_word_is_refused() {
    for rows in ENABLES {
        let trace = format!(
            "{}reached 20000200 9000
",
            three_writes(&rows, "300", "4008e284", "4008e384")
        );
        assert!(
            matches!(held(&trace, rows.pin), PinAtEntry::Disturbed(write) if write.value == 0x300),
            "{}",
            rows.pin.name
        );
    }
}

#[test]
fn a_write_that_raises_an_enable_before_main_undoes_the_table() {
    for rows in ENABLES {
        for raise in [
            format!("4008e204 4 {}", rows.bit),
            format!("{} 4 1", rows.word),
            format!("{} 1 1", rows.byte),
            format!("4008e304 4 {}", rows.bit),
            format!("4008e404 4 {}", rows.bit),
        ] {
            let trace = format!(
                "{}write 500 20003000 {raise} startup
reached 20000200 9000
",
                three_writes(&rows, "100", "4008e284", "4008e384")
            );
            assert!(
                matches!(held(&trace, rows.pin), PinAtEntry::Disturbed(write) if write.index == 500),
                "{}: {raise}",
                rows.pin.name
            );
        }
    }
}

#[test]
fn the_hand_over_routine_ends_where_the_next_symbol_starts() {
    let symbols = "20000130 T Reset\n200001a9 T __pre_init\n200001a9 T SysTick\n200001cc t $d.1\n2000b734 T main\n";
    assert_eq!(
        extent(symbols, "__pre_init"),
        Some(0x2000_01a8..0x2000_01cc)
    );
    assert_eq!(extent(symbols, "main"), None);
}

const P0_28_LOW: &str = "write 9100 20000400 4008d070 4 0 after
write 9101 20000400 4008e380 4 10000000 after
";
const P1_26_LOW: &str = "write 9102 20000400 4008d0e8 4 0 after
write 9103 20000400 4008e384 4 4000000 after
";
const P1_22_LOW: &str = "write 9104 20000400 4008d0d8 4 0 after
write 9105 20000400 4008e384 4 400000 after
";
const P1_10_LOW: &str = "write 9106 20000400 4008d0a8 4 0 after
write 9107 20000400 4008e384 4 400 after
";

fn lows(after_main: &[&str]) -> Result<(), String> {
    let trace = format!(
        "reached 20000200 9000
{QUIESCED}
{}",
        after_main.concat()
    );
    quiesce_lows(&read(&trace).expect("the trace reads"))
}

#[test]
fn p0_28_then_the_three_rail_pins_driven_low_in_the_shutdown_order_is_the_quiesce_pin_order() {
    assert_eq!(lows(&[P0_28_LOW, P1_26_LOW, P1_22_LOW, P1_10_LOW]), Ok(()));
}

#[test]
fn a_quiesce_that_drives_only_p0_28_low_is_reported() {
    assert_eq!(
        lows(&[P0_28_LOW]),
        Err("past `main` the image drives P0_28 low and then stops driving pins, and the Quiesce script drives P0_28, P1_26, P1_22 and P1_10 low in that order".to_string())
    );
}

#[test]
fn the_rail_pins_driven_low_in_the_order_the_rails_raise_them_is_reported() {
    assert_eq!(
        lows(&[P0_28_LOW, P1_22_LOW, P1_26_LOW, P1_10_LOW]),
        Err("past `main` pin drive 2 is P1_22 low at step 9105, and the Quiesce script drives P0_28, P1_26, P1_22 and P1_10 low in that order".to_string())
    );
}

#[test]
fn the_rail_pins_driven_low_before_p0_28_is_reported() {
    assert_eq!(
        lows(&[P1_26_LOW, P1_22_LOW, P1_10_LOW, P0_28_LOW]),
        Err("past `main` pin drive 1 is P1_26 low at step 9103, and the Quiesce script drives P0_28, P1_26, P1_22 and P1_10 low in that order".to_string())
    );
}

#[test]
fn a_rail_pin_driven_high_in_the_quiesce_is_reported() {
    let high = "write 9102 20000400 4008d0e8 4 1 after
write 9103 20000400 4008e384 4 4000000 after
";
    assert_eq!(
        lows(&[P0_28_LOW, high, P1_22_LOW, P1_10_LOW]),
        Err("past `main` pin drive 2 is P1_26 high at step 9103, and the Quiesce script drives P0_28, P1_26, P1_22 and P1_10 low in that order".to_string())
    );
}

#[test]
fn a_level_word_with_no_direction_write_drives_nothing() {
    let level_only = "write 9102 20000400 4008d0e8 4 0 after
";
    assert_eq!(
        lows(&[P0_28_LOW, level_only, P1_22_LOW, P1_10_LOW]),
        Err("past `main` pin drive 2 is P1_22 low at step 9105, and the Quiesce script drives P0_28, P1_26, P1_22 and P1_10 low in that order".to_string())
    );
}

#[test]
fn pin_writes_before_main_are_not_the_quiesce_script() {
    let trace = format!(
        "write 4 20000300 4008d070 4 0 startup
write 5 20000300 4008e380 4 10000000 startup
reached 20000200 9000
{QUIESCED}
{P1_26_LOW}{P1_22_LOW}{P1_10_LOW}"
    );
    assert!(quiesce_lows(&read(&trace).expect("the trace reads")).is_err());
}
