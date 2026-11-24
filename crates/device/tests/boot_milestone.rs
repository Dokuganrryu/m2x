mod boot_board;

use boot_board::{Bench, Seen, is_transfer, run_image};
use device::boot::{Image, PllLock, Stage, State};
use device::command::Command;
use device::output_pin::OutputPin;
use device::phantom::{PhantomGrant, PinLevel};

const ENOUGH_PASSES: usize = 5000;

fn is_audio(command: Command) -> bool {
    is_transfer(command)
        || matches!(
            command,
            Command::SwitchPll0Family(_)
                | Command::EnableMasterClockOutput
                | Command::ZeroTransmitBuffers
                | Command::StartDma
                | Command::StartI2s(_)
                | Command::StartTickTimer(_)
                | Command::StartKnobScan
        )
}

fn drives_high_other_than_the_rails(command: Command) -> bool {
    matches!(command, Command::DrivePin(pin, PinLevel::High)
        if !matches!(pin, OutputPin::FirstBringUp | OutputPin::SecondBringUp | OutputPin::ThirdBringUp))
}

fn assert_the_milestone_stops_after_the_screen_start(bench: Bench, what: &str) {
    let full = run_image(Image::Full, bench, ENOUGH_PASSES);
    let milestone = run_image(Image::Milestone, bench, ENOUGH_PASSES);
    let started = full
        .trace
        .iter()
        .position(|seen| *seen == Seen::StartLcd)
        .unwrap_or_else(|| panic!("{what}: the full boot starts the screen"));
    assert_eq!(
        milestone.trace,
        full.trace[..=started],
        "{what}: the milestone boot is the full boot through StartLcd and nothing after"
    );
    assert!(
        full.trace[started..]
            .iter()
            .any(|seen| matches!(seen, Seen::Command(command) if is_transfer(*command))),
        "{what}: the full boot goes on to converter or expander transfers"
    );
    for seen in &milestone.trace {
        if let Seen::Command(command) = *seen {
            assert!(
                !is_audio(command),
                "{what}: the milestone issued {command:?}"
            );
            assert!(
                !drives_high_other_than_the_rails(command),
                "{what}: the milestone drove {command:?}"
            );
        }
    }
    assert_eq!(
        milestone.phantom.grant(),
        PhantomGrant::NONE,
        "{what}: phantom stays off"
    );
    assert!(
        matches!(milestone.boot, State::Scripted(stage, ..) if stage.number() > Stage::StartLcd.number()),
        "{what}: the milestone boot rests past StartLcd, at {:?}",
        milestone.boot
    );
}

#[test]
fn the_milestone_boot_ends_after_start_lcd_with_no_audio_command() {
    assert_the_milestone_stops_after_the_screen_start(Bench::ALL_ACK, "clean bench");
}

#[test]
fn the_milestone_boot_skips_the_panel_stage_when_the_core_pll_never_locks() {
    assert_the_milestone_stops_after_the_screen_start(
        Bench {
            core_pll_lock: PllLock::TimedOut,
            ..Bench::ALL_ACK
        },
        "core PLL never locks",
    );
}
