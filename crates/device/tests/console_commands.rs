mod console_support;

use console_support::typed;
use device::console::{ConsoleCommand, Output, Refusal, Report};
use dsp::DspMode;

fn executes(line: &str) -> ConsoleCommand {
    let text = format!("{line}\n");
    match typed(text.as_bytes()).as_slice() {
        [Output::Execute(command)] => *command,
        other => panic!("{line:?} gave {other:?}"),
    }
}

fn refusal_of(line: &str) -> Refusal {
    let text = format!("{line}\n");
    match typed(text.as_bytes()).as_slice() {
        [Output::Reject(refusal)] => *refusal,
        other => panic!("{line:?} gave {other:?}"),
    }
}

#[test]
fn a_status_line_executes_the_status_report() {
    assert_eq!(
        typed(b"status\n"),
        [Output::Execute(ConsoleCommand::Report(Report::Status))]
    );
}

#[test]
fn every_read_report_of_d13_executes_by_its_name() {
    for (line, report) in [
        ("status", Report::Status),
        ("counters", Report::Counters),
        ("dma", Report::Dma),
        ("knob", Report::Knob),
        ("serial", Report::Serial),
        ("version", Report::Version),
        ("cycles", Report::Cycles),
        ("fault", Report::Fault),
        ("panel", Report::Panel),
        ("clocks", Report::Clocks),
        ("i2c", Report::Bus),
        ("power", Report::Power),
    ] {
        assert_eq!(executes(line), ConsoleCommand::Report(report), "{line}");
    }
}

#[test]
fn there_is_no_log_report_because_the_firmware_keeps_no_log() {
    assert_eq!(refusal_of("log"), Refusal::NotAllowlisted);
}

#[test]
fn dsp_bypass_and_dsp_on_switch_the_dsp_mode() {
    assert_eq!(
        executes("dsp bypass"),
        ConsoleCommand::SetDspMode(DspMode::Bypass)
    );
    assert_eq!(executes("dsp on"), ConsoleCommand::SetDspMode(DspMode::On));
}

#[test]
fn meter_reset_clears_the_meters() {
    assert_eq!(executes("meter reset"), ConsoleCommand::ResetMeters);
}

#[test]
fn words_may_be_separated_and_surrounded_by_any_run_of_spaces() {
    assert_eq!(
        executes("   dsp     on  "),
        ConsoleCommand::SetDspMode(DspMode::On)
    );
}

#[test]
fn a_fixed_command_given_extra_words_is_refused_as_malformed() {
    for line in ["status now", "dsp on now", "meter reset all", "fault 3"] {
        assert_eq!(refusal_of(line), Refusal::MalformedArguments, "{line}");
    }
}

#[test]
fn commands_are_lowercase_only() {
    for line in ["STATUS", "Status", "dsp ON", "Meter reset"] {
        assert_eq!(refusal_of(line), Refusal::NotAllowlisted, "{line}");
    }
}

#[test]
fn a_dsp_or_meter_word_that_is_not_a_listed_pair_is_not_allowlisted() {
    // WHY: A bare `meter` is the profile query and `meter <word>` a profile name, so a wrong word there is malformed, not unlisted
    let lines: &[&str] = if cfg!(feature = "sound3") {
        &["dsp", "dsp off", "dsp gain 6"]
    } else {
        &["dsp", "dsp off", "dsp gain 6", "meter", "meter set 0"]
    };
    for &line in lines {
        assert_eq!(refusal_of(line), Refusal::NotAllowlisted, "{line}");
    }
}

#[test]
fn i2cset_and_i2cget_are_not_allowlisted() {
    for line in [
        "i2cset",
        "i2cset 0x48 0 0",
        "i2cset 0x48 62 0xff",
        "i2cget 0x48 113",
        "i2cget",
    ] {
        assert_eq!(refusal_of(line), Refusal::NotAllowlisted, "{line}");
    }
}

#[test]
fn the_factory_adc_command_is_not_allowlisted() {
    for line in ["adc", "adc on", "adc off", "adc 1"] {
        assert_eq!(refusal_of(line), Refusal::NotAllowlisted, "{line}");
    }
}

#[test]
fn no_phantom_command_exists() {
    for line in [
        "phantom",
        "phantom on",
        "phantom 1 on",
        "48v",
        "48v on",
        "p48 1",
        "dsp phantom",
    ] {
        assert_eq!(refusal_of(line), Refusal::NotAllowlisted, "{line}");
    }
}

#[test]
fn no_gain_or_volume_raising_command_exists() {
    for line in [
        "gain",
        "gain 1 60",
        "preamp 1 0x3f",
        "volume",
        "volume 0",
        "volume up",
        "vol +",
        "louder",
        "unmute",
        "attenuation 0",
    ] {
        assert_eq!(refusal_of(line), Refusal::NotAllowlisted, "{line}");
    }
    // WHY: -18 to 0 dB, so no build accepts a trim that raises the gain
    for line in ["trim 0x7fffffff", "trim 0.1", "trim 6"] {
        let typed_line = format!("{line}\n");
        assert!(
            matches!(typed(typed_line.as_bytes()).as_slice(), [Output::Reject(_)]),
            "{line}"
        );
    }
}

#[test]
fn no_reboot_to_bootloader_command_exists() {
    for line in [
        "reboot",
        "reboot bootloader",
        "bootloader",
        "dfu",
        "isp",
        "reset",
        "boot",
    ] {
        assert_eq!(refusal_of(line), Refusal::NotAllowlisted, "{line}");
    }
}

#[test]
fn no_flash_command_exists() {
    for line in [
        "flash",
        "flash write 0x38200",
        "erase",
        "erase 0x00038200",
        "program",
        "save",
        "q",
        "r",
    ] {
        assert_eq!(refusal_of(line), Refusal::NotAllowlisted, "{line}");
    }
}

#[test]
fn no_memory_or_register_write_command_exists() {
    for line in [
        "poke 0x40034000 1",
        "peek 0x20000000",
        "mw 0x20000000 0",
        "md 0x00038200",
        "write 0x40034000 0",
        "mem",
        "reg 0 0",
        "counts",
    ] {
        assert_eq!(refusal_of(line), Refusal::NotAllowlisted, "{line}");
    }
}

#[test]
fn every_refusal_has_a_fixed_reply_line() {
    for refusal in [
        Refusal::NotAllowlisted,
        Refusal::MalformedArguments,
        Refusal::SettingOutOfRange,
        Refusal::LineTooLong,
        Refusal::NotPrintableAscii,
    ] {
        let reply = refusal.reply();
        assert!(reply.starts_with(b"refused: "), "{refusal:?}");
        assert!(reply.ends_with(b"\r\n"), "{refusal:?}");
    }
}
