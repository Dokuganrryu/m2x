use std::fs;
use std::path::PathBuf;

fn squeezed(source: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../board/src/firmware")
        .join(source);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn every_frame_takes_this_boots_breadcrumb_from_the_word_the_thread_marks() {
    let panel = squeezed("panel.rs");
    assert!(
        panel.contains(
            "dsp: facts.dsp, breadcrumb: fault::breadcrumb(), previous_boot: self.shown.previous_boot, }; let dirty = screen.regions_changed_from(&self.shown); self.shown = screen;"
        ),
        "a frame is compared or kept with a breadcrumb that is not read in that frame"
    );
    assert!(
        panel.contains(
            "pill: self.shown.pill, breadcrumb: fault::breadcrumb(), previous_boot: self.shown.previous_boot, }; screen.pill = screen .pill .next_frame(screen.pill_target(), device::lcd::screen::motion()); let dirty = screen.regions_changed_from(&self.shown); self.shown = screen;"
        ),
        "the sound2-ui frame eases its pill from a screen whose breadcrumb is not read in that frame"
    );
    assert_eq!(
        panel.matches(" breadcrumb: ").count(),
        3,
        "the breadcrumb of a screen is set at boot and in the frame (one frame per screen module), nowhere else"
    );
    assert!(panel.contains("breadcrumb: Breadcrumb::START, previous_boot, } }"));
}

#[test]
fn the_fault_screen_takes_its_stage_from_the_record_when_it_is_sent() {
    assert!(
        squeezed("panel.rs").contains(
            "scheduler::Output::SendFaultCode(code) => { self.link.send_fault(code, fault::breadcrumb().place.stage);"
        ),
        "the fault screen's stage is not the recorded place read at the send"
    );
}

#[test]
fn the_previous_record_reaches_the_screen_from_the_one_read_at_reset() {
    assert!(
        squeezed("thread.rs").contains("Panel::claim(console.previous_boot())"),
        "the panel's previous record is not the console's"
    );
    let console = squeezed("console.rs");
    assert!(console.contains(
        "pub const fn previous_boot(&self) -> Option<Breadcrumb> { self.previous_boot }"
    ));
    assert_eq!(
        squeezed("../firmware.rs").matches("previous_boot").count(),
        2,
        "main reads the previous record once and hands it to the console"
    );
}
