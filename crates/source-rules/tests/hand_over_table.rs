use std::fs;
use std::path::PathBuf;

fn hand_over() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../board/src/firmware/hand_over.rs");
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn map() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../board/src/firmware/map.rs");
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn map_value(name: &str) -> u32 {
    let map = map();
    let opening = format!("pub const {name}: ");
    let line = map
        .lines()
        .find(|line| line.starts_with(&opening))
        .unwrap_or_else(|| panic!("map.rs names no {name}"));
    let written = line[opening.len()..]
        .split_once(" = ")
        .unwrap_or_else(|| panic!("{name} has no value"))
        .1
        .trim_end_matches(';')
        .replace('_', "");
    if let Some(hexadecimal) = written.strip_prefix("0x") {
        return u32::from_str_radix(hexadecimal, 16)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
    }
    let (base, shift) = written
        .split_once(" << ")
        .unwrap_or((written.as_str(), "0"));
    let base = base
        .parse::<u32>()
        .unwrap_or_else(|error| panic!("{name}: {error}"));
    let shift = shift
        .parse::<u32>()
        .unwrap_or_else(|error| panic!("{name}: {error}"));
    base << shift
}

fn clock_gates() -> String {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../board/src/firmware/clock_gates.rs");
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn table() -> String {
    let source = hand_over();
    source
        .split("pub static HAND_OVER_WRITES")
        .nth(1)
        .and_then(|rest| rest.split("END_OF_WRITES").next())
        .expect("the hand-over table is in hand_over.rs")
        .to_string()
}

fn position(table: &str, entry: &str) -> usize {
    table
        .find(entry)
        .unwrap_or_else(|| panic!("the hand-over table has no {entry} entry"))
}

#[test]
fn the_hand_over_table_touches_no_usb_controller_register() {
    let table = table();
    for usb in ["USBHSD", "USBHSH", "USBPHY", "USB1_"] {
        assert!(
            !table.contains(usb),
            "the hand-over runs before the USB controller is powered and clocks itself; it names {usb}"
        );
    }
}

#[test]
fn the_hand_over_table_enables_each_clock_before_its_registers_and_binds_each_pin_to_its_bit() {
    assert_eq!(map_value("GPIO_P1_24"), 1 << 24, "P1_24 is bit 24");
    assert_eq!(map_value("GPIO_P1_25"), 1 << 25, "P1_25 is bit 25");
    assert_eq!(
        map_value("GPIO_CLR_PORT_STEP"),
        4,
        "CLR steps one word a port"
    );
    assert_eq!(
        map_value("GPIO_DIRSET_PORT_STEP"),
        4,
        "DIRSET steps one word a port"
    );
    assert_eq!(
        map_value("GPIO_DIRCLR_PORT_STEP"),
        4,
        "DIRCLR steps one word a port"
    );
    assert_eq!(
        map_value("SYSCON_AHBCLKCTRL0_IOCON"),
        1 << 13,
        "the IOCON clock is bit 13 of AHBCLKCTRL0"
    );
    assert_eq!(
        map_value("SYSCON_AHBCLKCTRL0_GPIO1"),
        1 << 15,
        "the GPIO1 clock is bit 15 of AHBCLKCTRL0"
    );
    let gates = clock_gates();
    for wiring in [
        "BusClock::Iocon => (0, SYSCON_AHBCLKCTRL0_IOCON)",
        "BusClock::Gpio1 => (0, SYSCON_AHBCLKCTRL0_GPIO1)",
    ] {
        assert!(
            gates.contains(wiring),
            "clock_gates.rs does not wire `{wiring}`"
        );
    }
    let table = table();
    let mut after = 0;
    for entry in [
        "clock_on(BusClock::Iocon)",
        "clock_on(BusClock::Gpio1)",
        "write(IOCON_PIO1_24, IOCON_FACTORY_DIGITAL_GPIO)",
        "write(IOCON_PIO1_25, IOCON_FACTORY_DIGITAL_GPIO)",
        "write(GPIO_CLR.bytes_after(GPIO_CLR_PORT_STEP), GPIO_P1_24)",
        "write(GPIO_DIRSET.bytes_after(GPIO_DIRSET_PORT_STEP), GPIO_P1_24)",
        "write(GPIO_DIRCLR.bytes_after(GPIO_DIRCLR_PORT_STEP), GPIO_P1_25)",
    ] {
        let found = table[after..]
            .find(entry)
            .unwrap_or_else(|| panic!("the hand-over table has no `{entry}` entry in that order"));
        after += found + entry.len();
    }
}

#[test]
fn the_hand_over_table_drives_p1_24_low_as_an_output_before_the_panel_can_run() {
    let table = table();
    let pad = position(&table, "IOCON_PIO1_24");
    let low = position(&table, "GPIO_CLR.bytes_after(GPIO_CLR_PORT_STEP)");
    let output = position(&table, "GPIO_DIRSET.bytes_after(GPIO_DIRSET_PORT_STEP)");
    assert!(
        pad < low,
        "the P1_24 pad is muxed before its level is driven"
    );
    assert!(
        low < output,
        "P1_24 is driven low before it becomes an output"
    );
    assert!(
        position(&table, "IOCON_PIO1_25") < position(&table, "GPIO_P1_25"),
        "the P1_25 pad is muxed before its direction is cleared"
    );
}

#[test]
fn the_hand_over_table_drives_the_phantom_pin_low_with_the_pad_word_the_executor_uses() {
    assert_eq!(map_value("GPIO_P1_4"), 1 << 4, "P1_4 is bit 4");
    let pins = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../board/src/firmware/pins.rs"),
    )
    .expect("pins.rs reads");
    for executor in [
        "BoardOutput::PhantomActive => IOCON_PIO1_4,",
        "pad_control(pin).write(IOCON_FACTORY_GPIO_OUTPUT_PAD);",
    ] {
        assert!(
            pins.contains(executor),
            "the executor no longer writes `{executor}`"
        );
    }
    let table = table();
    let mut after = position(&table, "clock_on(BusClock::Gpio1)");
    for entry in [
        "write(IOCON_PIO1_4, IOCON_FACTORY_GPIO_OUTPUT_PAD)",
        "write(GPIO_CLR.bytes_after(GPIO_CLR_PORT_STEP), GPIO_P1_4)",
        "write(GPIO_DIRSET.bytes_after(GPIO_DIRSET_PORT_STEP), GPIO_P1_4)",
    ] {
        let found = table[after..]
            .find(entry)
            .unwrap_or_else(|| panic!("the hand-over table has no `{entry}` entry in that order"));
        after += found + entry.len();
    }
    assert!(
        position(&table, "clock_on(BusClock::Iocon)") < position(&table, "IOCON_PIO1_4"),
        "the IOCON clock is on before the phantom pad word"
    );
}

