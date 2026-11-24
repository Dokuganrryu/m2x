#[cfg(feature = "milestone2")]
mod boot_board;

#[cfg(feature = "milestone2")]
mod milestone2 {
    use super::boot_board::{Bench, Caller, Seen, is_transfer, run_image};
    use device::boot::{ClockInput, Image, PllLock, Stage, State};
    use device::command::Command;
    use device::fault::FaultCode;
    use device::output_pin::OutputPin;
    use device::phantom::{PhantomGrant, PinLevel};
    use device::pll::{CoreClockForUsb, MainClockSelectors, RaiseTo96Mhz, core_clock_for_usb};
    use device::supervisor;

    const ENOUGH_PASSES: usize = 5000;

    fn uses_the_core_pll(command: Command) -> bool {
        matches!(
            command,
            Command::StartCorePll | Command::SelectCorePllAsMainClock
        )
    }

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

    fn commands(caller: &Caller) -> Vec<Command> {
        caller
            .trace
            .iter()
            .filter_map(|seen| match seen {
                Seen::Command(command) => Some(*command),
                Seen::Waited(_) | Seen::Log(_) | Seen::StartLcd => None,
            })
            .collect()
    }

    fn assert_the_milestone2_boot(bench: Bench, what: &str) -> Caller {
        let caller = run_image(Image::Milestone, bench, ENOUGH_PASSES);
        let issued = commands(&caller);
        for command in &issued {
            assert!(
                !uses_the_core_pll(*command),
                "{what}: the milestone2 boot issued {command:?}"
            );
            assert!(
                !is_audio(*command),
                "{what}: the milestone2 issued {command:?}"
            );
            assert!(
                !drives_high_other_than_the_rails(*command),
                "{what}: the milestone2 drove {command:?}"
            );
        }
        assert!(
            !issued.contains(&Command::FenceFlashController),
            "{what}: the milestone2 boot raises no MPU fence"
        );
        assert!(
            issued.contains(&Command::StartClockInput),
            "{what}: the milestone2 boot starts the crystal for the USB PHY"
        );
        assert!(
            issued.contains(&Command::AttachUsb),
            "{what}: the milestone2 boot attaches USB, trace {:?}",
            caller.trace
        );
        assert!(
            caller.trace.contains(&Seen::StartLcd),
            "{what}: the milestone2 boot starts the screen"
        );
        assert_eq!(
            caller.phantom.grant(),
            PhantomGrant::NONE,
            "{what}: phantom stays off"
        );
        assert!(
            matches!(caller.boot, State::Scripted(stage, ..) if stage.number() > Stage::StartLcd.number()),
            "{what}: the milestone2 boot rests past StartLcd, at {:?}",
            caller.boot
        );
        caller
    }

    fn raised(caller: &Caller, code: FaultCode) -> bool {
        caller.reported.iter().any(
            |event| matches!(event, supervisor::Event::FaultRaised(raised, _) if *raised == code),
        )
    }

    #[test]
    fn the_milestone2_boot_attaches_usb_without_the_core_pll() {
        assert_the_milestone2_boot(Bench::ALL_ACK, "clean bench");
    }

    #[test]
    fn the_milestone2_boot_attaches_usb_after_a_crystal_timeout_and_records_it() {
        let caller = assert_the_milestone2_boot(
            Bench {
                clock_input: ClockInput::NotReady,
                ..Bench::ALL_ACK
            },
            "crystal never ready",
        );
        assert!(
            raised(&caller, FaultCode::CrystalNotReady),
            "the crystal timeout is raised, reported {:?}",
            caller.reported
        );
    }

    #[test]
    fn the_milestone2_boot_attaches_usb_when_the_core_pll_would_never_lock() {
        assert_the_milestone2_boot(
            Bench {
                core_pll_lock: PllLock::TimedOut,
                ..Bench::ALL_ACK
            },
            "core PLL never locks",
        );
    }

    #[test]
    fn the_milestone2_boot_attaches_usb_and_records_a_usb_phy_lock_timeout() {
        let caller = assert_the_milestone2_boot(
            Bench {
                usb_phy_pll_lock: PllLock::TimedOut,
                ..Bench::ALL_ACK
            },
            "USB PHY PLL never locks",
        );
        assert!(
            caller.trace.iter().any(|seen| matches!(
                seen,
                Seen::Command(Command::RecordFault(FaultCode::UsbPhyPllUnlocked, ..))
            )),
            "the USB PHY lock timeout is recorded"
        );
    }

