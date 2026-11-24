use std::fs;
use std::path::Path;
use xtask::link_check::{Build, Bytes, Layout, LayoutRefusal, Plant, Verdict, judge};

const LLD_OVERSIZE: &str =
    "  = note: rust-lld: error: board: the image is larger than the 168532-byte image region\n";
const LLD_STACK: &str =
    "  = note: rust-lld: error: board: less than 8 KB of stack remains below 0x20030000\n";
const LLD_STACK_AND_REGION: &str =
    "  = note: rust-lld: error: board: less than 8 KB of stack remains below 0x20030000
          rust-lld: error: section '.bss' will not fit in region 'RAM': overflowed by 2292 bytes
";
const LLD_REGION_ONLY: &str = "  = note: rust-lld: error: section '.bss' will not fit in region 'RAM': overflowed by 4 bytes\n";
const COMPILE_ERROR: &str = "error[E0425]: cannot find value `PLANTED` in this scope\n";

fn clean_board_symbols() -> String {
    let listing = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("link_check")
        .join("board.nm.txt");
    fs::read_to_string(listing).expect("the clean board listing is a fixture")
}

const CLEAN_BOARD: Layout = Layout {
    image: Bytes(48_820),
    stack: Bytes(17_168),
};

#[test]
fn the_layout_of_the_linked_board_counts_the_loaded_image_and_the_free_stack() {
    assert_eq!(
        Layout::from_symbols(&clean_board_symbols()),
        Ok(CLEAN_BOARD)
    );
}

#[test]
fn a_listing_missing_a_layout_symbol_is_refused_by_name() {
    let without_stack_end: String = clean_board_symbols()
        .lines()
        .filter(|line| !line.ends_with(" _stack_end"))
        .map(|line| format!("{line}\n"))
        .collect();
    assert_eq!(
        Layout::from_symbols(&without_stack_end),
        Err(LayoutRefusal::MissingSymbol("_stack_end"))
    );
}

#[test]
fn the_oversize_plant_grows_the_clean_image_to_one_byte_over_the_image_region() {
    assert_eq!(
        Plant::OversizeImage.bytes_to_break(CLEAN_BOARD),
        Bytes(119_713)
    );
}

#[test]
fn the_stack_plant_leaves_less_than_eight_kilobytes_even_when_the_linker_absorbs_eight_bytes_of_padding()
 {
    assert_eq!(
        Plant::StackSqueeze.bytes_to_break(CLEAN_BOARD),
        Bytes(8_985)
    );
}

const LLD_OVERSIZE_PUSHING_THE_PANEL_BUFFERS: &str = "  = note: rust-lld: error: board: the image is larger than the 168532-byte image region
          rust-lld: error: section '.panel_buffers' will not fit in region 'FLASH': overflowed by 21424 bytes
          rust-lld: error: section .data virtual address range overlaps with .panel_buffers
          rust-lld: error: section .panel_buffers virtual address range overlaps with .bss
";
const LLD_PANEL_BUFFERS_ONLY: &str = "  = note: rust-lld: error: section '.panel_buffers' will not fit in region 'FLASH': overflowed by 4 bytes
";

#[test]
fn an_oversize_image_that_also_pushes_the_panel_buffers_out_is_still_caught_by_its_assertion() {
    assert_eq!(
        judge(
            Build::Planted(Plant::OversizeImage),
            false,
            LLD_OVERSIZE_PUSHING_THE_PANEL_BUFFERS
        ),
        Verdict::Holds
    );
}

#[test]
fn panel_buffers_overflowing_without_the_image_assertion_do_not_count_as_the_oversize_plant() {
    assert_ne!(
        judge(
            Build::Planted(Plant::OversizeImage),
            false,
            LLD_PANEL_BUFFERS_ONLY
        ),
        Verdict::Holds
    );
    assert_ne!(
        judge(
            Build::Planted(Plant::StackSqueeze),
            false,
            LLD_OVERSIZE_PUSHING_THE_PANEL_BUFFERS
        ),
        Verdict::Holds
    );
}

#[test]
fn a_clean_build_that_links_holds() {
    assert_eq!(judge(Build::Clean, true, ""), Verdict::Holds);
}

#[test]
fn a_clean_build_that_fails_to_link_breaks_the_check() {
    assert_eq!(
        judge(Build::Clean, false, LLD_STACK),
        Verdict::CleanBuildFailed
    );
}

#[test]
fn a_planted_build_holds_only_when_its_own_assertion_stops_the_link() {
    assert_eq!(
        judge(Build::Planted(Plant::OversizeImage), false, LLD_OVERSIZE),
        Verdict::Holds
    );
    assert_eq!(
        judge(Build::Planted(Plant::StackSqueeze), false, LLD_STACK),
        Verdict::Holds
    );
}

#[test]
fn a_planted_build_that_links_breaks_the_check() {
    assert_eq!(
        judge(Build::Planted(Plant::StackSqueeze), true, ""),
        Verdict::PlantLinked(Plant::StackSqueeze)
    );
}

#[test]
fn a_planted_build_stopped_by_something_other_than_its_assertion_breaks_the_check() {
    assert_eq!(
        judge(Build::Planted(Plant::StackSqueeze), false, LLD_REGION_ONLY),
        Verdict::FailedForAnotherReason(Plant::StackSqueeze)
    );
    assert_eq!(
        judge(Build::Planted(Plant::OversizeImage), false, LLD_STACK),
        Verdict::FailedForAnotherReason(Plant::OversizeImage)
    );
    assert_eq!(
        judge(Build::Planted(Plant::OversizeImage), false, COMPILE_ERROR),
        Verdict::FailedForAnotherReason(Plant::OversizeImage)
    );
}

#[test]
fn a_planted_build_that_also_overflows_a_region_does_not_prove_its_assertion() {
    assert_eq!(
        judge(
            Build::Planted(Plant::StackSqueeze),
            false,
            LLD_STACK_AND_REGION
        ),
        Verdict::FailedForAnotherReason(Plant::StackSqueeze)
    );
}

#[test]
fn every_plant_names_an_assertion_that_the_board_linker_script_carries() {
    let memory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("board")
        .join("memory.x");
    let script = fs::read_to_string(memory).expect("the board crate ships memory.x");
    for plant in Plant::ALL {
        let asserted = format!("\"{}\"", plant.assertion());
        assert!(
            script.contains(&asserted),
            "{asserted} is missing from memory.x"
        );
    }
}