#[test]
fn the_hand_over_table_drives_both_converter_enables_low_with_the_pad_word_the_executor_uses() {
    assert_eq!(map_value("GPIO_P1_3"), 1 << 3, "P1_3 is bit 3");
    assert_eq!(map_value("GPIO_P1_29"), 1 << 29, "P1_29 is bit 29");
    let pins = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../board/src/firmware/pins.rs"),
    )
    .expect("pins.rs reads");
    for executor in [
        "BoardOutput::ConverterEnable => IOCON_PIO1_3,",
        "BoardOutput::AdcEnable => IOCON_PIO1_29,",
        "pad_control(pin).write(IOCON_FACTORY_GPIO_OUTPUT_PAD);",
    ] {
        assert!(
            pins.contains(executor),
            "the executor no longer writes `{executor}`"
        );
    }
    let table = table();
    for (pad, bit) in [
        ("IOCON_PIO1_3", "GPIO_P1_3"),
        ("IOCON_PIO1_29", "GPIO_P1_29"),
    ] {
        let mut after = position(&table, "clock_on(BusClock::Gpio1)");
        for entry in [
            format!("write({pad}, IOCON_FACTORY_GPIO_OUTPUT_PAD)"),
            format!("write(GPIO_CLR.bytes_after(GPIO_CLR_PORT_STEP), {bit})"),
            format!("write(GPIO_DIRSET.bytes_after(GPIO_DIRSET_PORT_STEP), {bit})"),
        ] {
            let found = table[after..].find(&entry).unwrap_or_else(|| {
                panic!("the hand-over table has no `{entry}` entry in that order")
            });
            after += found + entry.len();
        }
    }
}
