use std::fs;
use std::path::PathBuf;

fn squeezed(file: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../board/src/firmware")
        .join(file);
    let text =
        fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn the_thread_drains_the_usb_notices_with_whether_the_boot_is_running() {
    let thread = squeezed("thread.rs");
    assert!(
        thread.contains(
            "let running = matches!(self.stage, Stage::Running { .. }); let notices = self.usb_notices.drain(&handovers::USB_NOTICES, running);"
        ),
        "the thread drains the USB notices with something other than whether it is running"
    );
    assert_eq!(
        thread.matches(".drain(").count(),
        1,
        "the thread drains the USB notices in more than one place"
    );
}

#[test]
fn every_console_command_passes_the_running_rule_before_it_is_answered() {
    let thread = squeezed("thread.rs");
    assert!(
        thread.contains("running: matches!(self.stage, Stage::Running { .. }),"),
        "the console is told it is running by something other than the thread's stage"
    );
    let console = squeezed("console.rs");
    assert_eq!(
        console.matches("self.answer(").count(),
        1,
        "the console answers from more than one place"
    );
    assert!(
        console.contains("self.answer(output.admitted(facts.running), facts, reach);"),
        "the console answers a command that has not passed the running rule"
    );
    assert_eq!(
        console.matches("self.execute(").count(),
        1,
        "the console executes a command from somewhere other than its answer"
    );
}
