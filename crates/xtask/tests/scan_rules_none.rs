use std::fs;
use std::path::PathBuf;
use xtask::scan::{InputErrorKind, RuleKey, scan};

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("scan")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn rules_with_register_zero_lines(lines: &str) -> String {
    let others: String = fixture("image.rules")
        .lines()
        .filter(|line| !line.starts_with("dac-register-zero-writer"))
        .map(|line| format!("{line}\n"))
        .collect();
    others + lines
}

fn symbols_with(extra: &str) -> String {
    fixture("clean-speed.nm.txt") + extra
}

#[test]
fn register_zero_writer_none_states_that_no_writer_exists_rather_than_naming_one() {
    let violations = scan(
        &rules_with_register_zero_lines("dac-register-zero-writer none\n"),
        &symbols_with("20000400 T image::none\n"),
        &fixture("clean-speed.objdump.txt"),
    )
    .expect("`none` is a complete register-0 rule");
    assert_eq!(violations, Vec::new());
}

#[test]
fn register_zero_writer_none_beside_a_named_writer_is_refused() {
    let error = scan(
        &rules_with_register_zero_lines(
            "dac-register-zero-writer none\ndac-register-zero-writer RegisterZeroWrite::transaction\n",
        ),
        &fixture("clean-speed.nm.txt"),
        &fixture("clean-speed.objdump.txt"),
    )
    .expect_err("`none` and a named writer contradict each other");
    assert_eq!(
        error.kind,
        InputErrorKind::NoneBesideNamed(RuleKey::DacRegisterZeroWriter)
    );
    assert_eq!(error.line, Some(7));
}
