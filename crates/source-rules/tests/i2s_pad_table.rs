use std::fs;
use std::path::{Path, PathBuf};

const FACTORY_PAD_WORDS: [(&str, &str, u32); 7] = [
    ("IOCON_PIO1_28", "IOCON_PIO1_28_FACTORY_FC7_SCK", 0x101),
    ("IOCON_PIO1_30", "IOCON_PIO1_30_FACTORY_FC7_WS", 0x101),
    ("IOCON_PIO0_20", "IOCON_PIO0_20_FACTORY_FC7_DATA", 0x107),
    ("IOCON_PIO1_12", "IOCON_PIO1_12_FACTORY_FC6_SCK", 0x102),
    ("IOCON_PIO1_13", "IOCON_PIO1_13_FACTORY_FC6_DATA", 0x102),
    ("IOCON_PIO0_26", "IOCON_PIO0_26_FACTORY_FC2_DATA", 0x101),
    ("IOCON_PIO0_24", "IOCON_PIO0_24_FACTORY_FC0_DATA", 0x101),
];

fn board_firmware() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("source-rules sits two folders below the root")
        .join("crates/board/src/firmware")
}

fn source(name: &str) -> String {
    let path = board_firmware().join(name);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn between(source: &str, opening: &str) -> String {
    let rest = source
        .split_once(opening)
        .unwrap_or_else(|| panic!("the source has no {opening}"))
        .1;
    rest.split_once("\n}\n")
        .map_or_else(|| rest.to_string(), |(body, _)| body.to_string())
}

fn array(source: &str, opening: &str) -> String {
    let rest = source
        .split_once(opening)
        .unwrap_or_else(|| panic!("the source has no {opening}"))
        .1;
    let (body, _) = rest
        .split_once("\n];\n")
        .unwrap_or_else(|| panic!("the array after {opening} does not close at column zero"));
    body.to_string()
}

fn pad_table() -> String {
    array(&source("i2s.rs"), "const FACTORY_I2S_PAD_WORDS")
}

fn map_value(name: &str) -> u32 {
    let map = source("map.rs");
    let opening = format!("pub const {name}: u32 = ");
    let line = map
        .lines()
        .find(|line| line.starts_with(&opening))
        .unwrap_or_else(|| panic!("map.rs names no {name}"));
    let digits = line[opening.len()..].trim_end_matches(';').replace('_', "");
    let digits = digits
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("{name} is not written in hexadecimal"));
    u32::from_str_radix(digits, 16).expect("hexadecimal constant")
}

#[test]
fn the_i2s_pad_table_pairs_each_factory_pad_with_its_factory_word_in_the_factory_function_order() {
    let table = pad_table();
    let mut after = 0;
    for (pad, word, _) in FACTORY_PAD_WORDS {
        let entry = format!("({pad}, {word})");
        let found = table[after..]
            .find(&entry)
            .unwrap_or_else(|| panic!("the I2S pad table has no `{entry}` entry"));
        after += found + entry.len();
    }
    assert_eq!(
        table.matches(", IOCON_PIO").count(),
        FACTORY_PAD_WORDS.len(),
        "the I2S pad table names a pad the factory pin function does not mux for I2S"
    );
}

#[test]
fn every_i2s_pad_word_equals_the_word_the_factory_pin_function_stores() {
    for (pad, word, factory) in FACTORY_PAD_WORDS {
        assert_eq!(map_value(word), factory, "{pad} is muxed with {word}");
    }
}

#[test]
fn the_pads_are_muxed_after_the_iocon_clock_and_before_the_flexcomms_select_i2s() {
    let i2s = source("i2s.rs");
    assert!(
        between(&i2s, "fn route_the_shared_clocks_from_fc7(").contains("BusClock::Iocon"),
        "the I2S pad writes need the IOCON bus clock"
    );
    let start = between(&i2s, "pub fn start(");
    let clocks = start
        .find("route_the_shared_clocks_from_fc7()")
        .expect("start enables the Flexcomm clocks first");
    let pads = start
        .find("mux_the_factory_i2s_pads()")
        .expect("start muxes the I2S pads");
    let selected = start
        .find("FLEXCOMM_PSELID_PERSEL")
        .expect("start selects I2S on each Flexcomm");
    assert!(clocks < pads, "the IOCON clock is on before the pads");
    assert!(
        pads < selected,
        "the pads are muxed before any Flexcomm becomes I2S"
    );
}
