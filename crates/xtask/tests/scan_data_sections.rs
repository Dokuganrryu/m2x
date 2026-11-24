use std::fs;
use std::path::PathBuf;
use xtask::scan::{Input, InputErrorKind, Rule, Violation, scan_data_sections};

const RULES: &str = "interrupt-root audio_tick_interrupt
factory-accessor none
rom-flash-api-entry 0x130010F0
dac-register-zero-writer none
dead-stripped NominalPacket::at
";

const FIRST_RODATA_LINE: &str = " 2000a250 04000000 00000000 00000000 04000000";
const SECOND_DATA_LINE: &str = " 20029410 00000000 00000000 00000000 00000000";

fn clean_sections() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("scan-image")
        .join("clean.sections.txt");
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn with_line(original: &str, replacement: &str) -> String {
    let clean = clean_sections();
    assert!(clean.contains(original), "the fixture holds `{original}`");
    clean.replacen(original, replacement, 1)
}

fn violations_in(dump: &str) -> Vec<Violation> {
    scan_data_sections(RULES, dump).unwrap_or_else(|error| panic!("a readable dump: {error}"))
}

#[test]
fn the_data_sections_of_the_clean_image_hold_no_guarded_word() {
    assert_eq!(violations_in(&clean_sections()), Vec::new());
}

#[test]
fn a_flash_controller_address_kept_in_a_rodata_table_fails_and_names_the_section() {
    let dump = with_line(
        FIRST_RODATA_LINE,
        " 2000a250 04000000 00400340 00000000 04000000",
    );
    assert_eq!(
        violations_in(&dump),
        vec![Violation::FlashControllerConstant {
            line: 28,
            function: "section .rodata".to_string(),
            value: 0x4003_4000,
        }]
    );
}

#[test]
fn a_rom_flash_api_entry_kept_in_initialised_data_fails() {
    let dump = with_line(
        SECOND_DATA_LINE,
        " 20029410 00000000 00000000 f0100013 00000000",
    );
    let rules: Vec<Rule> = violations_in(&dump).iter().map(Violation::rule).collect();
    assert_eq!(rules, vec![Rule::RomFlashApiEntryConstant]);
}

#[test]
fn the_factory_magic_in_a_data_table_fails_even_though_it_is_a_read_address() {
    let dump = with_line(
        SECOND_DATA_LINE,
        " 20029410 00820300 00000000 00000000 00000000",
    );
    let rules: Vec<Rule> = violations_in(&dump).iter().map(Violation::rule).collect();
    assert_eq!(rules, vec![Rule::FactoryAddressOutsideAccessor]);
}

#[test]
fn a_guarded_value_that_straddles_word_alignment_is_not_read_as_an_address() {
    let dump = with_line(
        FIRST_RODATA_LINE,
        " 2000a250 04004000 03400000 00000000 04000000",
    );
    assert_eq!(violations_in(&dump), Vec::new());
}

#[test]
fn the_text_section_and_the_unloaded_metadata_are_not_read_as_data() {
    let dump = with_line(
        " 20000130 134880f3 08881348 13490160 134880f3",
        " 20000130 00400340 08881348 13490160 134880f3",
    );
    assert_eq!(violations_in(&dump), Vec::new());
}

#[test]
fn the_uninitialised_panel_buffers_after_the_image_are_not_read_as_data() {
    let dump = format!(
        "{}Contents of section .panel_buffers:
 2001c300 00400340 00400340 00400340 00400340  .@.@.@.@.@.@.@.@
",
        clean_sections()
    );
    assert_eq!(violations_in(&dump), Vec::new());
}

#[test]
fn a_section_the_scan_has_no_policy_for_is_refused() {
    let dump = with_line(
        "Contents of section .data:",
        "Contents of section .ramfunc:",
    );
    let error = scan_data_sections(RULES, &dump).expect_err("an unknown section is refused");
    assert_eq!(error.input, Input::DataSections);
    assert_eq!(
        error.kind,
        InputErrorKind::UnrecognisedDataSection(".ramfunc".to_string())
    );
}

#[test]
fn a_dump_line_that_does_not_continue_its_section_is_refused() {
    let dump = with_line(
        SECOND_DATA_LINE,
        " 20029420 00000000 00000000 00000000 00000000",
    );
    let error = scan_data_sections(RULES, &dump).expect_err("a gap is refused");
    assert_eq!((error.input, error.line), (Input::DataSections, Some(143)));
    assert_eq!(error.kind, InputErrorKind::UnreadableLine);
}

#[test]
fn a_symbol_listing_given_as_the_section_dump_is_refused() {
    let error = scan_data_sections(RULES, "20000130 T Reset\n").expect_err("not a dump");
    assert_eq!(error.kind, InputErrorKind::NotAnArmListing);
}

#[test]
fn a_dump_with_no_loaded_data_section_is_refused() {
    let dump = "board:\tfile format elf32-littlearm\nContents of section .comment:\n 0000 4c696e6b  Link\n";
    let error = scan_data_sections(RULES, dump).expect_err("nothing to scan");
    assert_eq!(error.kind, InputErrorKind::Empty);
}
