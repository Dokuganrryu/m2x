use std::path::PathBuf;
use std::process::{Command, Output};

fn fixture(name: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("scan")
        .join(name)
        .to_str()
        .expect("fixture path is UTF-8")
        .to_string()
}

fn scan(rules: &str, image: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args([
            "scan",
            &fixture(rules),
            &fixture(&format!("{image}.nm.txt")),
            &fixture(&format!("{image}.objdump.txt")),
        ])
        .output()
        .expect("the xtask binary runs")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("output is UTF-8")
}

#[test]
fn a_clean_image_exits_zero_with_the_scan_limit_stated() {
    let output = scan("image.rules", "clean-speed");
    let report = text(&output.stdout);
    assert_eq!(output.status.code(), Some(0), "{report}");
    assert!(report.starts_with("0 violations. "), "{report}");
}

#[test]
fn a_planted_division_exits_one_and_names_the_rule_function_and_root() {
    let output = scan("image.rules", "udiv-in-tick");
    let report = text(&output.stdout);
    assert_eq!(output.status.code(), Some(1), "{report}");
    assert!(
        report.starts_with(
            "FAIL interrupt-graph: llvm-objdump line 68 in `image::mix_block`, reached from `CTIMER0`: `udiv` divides"
        ),
        "{report}"
    );
}

#[test]
fn rules_missing_the_rom_entry_exit_two_and_say_why() {
    let output = scan("without-rom-flash-api-entry.rules", "clean-speed");
    let errors = text(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{errors}");
    assert!(errors.contains("rom-flash-api-entry"), "{errors}");
    assert!(output.stdout.is_empty());
}

#[test]
fn scan_with_the_wrong_number_of_inputs_prints_the_usage_and_exits_two() {
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["scan", &fixture("image.rules")])
        .output()
        .expect("the xtask binary runs");
    assert_eq!(output.status.code(), Some(2));
    assert!(
        text(&output.stderr)
            .contains("xtask scan <rules> <llvm-nm listing> <llvm-objdump listing>")
    );
}
