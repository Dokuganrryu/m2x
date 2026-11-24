use std::fs;
use std::path::PathBuf;
use xtask::scan::{InputErrorKind, Violation, scan};

const ROM_API_TREE: u32 = 0x1300_10F0;
const ROM_API_TREE_NON_SECURE_ALIAS: u32 = 0x0300_10F0;
const ROM_FLASH_ERASE_ENTRY_VERSION_0: u32 = 0x1300_413B;
const ROM_FLASH_ERASE_ENTRY_NON_SECURE_ALIAS: u32 = 0x0300_413B;
const SECURITY_ALIAS_BIT: u32 = 1 << 28;

fn crate_file(parts: &[&str]) -> String {
    let path = parts
        .iter()
        .fold(PathBuf::from(env!("CARGO_MANIFEST_DIR")), |path, part| {
            path.join(part)
        });
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn committed_rules() -> String {
    crate_file(&["rules", "m2x-image.rules"])
}

fn fixture(name: &str) -> String {
    crate_file(&["tests", "fixtures", "scan", name])
}

fn committed_rom_entries_with_fixture_names() -> String {
    let rom_entries: String = committed_rules()
        .lines()
        .filter(|line| line.starts_with("rom-flash-api-entry"))
        .map(|line| format!("{line}\n"))
        .collect();
    let fixture_names: String = fixture("image.rules")
        .lines()
        .filter(|line| !line.starts_with("rom-flash-api-entry"))
        .map(|line| format!("{line}\n"))
        .collect();
    rom_entries + &fixture_names
}

fn rom_values_rejected_in(image: &str) -> Vec<u32> {
    scan(
        &committed_rom_entries_with_fixture_names(),
        &fixture(&format!("{image}.nm.txt")),
        &fixture(&format!("{image}.objdump.txt")),
    )
    .unwrap_or_else(|error| panic!("{image} is a readable listing: {error}"))
    .into_iter()
    .filter_map(|violation| match violation {
        Violation::RomFlashApiEntryConstant { value, .. } => Some(value),
        _ => None,
    })
    .collect()
}

#[test]
fn the_committed_rules_reject_the_rom_api_tree_address_in_a_literal_pool() {
    assert!(rom_values_rejected_in("rom-api-tree-literal-pool").contains(&ROM_API_TREE));
}

#[test]
fn the_committed_rules_reject_the_rom_api_tree_address_formed_by_movw_and_movt() {
    assert!(rom_values_rejected_in("rom-api-tree-movw-movt").contains(&ROM_API_TREE));
}

#[test]
fn the_committed_rules_reject_the_non_secure_alias_of_the_rom_api_tree() {
    assert!(
        rom_values_rejected_in("rom-api-tree-non-secure-alias")
            .contains(&ROM_API_TREE_NON_SECURE_ALIAS)
    );
}

#[test]
fn the_committed_rules_reject_the_version_0_rom_flash_erase_entry() {
    assert!(
        rom_values_rejected_in("rom-flash-erase-entry-version-0")
            .contains(&ROM_FLASH_ERASE_ENTRY_VERSION_0)
    );
}

#[test]
fn the_committed_rules_reject_the_non_secure_alias_of_the_rom_flash_erase_entry() {
    assert!(
        rom_values_rejected_in("rom-flash-erase-entry-non-secure-alias")
            .contains(&ROM_FLASH_ERASE_ENTRY_NON_SECURE_ALIAS)
    );
}

#[test]
fn every_committed_rom_entry_is_rejected_under_both_security_aliases() {
    let entries: Vec<u32> = committed_rules()
        .lines()
        .filter_map(|line| line.strip_prefix("rom-flash-api-entry 0x"))
        .map(|hex| u32::from_str_radix(hex, 16).expect("a hex ROM entry"))
        .collect();
    let unpaired: Vec<String> = entries
        .iter()
        .filter(|entry| !entries.contains(&(*entry ^ SECURITY_ALIAS_BIT)))
        .map(|entry| format!("{entry:#010X}"))
        .collect();
    assert_eq!(unpaired, Vec::<String>::new());
}

#[test]
fn the_committed_rules_pass_the_clean_image() {
    assert_eq!(rom_values_rejected_in("clean-speed"), Vec::<u32>::new());
    assert_eq!(rom_values_rejected_in("clean-size"), Vec::<u32>::new());
}

#[test]
fn the_committed_rules_refuse_an_image_without_the_firmware_tick_handler() {
    let error = scan(
        &committed_rules(),
        &fixture("clean-speed.nm.txt"),
        &fixture("clean-speed.objdump.txt"),
    )
    .expect_err("the fixture image has no `audio_tick_interrupt` body");
    assert_eq!(
        error.kind,
        InputErrorKind::NotInListing("audio_tick_interrupt".to_string())
    );
}