    const SWITCH_TO_FRO_96: CoreClockForUsb =
        CoreClockForUsb::RaiseTo96Mhz(RaiseTo96Mhz::SwitchToFro96);
    const DIVIDE_FRO_96_BY_ONE: CoreClockForUsb =
        CoreClockForUsb::RaiseTo96Mhz(RaiseTo96Mhz::DivideFro96ByOne);
    const FRO_96_MHZ: MainClockSelectors = MainClockSelectors { a: 3, b: 0 };
    const DIVIDE_BY_ONE: u32 = 0;
    const DIVIDE_BY_TWO: u32 = 1;

    #[test]
    fn only_a_core_handed_over_on_the_12_mhz_fro_is_switched_to_the_96_mhz_fro() {
        for b in 0..8 {
            for a in 0..8 {
                let expected = if (a, b) == (0, 0) {
                    SWITCH_TO_FRO_96
                } else {
                    CoreClockForUsb::KeepAsHandedOver
                };
                assert_eq!(
                    core_clock_for_usb(MainClockSelectors { a, b }, DIVIDE_BY_ONE),
                    expected,
                    "MAINCLKSELA {a}, MAINCLKSELB {b}"
                );
            }
        }
    }

    #[test]
    fn the_reserved_selector_bits_do_not_hide_a_12_mhz_hand_over() {
        assert_eq!(
            core_clock_for_usb(
                MainClockSelectors {
                    a: 0xFFFF_FFF8,
                    b: 0x8000_0000
                },
                DIVIDE_BY_ONE
            ),
            SWITCH_TO_FRO_96
        );
    }

    #[test]
    fn the_measured_hand_over_96_mhz_fro_divided_by_two_is_raised_to_96_mhz() {
        assert_eq!(
            core_clock_for_usb(FRO_96_MHZ, DIVIDE_BY_TWO),
            DIVIDE_FRO_96_BY_ONE
        );
    }

    #[test]
    fn every_divider_that_takes_the_96_mhz_fro_below_60_mhz_is_lifted_and_only_those() {
        for div in 0..=0xFF {
            let expected = if div == DIVIDE_BY_ONE {
                CoreClockForUsb::KeepAsHandedOver
            } else {
                DIVIDE_FRO_96_BY_ONE
            };
            assert_eq!(
                core_clock_for_usb(FRO_96_MHZ, div),
                expected,
                "AHBCLKDIV {div}"
            );
        }
        assert_eq!(
            core_clock_for_usb(FRO_96_MHZ, 0xE000_0000 | DIVIDE_BY_ONE),
            CoreClockForUsb::KeepAsHandedOver,
            "RESET, HALT and REQFLAG are not the divider"
        );
    }

    #[test]
    fn a_divided_12_mhz_fro_is_switched_to_the_96_mhz_fro_as_before() {
        assert_eq!(
            core_clock_for_usb(MainClockSelectors { a: 0, b: 0 }, DIVIDE_BY_TWO),
            SWITCH_TO_FRO_96
        );
    }

    #[test]
    fn a_divided_pll1_or_clkin_core_is_kept_as_handed_over() {
        for (what, handed_over) in [
            ("PLL1", MainClockSelectors { a: 0, b: 2 }),
            ("PLL1 over FRO 96 MHz", MainClockSelectors { a: 3, b: 2 }),
            ("CLKIN", MainClockSelectors { a: 1, b: 0 }),
        ] {
            assert_eq!(
                core_clock_for_usb(handed_over, DIVIDE_BY_TWO),
                CoreClockForUsb::KeepAsHandedOver,
                "{what}"
            );
        }
    }
}

#[cfg(not(feature = "milestone2"))]
#[test]
fn the_milestone2_build_passes_its_boot_tests() {
    use std::env;
    use std::ffi::OsString;
    use std::path::Path;
    use std::process::Command as Process;

    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let target = manifest.join("../../target/milestone2-device-tests");
    let output = Process::new(env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo")))
        .current_dir(manifest)
        .args([
            "test",
            "--package",
            "device",
            "--features",
            "milestone2",
            "--test",
            "boot_milestone2",
        ])
        .env("CARGO_TARGET_DIR", target)
        .output()
        .expect("cargo starts");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("test result: ok. 10 passed"),
        "the milestone2 boot tests fail:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
