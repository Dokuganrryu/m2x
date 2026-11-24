#[cfg(feature = "milestone7")]
mod boot_board;

#[cfg(feature = "milestone7")]
mod milestone7 {
    use super::boot_board::{Bench, Caller, Seen, run_image};
    use device::boot::{Image, PllLock, SCREEN_LIT_BEFORE_THE_CLOCKS, State};
    use device::command::Command;
    use device::dac::{Attenuation, DacWrite};
    use device::fault::FaultCode;
    use device::output_pin::OutputPin;
    use device::phantom::PinLevel;
    use device::silent::silenced;
    use device::supervisor;

    const ENOUGH_PASSES: usize = 5000;

    fn position(caller: &Caller, wanted: impl Fn(&Seen) -> bool) -> Option<usize> {
        caller.trace.iter().position(wanted)
    }

    fn issued(caller: &Caller, command: Command) -> Option<usize> {
        position(caller, |seen| *seen == Seen::Command(command))
    }

    fn assert_usb_and_screen_before_the_core_pll(caller: &Caller, what: &str) {
        let attach = issued(caller, Command::AttachUsb).expect("USB attaches");
        let screen = position(caller, |seen| *seen == Seen::StartLcd).expect("the screen starts");
        let core = issued(caller, Command::StartCorePll).expect("PLL1 starts");
        let select = issued(caller, Command::SelectCorePllAsMainClock).expect("PLL1 is selected");
        assert!(
            attach < core && screen < core && core < select,
            "{what}: attach {attach}, screen {screen}, PLL1 start {core}, select {select}"
        );
        let held = caller.trace[screen..core].iter().any(|seen| {
            matches!(seen, Seen::Waited(waited) if waited.0 >= SCREEN_LIT_BEFORE_THE_CLOCKS.0)
        });
        assert!(
            held,
            "{what}: the boot holds for the screen before PLL1, trace {:?}",
            &caller.trace[screen..core]
        );
        let pll0 = position(caller, |seen| {
            matches!(seen, Seen::Command(Command::SwitchPll0Family(_)))
        });
        if let Some(pll0) = pll0 {
            assert!(select < pll0, "{what}: PLL0 after the PLL1 select");
        }
    }

    fn assert_silenced_trace_reaches_nothing(caller: &Caller, what: &str) {
        for seen in &caller.trace {
            let Seen::Command(command) = seen else {
                continue;
            };
            match silenced(*command) {
                Command::DrivePin(
                    OutputPin::TransportActive | OutputPin::PhantomActive,
                    PinLevel::High,
                ) => panic!("{what}: {command:?} drives an output or phantom pin high"),
                Command::WriteDac(DacWrite::Volume(_, level)) => {
                    assert_eq!(level, Attenuation::FULL, "{what}: {command:?}");
                }
                _ => {}
            }
        }
    }

    fn raised(caller: &Caller, code: FaultCode) -> bool {
        caller.reported.iter().any(
            |event| matches!(event, supervisor::Event::FaultRaised(raised, _) if *raised == code),
        )
    }

    #[test]
    fn the_milestone7_boot_reaches_running_with_usb_and_the_screen_before_pll1() {
        let caller = run_image(Image::Full, Bench::ALL_ACK, ENOUGH_PASSES);
        assert!(
            matches!(caller.boot, State::Running(..)),
            "clean bench rests at {:?}",
            caller.boot
        );
        assert_usb_and_screen_before_the_core_pll(&caller, "clean bench");
        assert_silenced_trace_reaches_nothing(&caller, "clean bench");
        for command in [
            Command::WriteExpanderDirection,
            Command::StartKnobScan,
            Command::StartDma,
        ] {
            assert!(issued(&caller, command).is_some(), "{command:?} is issued");
        }
    }

    #[test]
    fn a_pll1_that_never_locks_keeps_usb_and_the_screen_and_still_starts_the_panel() {
        let caller = run_image(
            Image::Full,
            Bench {
                core_pll_lock: PllLock::TimedOut,
                ..Bench::ALL_ACK
            },
            ENOUGH_PASSES,
        );
        assert_usb_and_screen_before_the_core_pll(&caller, "PLL1 never locks");
        assert!(raised(&caller, FaultCode::CorePllUnlocked));
        assert!(
            matches!(caller.boot, State::Degraded(_)),
            "{:?}",
            caller.boot
        );
        assert!(issued(&caller, Command::WriteExpanderDirection).is_some());
        assert!(issued(&caller, Command::StartKnobScan).is_some());
        assert!(issued(&caller, Command::StartDma).is_none());
        assert!(
            position(&caller, |seen| matches!(
                seen,
                Seen::Command(Command::SwitchPll0Family(_))
            ))
            .is_none()
        );
        assert_silenced_trace_reaches_nothing(&caller, "PLL1 never locks");
    }

    #[test]
    fn a_pll0_that_never_locks_keeps_usb_and_the_screen_and_still_starts_the_panel() {
        let caller = run_image(
            Image::Full,
            Bench {
                audio_pll_lock: PllLock::TimedOut,
                ..Bench::ALL_ACK
            },
            ENOUGH_PASSES,
        );
        assert_usb_and_screen_before_the_core_pll(&caller, "PLL0 never locks");
        assert!(raised(&caller, FaultCode::AudioPllUnlocked));
        assert!(
            matches!(caller.boot, State::Degraded(_)),
            "{:?}",
            caller.boot
        );
        assert!(issued(&caller, Command::StartKnobScan).is_some());
        assert!(issued(&caller, Command::StartDma).is_none());
    }
}
