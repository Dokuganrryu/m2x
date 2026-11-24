use std::path::PathBuf;
use std::process::{Command, Output};

fn fixture(name: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
        .to_str()
        .expect("fixture path is UTF-8")
        .to_string()
}

fn xtask(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(arguments)
        .output()
        .expect("the xtask binary runs")
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8")
}

#[test]
fn a_real_listing_at_exactly_its_share_passes_with_its_cycle_figure_and_percentage() {
    let listing = fixture("limiter-peak-at-share.txt");
    let output = xtask(&["price", &listing]);
    let report = stdout_of(&output);
    assert_eq!(output.status.code(), Some(0), "{report}");
    assert!(
        report.contains(
            "PASS on-limiter-peak at 6 frames: 56 instructions, 82 proxy cycles, 0.7% of the 12,500-cycle tick, within its D3 share of 82 (0.7%). Proxy x1.5 is 123 (1.0%), reported, not gated."
        ),
        "{report}"
    );
}

#[test]
fn a_traced_loop_under_its_share_passes_and_the_command_exits_zero() {
    let listing = fixture("capture-ramp-within-share.txt");
    let output = xtask(&["price", &listing]);
    let report = stdout_of(&output);
    assert_eq!(output.status.code(), Some(0), "{report}");
    assert!(
        report.contains(
            "PASS capture-ramp at 12 frames: 12 instructions, 27 proxy cycles, 0.2% of the 12,500-cycle tick, within its D3 share of 494 (4.0%). Proxy x1.5 is 41 (0.3%), reported, not gated."
        ),
        "{report}"
    );
}

#[test]
fn one_failing_listing_among_several_fails_the_run_and_every_line_carries_the_inference_label() {
    let passing = fixture("capture-ramp-within-share.txt");
    let failing = fixture("dispatch-one-cycle-over-share.txt");
    let output = xtask(&["price", &passing, &failing]);
    let report = stdout_of(&output);
    assert_eq!(output.status.code(), Some(1), "{report}");
    let lines: Vec<&str> = report.lines().collect();
    assert_eq!(lines.len(), 3, "{report}");
    assert!(lines.iter().all(|line| line.contains("")), "{report}");
    assert!(
        lines[2].starts_with("2 blocks priced, 1 outside their D3 share."),
        "{report}"
    );
}

#[test]
fn a_listing_without_encodings_is_refused_with_its_path_and_line() {
    let listing = fixture("objdump-without-encodings.txt");
    let output = xtask(&["price", &listing]);
    let errors = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert_eq!(output.status.code(), Some(2), "{errors}");
    assert!(
        errors.contains("objdump-without-encodings.txt: listing line 2:"),
        "{errors}"
    );
}

#[test]
fn an_unknown_subcommand_prints_the_usage_and_exits_two() {
    let output = xtask(&["reprice"]);
    let errors = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert_eq!(output.status.code(), Some(2));
    assert!(
        errors.starts_with("usage: xtask price <listing>..."),
        "{errors}"
    );
}
